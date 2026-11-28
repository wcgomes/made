use made_remote::*;

#[test]
fn terminal_output_done_is_required_and_empty_false_is_malformed() {
    assert_eq!(PROTOCOL_MAJOR, 1);
    assert!(
        serde_json::from_str::<Operation>(r#"{"op":"TerminalOutput","data":{"data":""}}"#).is_err()
    );
    assert!(serde_json::from_str::<Operation>(r#"{"op":"Snapshot","data":{"bytes":""}}"#).is_err());
    let empty_false = frame(Operation::TerminalOutput {
        data: Chunk::new(Vec::new()).unwrap(),
        done: false,
    });
    assert_eq!(empty_false.validate(), Err(ProtocolError::Malformed));
    assert!(encode(&empty_false).is_err());
    let empty_true = frame(Operation::TerminalOutput {
        data: Chunk::new(Vec::new()).unwrap(),
        done: true,
    });
    empty_true.validate().unwrap();
    let encoded = encode(&empty_true).unwrap();
    let mut decoder = FrameDecoder::new();
    let (_, parsed) = decoder.push(&encoded).unwrap();
    assert_eq!(
        parsed.unwrap().operation,
        Operation::TerminalOutput {
            data: Chunk::new(Vec::new()).unwrap(),
            done: true,
        }
    );
}

#[test]
fn posix_bytes_are_lossless_bounded_and_never_repaired() {
    for bytes in [
        b"relative".as_slice(),
        b"/a/../b",
        b"/a/./b",
        b"/nul\0",
        b"//ambiguous",
        b"/trailing/",
        b"/a//b",
    ] {
        assert!(RemotePath::new(bytes).is_err(), "accepted malformed path");
    }
    assert!(RemotePath::new(vec![b'/'; MAX_PATH_BYTES + 1]).is_err());
    let path = RemotePath::new(b"/raw-\xff".as_slice()).unwrap();
    assert_eq!(path.as_bytes(), b"/raw-\xff");
    assert!(path.to_utf8().is_err());
    assert_eq!(
        RemotePath::new(b"/control\n".as_slice())
            .unwrap()
            .as_bytes(),
        b"/control\n"
    );
    assert_eq!(
        serde_json::from_slice::<RemotePath>(&serde_json::to_vec(&path).unwrap()).unwrap(),
        path
    );
    assert!(!RemotePath::new(b"/project2/a".as_slice())
        .unwrap()
        .is_within(&RemotePath::new(b"/project".as_slice()).unwrap()));
    assert!(serde_json::from_str::<RemotePath>("[114]").is_err());
    assert!(!format!("{path:?}").contains("raw"));
}

fn frame(operation: Operation) -> WireEnvelope {
    WireEnvelope::new(
        Identity {
            authority: AuthorityId([1; 16]),
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        },
        1,
        operation,
    )
}
fn path(raw: &str) -> RemotePath {
    RemotePath::new(raw.as_bytes()).unwrap()
}

#[test]
fn codec_handles_fragments_concatenation_and_truncation() {
    let original = frame(Operation::Heartbeat);
    let bytes = encode(&original).unwrap();
    for split in 0..bytes.len() {
        let mut decoder = FrameDecoder::new();
        assert!(decoder.push(&bytes[..split]).unwrap().1.is_none());
        assert!(decoder.finish().is_err() || split == 0);
        assert_eq!(
            decoder.push(&bytes[split..]).unwrap().1,
            Some(original.clone())
        );
        assert!(decoder.finish().is_ok());
    }
    let joined = [bytes.clone(), bytes.clone()].concat();
    let mut decoder = FrameDecoder::new();
    let (used, result) = decoder.push(&joined).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!(result, Some(original.clone()));
    assert_eq!(decoder.push(&joined[used..]).unwrap().1, Some(original));
}

#[test]
fn rejects_hostile_frames_versions_routing_and_closed_verbs() {
    let mut decoder = FrameDecoder::new();
    assert_eq!(
        decoder.push(&((MAX_FRAME_BYTES + 1) as u32).to_be_bytes()),
        Err(ProtocolError::Limit)
    );
    assert!(FrameDecoder::new().push(&0u32.to_be_bytes()).is_err());
    assert!(FrameDecoder::new().push(b"\0\0\0\x01{").is_err());
    let mut request = frame(Operation::List {
        path: path("/root"),
    });
    let mut reply = request.clone();
    reply.operation = Operation::Listed {
        path: path("/root"),
        entries: vec![DirectoryEntry {
            path: path("/outside"),
            kind: EntryKind::File,
        }],
        truncated: false,
        done: true,
    };
    assert!(reply.validate_reply(&request).is_err());
    reply.operation = Operation::Listed {
        path: path("/root"),
        entries: vec![DirectoryEntry {
            path: path("/root/file"),
            kind: EntryKind::File,
        }],
        truncated: false,
        done: true,
    };
    assert!(reply.validate_reply(&request).is_ok());
    for field in 0..5 {
        let mut hostile = reply.clone();
        match field {
            0 => hostile.identity.authority = AuthorityId([2; 16]),
            1 => hostile.identity.tab = TabCapability::random().unwrap(),
            2 => hostile.identity.epoch += 1,
            3 => hostile.identity.generation += 1,
            _ => hostile.request_id += 1,
        }
        assert_eq!(
            hostile.validate_reply(&request),
            Err(ProtocolError::Identity)
        );
    }
    reply.operation = Operation::TerminalInput {
        data: Chunk::new(b"secret command".to_vec()).unwrap(),
    };
    assert_eq!(
        reply.validate_reply(&request),
        Err(ProtocolError::UnexpectedResponse)
    );
    request.major = 2;
    assert_eq!(encode(&request), Err(ProtocolError::Version));
    request.major = 1;
    request.request_id = 0;
    assert!(encode(&request).is_err());
    request.request_id = 1;
    request.identity.epoch = 0;
    assert!(encode(&request).is_err());
    assert!(serde_json::from_str::<Operation>(r#"{"op":"Exec","command":"whoami"}"#).is_err());
    assert!(serde_json::from_str::<Operation>(r#"{"op":"Heartbeat","extra":true}"#).is_err());
}

#[test]
fn chunks_uploads_preview_and_debug_are_bounded() {
    assert!(Chunk::new(vec![0; MAX_CHUNK_BYTES + 1]).is_err());
    let chunk = Chunk::new(vec![255; MAX_CHUNK_BYTES]).unwrap();
    assert!(
        encode(&frame(Operation::TerminalOutput {
            data: chunk.clone(),
            done: false,
        }))
        .unwrap()
        .len()
            <= MAX_FRAME_BYTES + 4
    );
    assert!(UploadProgress::new(0).is_err());
    assert!(UploadProgress::new(MAX_UPLOAD_BYTES + 1).is_err());
    let mut upload = UploadProgress::new(4).unwrap();
    let two = Chunk::new(vec![1, 2]).unwrap();
    assert!(upload.append(1, &two).is_err());
    assert!(upload.commit().is_err());
    upload.append(0, &two).unwrap();
    assert!(upload.append(0, &two).is_err());
    assert!(upload.append(u64::MAX, &two).is_err());
    assert_eq!(upload.received(), 2);
    upload.append(2, &two).unwrap();
    upload.commit().unwrap();
    assert!(upload.append(4, &two).is_err());
    assert!(frame(Operation::PreviewResult {
        path: path("/root"),
        kind: PreviewKind::Text,
        offset: MAX_PREVIEW_BYTES,
        data: two,
        done: true
    })
    .validate()
    .is_err());
    assert!(frame(Operation::TerminalResize { rows: 0, cols: 80 })
        .validate()
        .is_err());
    let secret = frame(Operation::TerminalInput {
        data: Chunk::new(b"private payload".to_vec()).unwrap(),
    });
    let debug = format!("{secret:?}");
    assert!(!debug.contains("private payload"));
    assert!(debug.contains("TabCapability(<redacted>)"));
}

#[test]
fn request_paths_require_current_root_or_exact_transition_grant() {
    let root = path("/project");
    let outside = path("/elsewhere");
    assert!(frame(Operation::List {
        path: outside.clone()
    })
    .validate_request(&root, Some(&outside))
    .is_err());
    assert!(frame(Operation::RootSelect {
        path: outside.clone()
    })
    .validate_request(&root, None)
    .is_err());
    assert!(frame(Operation::RootSelect {
        path: outside.clone()
    })
    .validate_request(&root, Some(&outside))
    .is_ok());
    assert!(frame(Operation::Preview {
        path: path("/project/file")
    })
    .validate_request(&root, None)
    .is_ok());
    assert!(frame(Operation::Preview {
        path: path("/project-other/file")
    })
    .validate_request(&root, None)
    .is_err());
    assert!(frame(Operation::Ack).validate_request(&root, None).is_err());
}

#[test]
fn root_resolution_correlates_requested_path_and_new_generation() {
    let request = frame(Operation::RootSelect {
        path: path("/selected-link"),
    });
    let mut reply = request.clone();
    reply.operation = Operation::RootResult {
        requested: path("/selected-link"),
        path: path("/canonical-target"),
        generation: 2,
    };
    assert!(reply.validate_reply(&request).is_ok());
    reply.operation = Operation::RootResult {
        requested: path("/other"),
        path: path("/canonical-target"),
        generation: 2,
    };
    assert!(reply.validate_reply(&request).is_err());
    reply.operation = Operation::RootResult {
        requested: path("/selected-link"),
        path: path("/canonical-target"),
        generation: 1,
    };
    assert!(reply.validate_reply(&request).is_err());
}

#[test]
fn decoder_validates_version_fields_and_chunk_bounds_on_receive() {
    fn wire(payload: &[u8]) -> Vec<u8> {
        [(payload.len() as u32).to_be_bytes().as_slice(), payload].concat()
    }
    let mut original = frame(Operation::Heartbeat);
    original.major = 99;
    assert_eq!(
        FrameDecoder::new().push(&wire(&serde_json::to_vec(&original).unwrap())),
        Err(ProtocolError::Version)
    );
    original.major = 1;
    original.identity.generation = 0;
    assert_eq!(
        FrameDecoder::new().push(&wire(&serde_json::to_vec(&original).unwrap())),
        Err(ProtocolError::Identity)
    );
    original.identity.generation = 1;
    let mut json = serde_json::to_value(&original).unwrap();
    json["unexpected"] = serde_json::json!(true);
    assert!(FrameDecoder::new()
        .push(&wire(&serde_json::to_vec(&json).unwrap()))
        .is_err());
    assert!(serde_json::from_value::<Chunk>(serde_json::json!("!not base64!")).is_err());
    assert!(serde_json::from_value::<Chunk>(serde_json::json!(
        "A".repeat(MAX_CHUNK_BYTES.div_ceil(3) * 4 + 4)
    ))
    .is_err());
    assert!(serde_json::from_str::<Operation>(
        r#"{"op":"TerminalInput","data":{"data":"","argv":[]}}"#
    )
    .is_err());
}

#[test]
fn listings_are_direct_children_unique_and_frame_bounded() {
    let entry = DirectoryEntry {
        path: path("/root/file"),
        kind: EntryKind::File,
    };
    let listed = |entries| {
        frame(Operation::Listed {
            path: path("/root"),
            entries,
            truncated: false,
            done: true,
        })
    };
    assert!(listed(vec![entry.clone(), entry.clone()])
        .validate()
        .is_err());
    assert!(listed(vec![DirectoryEntry {
        path: path("/root/deep/file"),
        ..entry.clone()
    }])
    .validate()
    .is_err());
    assert!(listed(vec![entry; MAX_LIST_BATCH + 1]).validate().is_err());
    let entries = (0..MAX_LIST_BATCH)
        .map(|i| DirectoryEntry {
            path: path(&format!("/root/{i}{}", "x".repeat(3000))),
            kind: EntryKind::File,
        })
        .collect();
    assert_eq!(encode(&listed(entries)), Err(ProtocolError::Limit));
}

#[test]
fn hostile_text_preview_cannot_bypass_utf8_or_binary_classification() {
    let preview = |offset, data| {
        frame(Operation::PreviewResult {
            path: path("/root/file"),
            kind: PreviewKind::Text,
            offset,
            data: Chunk::new(data).unwrap(),
            done: true,
        })
    };
    assert!(preview(0, vec![0xff]).validate().is_ok());
    let request = frame(Operation::Preview {
        path: path("/root/file"),
    });
    let mut progress = ReplyProgress::new(request.clone()).unwrap();
    assert!(progress
        .accept(&response(&request, preview(0, vec![0xff]).operation))
        .is_err());
    assert!(preview(0, b"looks-text\0binary".to_vec())
        .validate()
        .is_err());
    // Preserve the existing shared preview classifier's 8192-byte sniff window.
    assert!(preview(8192, vec![0]).validate().is_ok());
}

#[test]
fn completed_upload_path_cannot_inject_terminal_control_or_newline() {
    for raw in [
        "/private/clipboard-image\n.png",
        "/private/clipboard-\u{1b}[31m.png",
    ] {
        assert!(frame(Operation::UploadResult {
            upload_id: 1,
            path: path(raw)
        })
        .validate()
        .is_err());
    }
    assert!(frame(Operation::UploadResult {
        upload_id: 1,
        path: path("/Application Support/Made/clipboard-1.png")
    })
    .validate()
    .is_ok());
}

#[test]
fn partial_decoder_debug_never_exposes_raw_buffer_or_capability() {
    let original = frame(Operation::Focus {
        session_id: "SECRET_PARTIAL_CONTENT".into(),
    });
    let bytes = encode(&original).unwrap();
    let mut decoder = FrameDecoder::new();
    decoder.push(&bytes[..bytes.len() - 1]).unwrap();
    let debug = format!("{decoder:?}");
    assert!(debug.contains("buffered_bytes"));
    assert!(!debug.contains("buffer:"));
    assert!(!debug.contains("SECRET_PARTIAL_CONTENT"));
    assert!(!debug.contains("83, 69, 67, 82, 69, 84"));
    assert!(!debug.contains("identity"));
    assert!(!debug.contains('['));
}

fn response(request: &WireEnvelope, operation: Operation) -> WireEnvelope {
    WireEnvelope::new(request.identity.clone(), request.request_id, operation)
}

#[test]
fn reply_progress_rejects_cross_batch_duplicates_total_limit_and_post_done() {
    let request = frame(Operation::List {
        path: path("/root"),
    });
    let listed = |names: Vec<RemotePath>, done| {
        response(
            &request,
            Operation::Listed {
                path: path("/root"),
                entries: names
                    .into_iter()
                    .map(|path| DirectoryEntry {
                        path,
                        kind: EntryKind::File,
                    })
                    .collect(),
                truncated: false,
                done,
            },
        )
    };
    let mut duplicate = ReplyProgress::new(request.clone()).unwrap();
    duplicate
        .accept(&listed(vec![path("/root/a")], false))
        .unwrap();
    assert!(duplicate
        .accept(&listed(vec![path("/root/a")], true))
        .is_err());
    assert!(duplicate
        .accept(&listed(vec![path("/root/b")], true))
        .is_err());
    let mut progress = ReplyProgress::new(request.clone()).unwrap();
    for start in (0..MAX_DIRECTORY_ENTRIES).step_by(MAX_LIST_BATCH) {
        let end = (start + MAX_LIST_BATCH).min(MAX_DIRECTORY_ENTRIES);
        progress
            .accept(&listed(
                (start..end).map(|n| path(&format!("/root/{n}"))).collect(),
                false,
            ))
            .unwrap();
    }
    assert_eq!(progress.entry_count(), MAX_DIRECTORY_ENTRIES);
    assert!(progress
        .accept(&listed(vec![path("/root/excess")], true))
        .is_err());
    let mut complete = ReplyProgress::new(request.clone()).unwrap();
    complete.accept(&listed(vec![], true)).unwrap();
    assert!(complete.is_done());
    complete.finish().unwrap();
    assert!(complete.accept(&listed(vec![], true)).is_err());
    let mut empty = ReplyProgress::new(request.clone()).unwrap();
    assert!(empty.accept(&listed(vec![], false)).is_err());
}

#[test]
fn reply_progress_correlates_every_frame_and_requires_contiguous_fixed_type_streams() {
    let request = frame(Operation::Preview {
        path: path("/root/file"),
    });
    let text = |offset, data: &[u8], done| {
        response(
            &request,
            Operation::PreviewResult {
                path: path("/root/file"),
                kind: PreviewKind::Text,
                offset,
                data: Chunk::new(data.to_vec()).unwrap(),
                done,
            },
        )
    };
    let mut progress = ReplyProgress::new(request.clone()).unwrap();
    progress.accept(&text(0, b"ab", false)).unwrap();
    assert_eq!(progress.received_bytes(), 2);
    assert!(progress.finish().is_err());
    assert!(progress.accept(&text(1, b"c", true)).is_err());
    for mismatch in 0..5 {
        let mut progress = ReplyProgress::new(request.clone()).unwrap();
        progress.accept(&text(0, b"ab", false)).unwrap();
        let mut bad = text(2, b"c", true);
        match mismatch {
            0 => bad.identity.epoch += 1,
            1 => bad.identity.generation += 1,
            2 => bad.request_id += 1,
            3 => bad.identity.authority = AuthorityId([9; 16]),
            _ => bad.identity.tab = TabCapability::random().unwrap(),
        }
        assert_eq!(progress.accept(&bad), Err(ProtocolError::Identity));
    }
    let mut changed = ReplyProgress::new(request.clone()).unwrap();
    changed.accept(&text(0, b"", false)).unwrap_err();
    let mut changed = ReplyProgress::new(request.clone()).unwrap();
    changed.accept(&text(0, b"a", false)).unwrap();
    assert!(changed
        .accept(&response(
            &request,
            Operation::PreviewResult {
                path: path("/root/file"),
                kind: PreviewKind::Missing,
                offset: 0,
                data: Chunk::new(vec![]).unwrap(),
                done: true
            }
        ))
        .is_err());
    let mut complete = ReplyProgress::new(request.clone()).unwrap();
    complete.accept(&text(0, b"abc", true)).unwrap();
    assert!(complete.accept(&text(3, b"d", true)).is_err());
    let git = frame(Operation::GitRead {
        command: GitRead::Status,
    });
    let chunk = |offset, done| {
        response(
            &git,
            Operation::GitResult {
                command: GitRead::Status,
                offset,
                data: Chunk::new(vec![255; MAX_CHUNK_BYTES]).unwrap(),
                done,
            },
        )
    };
    let mut progress = ReplyProgress::new(git.clone()).unwrap();
    for offset in (0..MAX_GIT_BYTES).step_by(MAX_CHUNK_BYTES) {
        progress.accept(&chunk(offset, false)).unwrap();
    }
    assert!(progress.accept(&chunk(MAX_GIT_BYTES, true)).is_err());
    let mut gap = ReplyProgress::new(git.clone()).unwrap();
    assert!(gap.accept(&chunk(1, true)).is_err());
}

#[test]
fn reply_progress_validates_utf8_across_any_multibyte_boundary_and_final_tail() {
    let request = frame(Operation::Preview {
        path: path("/root/file"),
    });
    let text = |offset, data: &[u8], done| {
        response(
            &request,
            Operation::PreviewResult {
                path: path("/root/file"),
                kind: PreviewKind::Text,
                offset,
                data: Chunk::new(data.to_vec()).unwrap(),
                done,
            },
        )
    };
    let bytes = "aé中🦀z".as_bytes();
    for split in 1..bytes.len() {
        let first = text(0, &bytes[..split], false);
        // The frame codec may not assume chunks end on UTF-8 boundaries.
        encode(&first).unwrap();
        let mut progress = ReplyProgress::new(request.clone()).unwrap();
        progress.accept(&first).unwrap();
        progress
            .accept(&text(split as u64, &bytes[split..], true))
            .unwrap();
        assert!(progress.is_done());
        progress.finish().unwrap();
    }
    let mut bytewise = ReplyProgress::new(request.clone()).unwrap();
    for (offset, byte) in bytes.iter().enumerate() {
        bytewise
            .accept(&text(offset as u64, &[*byte], offset + 1 == bytes.len()))
            .unwrap();
    }
    bytewise.finish().unwrap();
    for invalid in [
        vec![0xe2],
        vec![0xff],
        vec![0xc0, 0x80],
        vec![0xed, 0xa0, 0x80],
        b"a\0b".to_vec(),
    ] {
        let mut progress = ReplyProgress::new(request.clone()).unwrap();
        assert!(progress.accept(&text(0, &invalid, true)).is_err());
    }
    let mut continuation = ReplyProgress::new(request.clone()).unwrap();
    continuation.accept(&text(0, &[0xf0], false)).unwrap();
    assert!(continuation.accept(&text(1, b"X", true)).is_err());
    let mut complete = ReplyProgress::new(request.clone()).unwrap();
    for offset in (0..MAX_PREVIEW_BYTES).step_by(MAX_CHUNK_BYTES) {
        complete
            .accept(&text(offset, &vec![b'x'; MAX_CHUNK_BYTES], false))
            .unwrap();
    }
    assert!(complete
        .accept(&text(MAX_PREVIEW_BYTES, b"x", true))
        .is_err());
}

fn grok_session(id: &str) -> GrokSessionInfo {
    GrokSessionInfo {
        session_id: id.to_owned(),
        cwd: None,
        opened_at: 0,
        activity: GrokActivity::Idle,
    }
}

fn grok_snapshot(revision: u64, sessions: Vec<GrokSessionInfo>) -> GrokSessionSnapshot {
    GrokSessionSnapshot {
        revision,
        running: false,
        hooks_ready: false,
        sessions,
    }
}

fn grok_event(kind: GrokEventKind, session_id: &str) -> GrokSessionEvent {
    GrokSessionEvent {
        kind,
        session_id: session_id.to_owned(),
        cwd: None,
        activity: GrokActivity::Idle,
    }
}

#[test]
fn grok_launch_target_is_additive_and_major_stays_one() {
    assert_eq!(PROTOCOL_MAJOR, 1);
    assert_eq!(
        serde_json::from_str::<LaunchTarget>(r#""Grok""#).unwrap(),
        LaunchTarget::Grok
    );
    assert!(serde_json::from_str::<LaunchTarget>(r#""Codex""#).is_err());
    assert!(serde_json::from_str::<Operation>(
        r#"{"op":"Open","data":{"directory":"/project","target":"Codex","rows":24,"cols":80}}"#
    )
    .is_err());
    assert!(frame(Operation::Open {
        directory: path("/project"),
        target: LaunchTarget::Grok,
        rows: 24,
        cols: 80,
    })
    .validate()
    .is_ok());
}

#[test]
fn grok_snapshot_hook_tier_is_additive_with_a_legacy_default() {
    let legacy = serde_json::from_value::<GrokSessionSnapshot>(serde_json::json!({
        "revision": 1,
        "running": true,
        "sessions": []
    }))
    .unwrap();
    assert!(!legacy.hooks_ready);

    let mut current = grok_snapshot(1, Vec::new());
    current.hooks_ready = true;
    let encoded = serde_json::to_value(&current).unwrap();
    assert_eq!(encoded["hooks_ready"], true);
    assert_eq!(
        serde_json::from_value::<GrokSessionSnapshot>(encoded).unwrap(),
        current
    );
}

#[test]
fn grok_state_reply_pairs_by_full_identity_and_events_stay_outside_replies() {
    let request = frame(Operation::GrokState);
    request.validate().unwrap();
    request.validate_request(&path("/project"), None).unwrap();
    let snapshot = grok_snapshot(1, Vec::new());
    let reply = response(&request, Operation::GrokSnapshot { snapshot });
    reply.validate_reply(&request).unwrap();
    for field in 0..5 {
        let mut hostile = response(
            &request,
            Operation::GrokSnapshot {
                snapshot: grok_snapshot(1, Vec::new()),
            },
        );
        match field {
            0 => hostile.identity.authority = AuthorityId([2; 16]),
            1 => hostile.identity.tab = TabCapability::random().unwrap(),
            2 => hostile.identity.epoch += 1,
            3 => hostile.identity.generation += 1,
            _ => hostile.request_id += 1,
        }
        assert_eq!(
            hostile.validate_reply(&request),
            Err(ProtocolError::Identity)
        );
    }
    // Unsolicited Grok events and snapshots are never correlated replies and
    // never client request verbs; routing stays with the current attach source.
    let event = response(
        &request,
        Operation::GrokEvent {
            event: grok_event(GrokEventKind::Prompt, "s0"),
        },
    );
    assert_eq!(
        event.validate_reply(&request),
        Err(ProtocolError::UnexpectedResponse)
    );
    assert_eq!(
        frame(Operation::GrokSnapshot {
            snapshot: grok_snapshot(1, Vec::new())
        })
        .validate_request(&path("/project"), None),
        Err(ProtocolError::UnexpectedResponse)
    );
    assert_eq!(
        frame(Operation::GrokEvent {
            event: grok_event(GrokEventKind::Prompt, "s0")
        })
        .validate_request(&path("/project"), None),
        Err(ProtocolError::UnexpectedResponse)
    );
    // Grok state traffic is never interactive terminal traffic.
    assert!(!event.operation.is_terminal());
    assert!(!reply.operation.is_terminal());
    // The correlated snapshot completes the state request exactly once.
    let mut progress = ReplyProgress::new(request.clone()).unwrap();
    progress.accept(&reply).unwrap();
    progress.finish().unwrap();
    assert!(progress.accept(&reply).is_err());
    assert!(ReplyProgress::new(event).is_err());
    assert!(ReplyProgress::new(frame(Operation::GrokSnapshot {
        snapshot: grok_snapshot(1, Vec::new())
    }))
    .is_err());
}

#[test]
fn grok_snapshots_are_bounded_unique_and_posix_validated() {
    let full = frame(Operation::GrokSnapshot {
        snapshot: grok_snapshot(
            1,
            (0..made_bridge::state::MAX_SESSIONS)
                .map(|i| grok_session(&format!("s{i}")))
                .collect(),
        ),
    });
    full.validate().unwrap();
    assert_eq!(
        frame(Operation::GrokSnapshot {
            snapshot: grok_snapshot(0, Vec::new())
        })
        .validate(),
        Err(ProtocolError::Malformed)
    );
    let sessions = |sessions: Vec<GrokSessionInfo>| {
        frame(Operation::GrokSnapshot {
            snapshot: grok_snapshot(1, sessions),
        })
    };
    assert!(sessions(vec![grok_session("a"), grok_session("a")])
        .validate()
        .is_err());
    assert!(sessions(
        (0..made_bridge::state::MAX_SESSIONS + 1)
            .map(|i| grok_session(&format!("s{i}")))
            .collect()
    )
    .validate()
    .is_err());
    for id in [
        "",
        "a\0b",
        "a\nb",
        &"x".repeat(made_bridge::state::MAX_SESSION_ID_CHARS + 1),
    ] {
        assert!(sessions(vec![grok_session(id)]).validate().is_err());
    }
    // Session directories are absolute traversal-free remote POSIX bytes.
    assert!(
        serde_json::from_value::<GrokSessionInfo>(serde_json::json!({
            "session_id": "s0", "opened_at": 0, "activity": "Idle", "cwd": "relative"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<GrokSessionInfo>(serde_json::json!({
            "session_id": "s0", "opened_at": 0, "activity": "Idle", "cwd": "/a/../b"
        }))
        .is_err()
    );
    // The whole snapshot stays inside one frame even at the session cap.
    let heavy = sessions(
        (0..made_bridge::state::MAX_SESSIONS)
            .map(|i| GrokSessionInfo {
                session_id: format!("s{i}"),
                cwd: Some(path(&format!("/{}", "x".repeat(MAX_PATH_BYTES - 1)))),
                opened_at: i as u64,
                activity: GrokActivity::Idle,
            })
            .collect(),
    );
    assert_eq!(encode(&heavy), Err(ProtocolError::Limit));
}

#[test]
fn grok_events_carry_the_local_equivalent_semantics_and_bounds() {
    for kind in [
        GrokEventKind::SessionStart,
        GrokEventKind::Prompt,
        GrokEventKind::Stop,
        GrokEventKind::Error,
        GrokEventKind::Cancel,
        GrokEventKind::SessionEnd,
    ] {
        let event = frame(Operation::GrokEvent {
            event: grok_event(kind, "s0"),
        });
        event.validate().unwrap();
        let encoded = encode(&event).unwrap();
        let (_, parsed) = FrameDecoder::new().push(&encoded).unwrap();
        assert_eq!(
            parsed.unwrap().operation,
            Operation::GrokEvent {
                event: grok_event(kind, "s0")
            }
        );
    }
    for id in [
        "",
        "a\0b",
        &"x".repeat(made_bridge::state::MAX_SESSION_ID_CHARS + 1),
    ] {
        assert!(frame(Operation::GrokEvent {
            event: grok_event(GrokEventKind::Stop, id)
        })
        .validate()
        .is_err());
    }
    // Event directories are validated remote POSIX bytes or absent.
    assert!(
        serde_json::from_value::<GrokSessionEvent>(serde_json::json!({
            "kind": "Prompt", "session_id": "s0", "activity": "Working", "cwd": "/a/./b"
        }))
        .is_err()
    );
    // Unknown kinds, activities and fields stay malformed; no event metadata
    // beyond the bounded classification rides the wire.
    assert!(serde_json::from_str::<Operation>(
        r#"{"op":"GrokEvent","data":{"event":{"kind":"Exploded","session_id":"s0","activity":"Idle"}}}"#
    )
    .is_err());
    assert!(serde_json::from_str::<Operation>(
        r#"{"op":"GrokEvent","data":{"event":{"kind":"Prompt","session_id":"s0","activity":"Busy"}}}"#
    )
    .is_err());
    assert!(serde_json::from_str::<Operation>(
        r#"{"op":"GrokEvent","data":{"event":{"kind":"Prompt","session_id":"s0","activity":"Idle","at":"2026-01-01T00:00:00Z"}}}"#
    )
    .is_err());
    assert!(serde_json::from_str::<Operation>(
        r#"{"op":"GrokEvent","data":{"event":{"kind":"Prompt","session_id":"s0","activity":"Idle"},"extra":1}}"#
    )
    .is_err());
}

#[test]
fn grok_capabilities_are_backward_safe_and_session_snapshot_is_not_overloaded() {
    // An older companion frame without the additive fields stays honest: the
    // capabilities deserialize as absent, not as a claimed availability.
    let legacy: Capabilities = serde_json::from_str(
        r#"{"target":"LinuxX64","tmux":false,"opencode":true,"shell_cwd":false,"helper_version":"1"}"#,
    )
    .unwrap();
    assert!(!legacy.grok);
    assert!(!legacy.python3);
    let caps: Capabilities = serde_json::from_str(
        r#"{"target":"LinuxX64","tmux":false,"opencode":true,"shell_cwd":false,"helper_version":"1","grok":true,"python3":true}"#,
    )
    .unwrap();
    assert!(caps.grok);
    // `python3` is frozen in major 1 like `tmux`: an older peer's true value
    // still decodes, no consumer reads it, and the companion encodes false.
    assert!(caps.python3);
    let encoded = serde_json::to_string(&Capabilities {
        python3: false,
        watch_dirs: false,
        start_shell: false,
        ..caps
    })
    .unwrap();
    assert!(encoded.contains(r#""python3":false"#), "{encoded}");
    assert!(serde_json::from_str::<Capabilities>(
        r#"{"target":"LinuxX64","tmux":false,"opencode":true,"shell_cwd":false,"helper_version":"1","codex":true}"#
    )
    .is_err());
    // Grok state travels only in its own operation: the OpenCode snapshot
    // shape cannot carry a Grok payload.
    assert!(serde_json::from_str::<Operation>(
        r#"{"op":"SessionSnapshot","data":{"snapshot":{"revision":1,"running":false,"sessions":[]}}}"#
    )
    .is_err());
    let grok = frame(Operation::GrokSnapshot {
        snapshot: grok_snapshot(1, Vec::new()),
    });
    let value = serde_json::to_value(&grok.operation).unwrap();
    assert_eq!(value["op"], "GrokSnapshot");
}

#[test]
fn unary_reply_is_terminal_and_truncation_is_final_only() {
    let request = frame(Operation::Heartbeat);
    let mut progress = ReplyProgress::new(request.clone()).unwrap();
    progress
        .accept(&response(&request, Operation::Ack))
        .unwrap();
    progress.finish().unwrap();
    assert!(progress
        .accept(&response(
            &request,
            Operation::Error {
                code: ErrorCode::Io
            }
        ))
        .is_err());
    assert!(progress.finish().is_err());
    let request = frame(Operation::List {
        path: path("/root"),
    });
    let listed = |done| {
        response(
            &request,
            Operation::Listed {
                path: path("/root"),
                entries: vec![DirectoryEntry {
                    path: path("/root/file"),
                    kind: EntryKind::File,
                }],
                truncated: true,
                done,
            },
        )
    };
    let mut bad = ReplyProgress::new(request.clone()).unwrap();
    assert!(bad.accept(&listed(false)).is_err());
    let mut good = ReplyProgress::new(request.clone()).unwrap();
    good.accept(&listed(true)).unwrap();
    assert!(good.is_truncated());
    good.finish().unwrap();
}

#[test]
fn claude_launch_target_is_additive_and_keeps_existing_encodings() {
    assert_eq!(PROTOCOL_MAJOR, 1);
    assert_eq!(
        serde_json::from_str::<LaunchTarget>(r#""Claude""#).unwrap(),
        LaunchTarget::Claude
    );
    assert_eq!(
        serde_json::to_string(&LaunchTarget::Claude).unwrap(),
        r#""Claude""#
    );
    for (text, target) in [
        (r#""Shell""#, LaunchTarget::Shell),
        (r#""OpenCode""#, LaunchTarget::OpenCode),
        (r#""Grok""#, LaunchTarget::Grok),
    ] {
        assert_eq!(serde_json::from_str::<LaunchTarget>(text).unwrap(), target);
    }
    assert!(frame(Operation::Open {
        directory: path("/project"),
        target: LaunchTarget::Claude,
        rows: 24,
        cols: 80,
    })
    .validate()
    .is_ok());
}

fn claude_session(id: &str, pid: u32) -> ClaudeSessionInfo {
    ClaudeSessionInfo {
        session_id: id.to_owned(),
        pid,
        cwd: Some(path("/work")),
        activity: ClaudeActivity::Idle,
        title: String::new(),
        recency: 1,
        nested: false,
    }
}

fn claude_snapshot(revision: u64, sessions: Vec<ClaudeSessionInfo>) -> ClaudeSessionSnapshot {
    ClaudeSessionSnapshot {
        revision,
        hooks_available: true,
        sessions,
    }
}

#[test]
fn claude_state_is_additive_and_pairs_only_with_its_snapshot() {
    assert_eq!(PROTOCOL_MAJOR, 1);
    // `Capabilities` is unchanged: no Claude Code field is added.
    assert!(serde_json::from_value::<Capabilities>(serde_json::json!({
        "target": "LinuxX64", "tmux": false, "opencode": false, "shell_cwd": false,
        "helper_version": "1", "grok": false, "python3": false, "claude": true
    }))
    .is_err());
    let request = frame(Operation::ClaudeState);
    request.validate_request(&path("/project"), None).unwrap();
    let reply = response(
        &request,
        Operation::ClaudeSnapshot {
            snapshot: claude_snapshot(1, vec![claude_session("s1", 7)]),
        },
    );
    reply.validate_reply(&request).unwrap();
    let mut progress = ReplyProgress::new(request.clone()).unwrap();
    progress.accept(&reply).unwrap();
    progress.finish().unwrap();
    let mut hostile = reply.clone();
    hostile.identity.epoch += 1;
    assert_eq!(
        hostile.validate_reply(&request),
        Err(ProtocolError::Identity)
    );
    // Pushes are never replies and never client requests.
    let event = response(
        &request,
        Operation::ClaudeEvent {
            snapshot: claude_snapshot(2, Vec::new()),
        },
    );
    event.validate().unwrap();
    assert_eq!(
        event.validate_reply(&request),
        Err(ProtocolError::UnexpectedResponse)
    );
    let listing = response(
        &request,
        Operation::ClaudeListing {
            listing: ClaudeListingResult {
                revision: 3,
                background: Vec::new(),
            },
        },
    );
    listing.validate().unwrap();
    assert_eq!(
        listing.validate_reply(&request),
        Err(ProtocolError::UnexpectedResponse)
    );
    for push in [&event, &listing, &reply] {
        assert_eq!(
            frame(push.operation.clone()).validate_request(&path("/project"), None),
            Err(ProtocolError::UnexpectedResponse)
        );
        assert!(ReplyProgress::new(frame(push.operation.clone())).is_err());
        assert!(!push.operation.is_terminal());
    }
}

#[test]
fn the_claude_listing_trigger_carries_no_input_or_title_bytes() {
    let trigger = frame(Operation::ClaudeTrigger);
    trigger.validate_request(&path("/project"), None).unwrap();
    let encoded = serde_json::to_value(&trigger.operation).unwrap();
    assert_eq!(encoded, serde_json::json!({ "op": "ClaudeTrigger" }));
    // A payload is not part of the verb: nothing can ride along.
    assert!(serde_json::from_value::<Operation>(
        serde_json::json!({ "op": "ClaudeTrigger", "data": { "input": "secret" } })
    )
    .is_err());
    let ack = response(&trigger, Operation::Ack);
    ack.validate_reply(&trigger).unwrap();
    assert!(ReplyProgress::new(trigger.clone()).is_ok());
    assert!(!trigger.operation.is_terminal());
}

#[test]
fn claude_listing_results_relay_only_pid_cwd_and_background_name_cwd() {
    // The owned pid's listed directory rides the session it belongs to.
    let session = serde_json::to_value(claude_session("s1", 7)).unwrap();
    let mut keys: Vec<&str> = session
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "activity",
            "cwd",
            "nested",
            "pid",
            "recency",
            "session_id",
            "title"
        ]
    );
    let listing = ClaudeListingResult {
        revision: 1,
        background: vec![ClaudeBackgroundEntry {
            name: "Spec task".to_owned(),
            cwd: path("/work/spec"),
        }],
    };
    let encoded = serde_json::to_value(&listing).unwrap();
    assert_eq!(
        encoded["background"][0].as_object().unwrap().len(),
        2,
        "only name and cwd"
    );
    assert_eq!(
        serde_json::from_value::<ClaudeListingResult>(encoded).unwrap(),
        listing
    );
    // Unknown fields, such as a prompt or a transcript path, are rejected.
    assert!(
        serde_json::from_value::<ClaudeBackgroundEntry>(serde_json::json!({
            "name": "x", "cwd": [47], "prompt": "MARKER"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<ClaudeSessionInfo>(serde_json::json!({
            "session_id": "s", "pid": 1, "activity": "Idle", "recency": 1, "msg": "MARKER"
        }))
        .is_err()
    );
}

#[test]
fn claude_frames_are_bounded_and_oversized_ones_are_rejected() {
    let snapshot = |sessions| {
        frame(Operation::ClaudeEvent {
            snapshot: claude_snapshot(1, sessions),
        })
    };
    snapshot(
        (0..made_bridge::state::MAX_SESSIONS)
            .map(|i| claude_session(&format!("s{i}"), i as u32 + 1))
            .collect(),
    )
    .validate()
    .unwrap();
    assert!(snapshot(
        (0..=made_bridge::state::MAX_SESSIONS)
            .map(|i| claude_session(&format!("s{i}"), i as u32 + 1))
            .collect()
    )
    .validate()
    .is_err());
    assert!(
        snapshot(vec![claude_session("a", 1), claude_session("a", 2)])
            .validate()
            .is_err()
    );
    assert!(snapshot(vec![claude_session("a", 0)]).validate().is_err());
    for id in [
        "",
        "a\nb",
        &"x".repeat(made_bridge::state::MAX_SESSION_ID_CHARS + 1),
    ] {
        assert!(snapshot(vec![claude_session(id, 1)]).validate().is_err());
    }
    for title in ["bad\u{7}title".to_owned(), "x".repeat(121)] {
        let mut session = claude_session("s", 1);
        session.title = title;
        assert!(snapshot(vec![session]).validate().is_err());
    }
    assert!(frame(Operation::ClaudeSnapshot {
        snapshot: claude_snapshot(0, Vec::new())
    })
    .validate()
    .is_err());
    let listing = |background: Vec<ClaudeBackgroundEntry>| {
        frame(Operation::ClaudeListing {
            listing: ClaudeListingResult {
                revision: 1,
                background,
            },
        })
    };
    let entry = |name: &str| ClaudeBackgroundEntry {
        name: name.to_owned(),
        cwd: path("/work"),
    };
    listing(vec![entry("a"); MAX_CLAUDE_BACKGROUND])
        .validate()
        .unwrap();
    assert!(listing(vec![entry("a"); MAX_CLAUDE_BACKGROUND + 1])
        .validate()
        .is_err());
    for name in ["", "a\u{1b}b", &"n".repeat(121)] {
        assert!(listing(vec![entry(name)]).validate().is_err());
    }
    assert!(frame(Operation::ClaudeListing {
        listing: ClaudeListingResult {
            revision: 0,
            background: Vec::new()
        }
    })
    .validate()
    .is_err());
    // An oversized frame fails at the codec bound on both sides.
    let long =
        RemotePath::new(format!("/{}", "d".repeat(MAX_PATH_BYTES - 1)).into_bytes()).unwrap();
    let huge = frame(Operation::ClaudeEvent {
        snapshot: claude_snapshot(
            1,
            (0..made_bridge::state::MAX_SESSIONS)
                .map(|i| ClaudeSessionInfo {
                    cwd: Some(long.clone()),
                    ..claude_session(&format!("s{i}"), i as u32 + 1)
                })
                .collect(),
        ),
    });
    assert_eq!(encode(&huge), Err(ProtocolError::Limit));
    let mut oversized = (MAX_FRAME_BYTES as u32 + 1).to_be_bytes().to_vec();
    oversized.extend_from_slice(b"{");
    assert!(FrameDecoder::new().push(&oversized).is_err());
}

#[test]
fn open_url_request_is_bounded_normalized_and_server_only() {
    fn wire(payload: &[u8]) -> Vec<u8> {
        [(payload.len() as u32).to_be_bytes().as_slice(), payload].concat()
    }
    let ok = frame(Operation::OpenUrlRequest {
        url: "https://example.com/a?b=1".into(),
    });
    assert!(ok.validate().is_ok());
    let bytes = encode(&ok).unwrap();
    assert_eq!(
        FrameDecoder::new().push(&bytes).unwrap().1,
        Some(ok.clone())
    );
    assert!(!format!("{:?}", ok.operation).contains("example"));
    let max = format!(
        "https://e.co/{}",
        "a".repeat(made_platform::open_url::MAX_URL_BYTES - 13)
    );
    for bad in [
        String::new(),
        "file:///etc/passwd".into(),
        "javascript:alert(1)".into(),
        "https://user@example.com/".into(),
        "https://example.com/\u{202e}x".into(),
        "https://example.com/\u{1b}[2J".into(),
        "HTTPS://example.com/".into(),
        format!("{max}a"),
    ] {
        let hostile = frame(Operation::OpenUrlRequest { url: bad });
        assert_eq!(hostile.validate(), Err(ProtocolError::Malformed));
        let payload = serde_json::to_vec(&hostile).unwrap();
        assert!(FrameDecoder::new().push(&wire(&payload)).is_err());
    }
    assert!(frame(Operation::OpenUrlRequest { url: max })
        .validate()
        .is_ok());
    // A client never sends it: it is not a request verb.
    assert_eq!(
        ok.validate_request(&path("/project"), None),
        Err(ProtocolError::UnexpectedResponse)
    );
    assert!(serde_json::from_str::<Operation>(
        r#"{"op":"OpenUrlRequest","data":{"url":"https://a.example/","open":true}}"#
    )
    .is_err());
}

fn browse_identity() -> BrowseIdentity {
    BrowseIdentity {
        authority: AuthorityId([7; 16]),
        grant: BrowseGrant::random().unwrap(),
        epoch: 3,
    }
}

fn browse(
    identity: &BrowseIdentity,
    request_id: u64,
    operation: BrowseOperation,
) -> BrowseEnvelope {
    BrowseEnvelope::new(identity.clone(), request_id, operation)
}

fn decode_browse(bytes: &[u8]) -> Result<BrowseEnvelope, ProtocolError> {
    let mut decoder = BrowseFrameDecoder::new();
    let (consumed, frame) = decoder.push(bytes)?;
    assert_eq!(consumed, bytes.len());
    decoder.finish()?;
    frame.ok_or(ProtocolError::Malformed)
}

fn wire(payload: &[u8]) -> Vec<u8> {
    let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(payload);
    bytes
}

fn listed_entries(entries: impl IntoIterator<Item = String>) -> Vec<DirectoryEntry> {
    entries
        .into_iter()
        .map(|raw| DirectoryEntry {
            path: path(&raw),
            kind: EntryKind::File,
        })
        .collect()
}

#[test]
fn browse_requests_and_replies_round_trip_under_one_grant() {
    let identity = browse_identity();
    let dir = path("/srv/project");
    let child = path("/srv/project/src");
    let mut requests = vec![
        BrowseOperation::BrowseOpen,
        BrowseOperation::BrowseClose,
        BrowseOperation::BrowseList { path: dir.clone() },
        BrowseOperation::BrowseCheck { path: dir.clone() },
    ];
    for command in [
        GitRead::ShowToplevel,
        GitRead::Branch,
        GitRead::GitDir,
        GitRead::CommonDir,
        GitRead::WorktreeList,
        GitRead::CommitId,
        GitRead::CommitSubject,
        GitRead::Status,
    ] {
        requests.push(BrowseOperation::BrowseGitRead {
            path: dir.clone(),
            command,
        });
    }
    for (index, operation) in requests.into_iter().enumerate() {
        let request = browse(&identity, index as u64 + 1, operation);
        request.validate_request().unwrap();
        let decoded = decode_browse(&encode_browse(&request).unwrap()).unwrap();
        assert_eq!(decoded, request);
        BrowseReplyProgress::new(request).unwrap();
    }

    let reply_pairs = [
        (
            BrowseOperation::BrowseOpen,
            BrowseOperation::BrowseOpened {
                home: Some(path("/home/user")),
            },
        ),
        (
            BrowseOperation::BrowseOpen,
            BrowseOperation::BrowseOpened { home: None },
        ),
        (BrowseOperation::BrowseClose, BrowseOperation::BrowseClosed),
        (
            BrowseOperation::BrowseList { path: dir.clone() },
            BrowseOperation::BrowseListed {
                path: dir.clone(),
                entries: vec![DirectoryEntry {
                    path: child.clone(),
                    kind: EntryKind::Directory,
                }],
                truncated: false,
                done: true,
            },
        ),
        (
            BrowseOperation::BrowseCheck { path: dir.clone() },
            BrowseOperation::BrowseChecked {
                requested: dir.clone(),
                result: DirectoryCheck::Directory {
                    path: path("/private/srv/project"),
                },
            },
        ),
        (
            BrowseOperation::BrowseCheck { path: dir.clone() },
            BrowseOperation::BrowseChecked {
                requested: dir.clone(),
                result: DirectoryCheck::Missing,
            },
        ),
        (
            BrowseOperation::BrowseCheck { path: dir.clone() },
            BrowseOperation::BrowseChecked {
                requested: dir.clone(),
                result: DirectoryCheck::Unreadable,
            },
        ),
        (
            BrowseOperation::BrowseCheck { path: dir.clone() },
            BrowseOperation::BrowseChecked {
                requested: dir.clone(),
                result: DirectoryCheck::NotDirectory,
            },
        ),
        (
            BrowseOperation::BrowseGitRead {
                path: dir.clone(),
                command: GitRead::Status,
            },
            BrowseOperation::BrowseGitResult {
                path: dir.clone(),
                command: GitRead::Status,
                offset: 0,
                data: Chunk::new(b" M a\0".to_vec()).unwrap(),
                done: true,
            },
        ),
        (
            BrowseOperation::BrowseList { path: dir.clone() },
            BrowseOperation::Error {
                code: ErrorCode::Unavailable,
            },
        ),
    ];
    for (request, reply) in reply_pairs {
        let request = browse(&identity, 9, request);
        let reply = browse(&identity, 9, reply);
        let decoded = decode_browse(&encode_browse(&reply).unwrap()).unwrap();
        assert_eq!(decoded, reply);
        reply.validate_reply(&request).unwrap();
        let mut progress = BrowseReplyProgress::new(request).unwrap();
        progress.accept(&reply).unwrap();
        progress.finish().unwrap();
        // A reply is never itself a request.
        assert_eq!(
            reply.validate_request(),
            Err(ProtocolError::UnexpectedResponse)
        );
        assert!(BrowseReplyProgress::new(reply).is_err());
    }

    // Replies must echo the request's path and Git command.
    let list = browse(
        &identity,
        4,
        BrowseOperation::BrowseList { path: dir.clone() },
    );
    let other_listed = browse(
        &identity,
        4,
        BrowseOperation::BrowseListed {
            path: child.clone(),
            entries: vec![],
            truncated: false,
            done: true,
        },
    );
    assert_eq!(
        other_listed.validate_reply(&list),
        Err(ProtocolError::UnexpectedResponse)
    );
    let git = browse(
        &identity,
        5,
        BrowseOperation::BrowseGitRead {
            path: dir.clone(),
            command: GitRead::Branch,
        },
    );
    for wrong in [
        BrowseOperation::BrowseGitResult {
            path: dir.clone(),
            command: GitRead::Status,
            offset: 0,
            data: Chunk::new(vec![]).unwrap(),
            done: true,
        },
        BrowseOperation::BrowseGitResult {
            path: child.clone(),
            command: GitRead::Branch,
            offset: 0,
            data: Chunk::new(vec![]).unwrap(),
            done: true,
        },
        BrowseOperation::BrowseChecked {
            requested: dir.clone(),
            result: DirectoryCheck::Missing,
        },
        BrowseOperation::BrowseOpened { home: None },
    ] {
        assert_eq!(
            browse(&identity, 5, wrong).validate_reply(&git),
            Err(ProtocolError::UnexpectedResponse)
        );
    }
    // The grant never appears in Debug output.
    assert_eq!(format!("{:?}", identity.grant), "BrowseGrant(<redacted>)");
    assert_eq!(
        format!("{:?}", BrowseOperation::BrowseList { path: dir }),
        "BrowseOperation(<redacted>)"
    );
}

#[test]
fn browse_frames_reject_unknown_fields_oversize_and_traversal() {
    let identity = browse_identity();
    let valid = browse(
        &identity,
        1,
        BrowseOperation::BrowseList { path: path("/srv") },
    );
    let json = serde_json::to_value(&valid).unwrap();
    let mut extra_envelope = json.clone();
    extra_envelope["tab"] = serde_json::json!(1);
    let mut extra_identity = json.clone();
    extra_identity["identity"]["generation"] = serde_json::json!(1);
    let mut extra_data = json.clone();
    extra_data["operation"]["data"]["recursive"] = serde_json::json!(true);
    let mut relative = json.clone();
    relative["operation"]["data"]["path"] = serde_json::json!(b"srv".to_vec());
    let mut traversal = json.clone();
    traversal["operation"]["data"]["path"] = serde_json::json!(b"/srv/../etc".to_vec());
    let mut zero_epoch = json.clone();
    zero_epoch["identity"]["epoch"] = serde_json::json!(0);
    let mut zero_request = json.clone();
    zero_request["request_id"] = serde_json::json!(0);
    let mut short_grant = json.clone();
    short_grant["identity"]["grant"] = serde_json::json!([1, 2, 3]);
    let mut other_major = json.clone();
    other_major["major"] = serde_json::json!(2);
    for value in [
        extra_envelope,
        extra_identity,
        extra_data,
        relative,
        traversal,
        zero_epoch,
        zero_request,
        short_grant,
        other_major,
    ] {
        assert!(
            decode_browse(&wire(&serde_json::to_vec(&value).unwrap())).is_err(),
            "accepted {value}"
        );
    }
    let mut checked = serde_json::to_value(browse(
        &identity,
        1,
        BrowseOperation::BrowseChecked {
            requested: path("/srv"),
            result: DirectoryCheck::Directory { path: path("/srv") },
        },
    ))
    .unwrap();
    checked["operation"]["data"]["result"]["Directory"]["contents"] = serde_json::json!("x");
    assert!(decode_browse(&wire(&serde_json::to_vec(&checked).unwrap())).is_err());

    // The server home rides only the open reply, as an absolute path.
    let opened = serde_json::to_value(browse(
        &identity,
        1,
        BrowseOperation::BrowseOpened {
            home: Some(path("/home/user")),
        },
    ))
    .unwrap();
    let mut extra_open = opened.clone();
    extra_open["operation"]["data"]["shell"] = serde_json::json!("/bin/sh");
    let mut relative_home = opened.clone();
    relative_home["operation"]["data"]["home"] = serde_json::json!(b"home/user".to_vec());
    let mut traversing_home = opened.clone();
    traversing_home["operation"]["data"]["home"] = serde_json::json!(b"/home/../etc".to_vec());
    let mut text_home = opened;
    text_home["operation"]["data"]["home"] = serde_json::json!("/home/user");
    for value in [extra_open, relative_home, traversing_home, text_home] {
        assert!(
            decode_browse(&wire(&serde_json::to_vec(&value).unwrap())).is_err(),
            "accepted {value}"
        );
    }
    // A home is never a request: a client cannot supply one.
    assert_eq!(
        browse(&identity, 1, BrowseOperation::BrowseOpened { home: None }).validate_request(),
        Err(ProtocolError::UnexpectedResponse)
    );

    // Oversized frames fail on the length prefix, before allocation.
    let mut decoder = BrowseFrameDecoder::new();
    assert_eq!(
        decoder
            .push(&((MAX_FRAME_BYTES + 1) as u32).to_be_bytes())
            .unwrap_err(),
        ProtocolError::Limit
    );
    assert_eq!(
        BrowseFrameDecoder::new()
            .push(&0u32.to_be_bytes())
            .unwrap_err(),
        ProtocolError::Malformed
    );
    let mut partial = BrowseFrameDecoder::new();
    partial.push(&[0, 0]).unwrap();
    assert!(partial.finish().is_err());

    // Per-frame reply bounds.
    let dir = path("/srv");
    let listed = |entries| {
        browse(
            &identity,
            1,
            BrowseOperation::BrowseListed {
                path: dir.clone(),
                entries,
                truncated: false,
                done: true,
            },
        )
    };
    let too_many = listed_entries((0..=MAX_LIST_BATCH).map(|i| format!("/srv/{i}")));
    assert_eq!(listed(too_many).validate(), Err(ProtocolError::Limit));
    for bad in ["/srv", "/srv/a/b", "/other/a"] {
        assert_eq!(
            listed(listed_entries([bad.to_owned()])).validate(),
            Err(ProtocolError::Path),
            "{bad}"
        );
    }
    assert_eq!(
        listed(listed_entries(["/srv/a".to_owned(), "/srv/a".to_owned()])).validate(),
        Err(ProtocolError::Path)
    );
    let git_over = browse(
        &identity,
        1,
        BrowseOperation::BrowseGitResult {
            path: dir.clone(),
            command: GitRead::Status,
            offset: MAX_GIT_BYTES,
            data: Chunk::new(vec![b'x']).unwrap(),
            done: true,
        },
    );
    assert_eq!(git_over.validate(), Err(ProtocolError::Limit));
    assert!(encode_browse(&git_over).is_err());

    // Aggregate listing bound: the screen-scoped 1000-entry cap.
    assert_eq!(MAX_BROWSE_ENTRIES, 1000);
    let request = browse(
        &identity,
        1,
        BrowseOperation::BrowseList { path: dir.clone() },
    );
    let mut progress = BrowseReplyProgress::new(request.clone()).unwrap();
    let mut next = 0;
    while next < MAX_BROWSE_ENTRIES {
        let end = (next + 8).min(MAX_BROWSE_ENTRIES);
        let entries = listed_entries((next..end).map(|i| format!("/srv/{i}")));
        next = end;
        progress
            .accept(&browse(
                &identity,
                1,
                BrowseOperation::BrowseListed {
                    path: dir.clone(),
                    entries,
                    truncated: false,
                    done: false,
                },
            ))
            .unwrap();
    }
    assert_eq!(progress.entry_count(), MAX_BROWSE_ENTRIES);
    assert_eq!(
        progress
            .accept(&listed(listed_entries(["/srv/extra".to_owned()])))
            .unwrap_err(),
        ProtocolError::Limit
    );
    assert!(progress.finish().is_err(), "an over-cap stream is poisoned");

    // Duplicates across batches are rejected.
    let mut repeated = BrowseReplyProgress::new(request.clone()).unwrap();
    let partial_batch = |done| {
        browse(
            &identity,
            1,
            BrowseOperation::BrowseListed {
                path: dir.clone(),
                entries: listed_entries(["/srv/a".to_owned()]),
                truncated: false,
                done,
            },
        )
    };
    repeated.accept(&partial_batch(false)).unwrap();
    assert_eq!(
        repeated.accept(&partial_batch(true)).unwrap_err(),
        ProtocolError::Path
    );

    // Truncation is final; nothing is accepted after completion.
    let mut truncated = BrowseReplyProgress::new(request).unwrap();
    truncated
        .accept(&browse(
            &identity,
            1,
            BrowseOperation::BrowseListed {
                path: dir.clone(),
                entries: vec![],
                truncated: true,
                done: true,
            },
        ))
        .unwrap();
    assert!(truncated.is_truncated());
    truncated.finish().unwrap();
    assert!(truncated.accept(&listed(vec![])).is_err());

    // Git streams must be contiguous and bounded.
    let git = browse(
        &identity,
        2,
        BrowseOperation::BrowseGitRead {
            path: dir.clone(),
            command: GitRead::Status,
        },
    );
    let mut git_progress = BrowseReplyProgress::new(git).unwrap();
    assert_eq!(
        git_progress
            .accept(&browse(
                &identity,
                2,
                BrowseOperation::BrowseGitResult {
                    path: dir,
                    command: GitRead::Status,
                    offset: 5,
                    data: Chunk::new(vec![b'x']).unwrap(),
                    done: true,
                },
            ))
            .unwrap_err(),
        ProtocolError::Transfer
    );
}

#[test]
fn foreign_browse_grants_epochs_and_request_ids_are_rejected() {
    let identity = browse_identity();
    let request = browse(
        &identity,
        4,
        BrowseOperation::BrowseCheck { path: path("/srv") },
    );
    let reply = |identity: BrowseIdentity, request_id| {
        BrowseEnvelope::new(
            identity,
            request_id,
            BrowseOperation::BrowseChecked {
                requested: path("/srv"),
                result: DirectoryCheck::Missing,
            },
        )
    };
    reply(identity.clone(), 4).validate_reply(&request).unwrap();
    let mut foreign_grant = identity.clone();
    foreign_grant.grant = BrowseGrant::random().unwrap();
    let mut foreign_authority = identity.clone();
    foreign_authority.authority = AuthorityId([8; 16]);
    let mut stale_epoch = identity.clone();
    stale_epoch.epoch = 2;
    for foreign in [foreign_grant, foreign_authority, stale_epoch] {
        assert!(!foreign.matches(&identity));
        assert_eq!(
            reply(foreign, 4).validate_reply(&request),
            Err(ProtocolError::Identity)
        );
    }
    assert_eq!(
        reply(identity.clone(), 5).validate_reply(&request),
        Err(ProtocolError::Identity)
    );
    let mut progress = BrowseReplyProgress::new(request).unwrap();
    let mut stranger = identity.clone();
    stranger.grant = BrowseGrant::random().unwrap();
    assert!(progress.accept(&reply(stranger, 4)).is_err());
    assert!(progress.accept(&reply(identity, 4)).is_err(), "poisoned");
}

#[test]
fn browse_grants_and_tab_grants_never_address_each_other() {
    let identity = browse_identity();
    let browse_frame = encode_browse(&browse(
        &identity,
        1,
        BrowseOperation::BrowseList { path: path("/srv") },
    ))
    .unwrap();
    // A browse frame never decodes as a tab frame.
    assert!(FrameDecoder::new().push(&browse_frame).is_err());
    // A tab frame never decodes as a browse frame, whatever its verb.
    for operation in [
        Operation::List { path: path("/srv") },
        Operation::RootValidate { path: path("/srv") },
        Operation::GitRead {
            command: GitRead::Status,
        },
        Operation::Preview {
            path: path("/srv/a"),
        },
        Operation::Close,
        Operation::Heartbeat,
    ] {
        assert!(decode_browse(&encode(&frame(operation)).unwrap()).is_err());
    }

    // Tab verbs inside a browse envelope and browse verbs inside a tab
    // envelope are unknown operations, not reinterpretations.
    let mut tab_verb = serde_json::to_value(browse(
        &identity,
        1,
        BrowseOperation::BrowseList { path: path("/srv") },
    ))
    .unwrap();
    for op in [
        "List",
        "Preview",
        "Open",
        "TerminalInput",
        "UploadBegin",
        "RootSelect",
    ] {
        tab_verb["operation"]["op"] = serde_json::json!(op);
        assert!(serde_json::from_value::<BrowseEnvelope>(tab_verb.clone()).is_err());
    }
    let mut browse_verb =
        serde_json::to_value(frame(Operation::List { path: path("/srv") })).unwrap();
    for op in [
        "BrowseOpen",
        "BrowseList",
        "BrowseCheck",
        "BrowseGitRead",
        "BrowseClose",
    ] {
        browse_verb["operation"]["op"] = serde_json::json!(op);
        assert!(serde_json::from_value::<WireEnvelope>(browse_verb.clone()).is_err());
    }

    // A tab capability's bytes in the grant field are a different grant.
    let tab_identity = frame(Operation::Close).identity;
    let mut borrowed = serde_json::to_value(&identity).unwrap();
    borrowed["grant"] = serde_json::to_value(&tab_identity.tab).unwrap();
    let borrowed: BrowseIdentity = serde_json::from_value(borrowed).unwrap();
    assert!(!borrowed.matches(&identity));
    // An identity that names a tab is not a browse identity at all, and the
    // reverse.
    assert!(
        serde_json::from_value::<BrowseIdentity>(serde_json::to_value(&tab_identity).unwrap())
            .is_err()
    );
    assert!(serde_json::from_value::<Identity>(serde_json::to_value(&identity).unwrap()).is_err());
}

/// `StartShell` is a request (confined to no filesystem root: it reads
/// nothing) answered by exactly one `ShellStarted`, and its frames round-trip.
#[test]
fn start_shell_is_a_request_answered_by_shell_started_only() {
    let request = frame(Operation::StartShell {
        directory: path("/work/followed"),
        fallback: path("/work"),
    });
    request.validate().unwrap();
    // Outside the current root, like any shell the user starts.
    request
        .validate_request(&path("/elsewhere"), None)
        .expect("a shell start is not a path request");
    let mut decoder = FrameDecoder::new();
    let (_, parsed) = decoder.push(&encode(&request).unwrap()).unwrap();
    assert_eq!(parsed.unwrap().operation, request.operation);

    let mut progress = ReplyProgress::new(request.clone()).unwrap();
    progress
        .accept(&response(
            &request,
            Operation::ShellStarted {
                path: path("/work/followed"),
            },
        ))
        .unwrap();
    progress.finish().unwrap();
    // No other success reply answers it.
    for wrong in [Operation::Ack, Operation::Attached] {
        assert_eq!(
            response(&request, wrong).validate_reply(&request),
            Err(ProtocolError::UnexpectedResponse)
        );
    }
    // A reply is not a request.
    let reply = frame(Operation::ShellStarted {
        path: path("/work"),
    });
    assert_eq!(
        reply.validate_request(&path("/work"), None),
        Err(ProtocolError::UnexpectedResponse)
    );
    assert!(made_remote::supervisor::generation_exempt(
        &request.operation
    ));
    assert!(!request.operation.is_terminal());
    assert_eq!(PROTOCOL_MAJOR, 1, "an added operation, not a new major");
}

/// `WatchDir`, `UnwatchDir` and `GitChanged`: subscription verbs are confined
/// to the root like `Watch`, pair only with their replies, and a pre-existing
/// companion's `Capabilities` (no `watch_dirs`) reads as "not supported".
#[test]
fn directory_watch_verbs_are_confined_paired_and_additive() {
    let root = path("/project");
    let inside = path("/project/src");
    let outside = path("/other");
    for operation in [
        Operation::WatchDir {
            path: inside.clone(),
        },
        Operation::UnwatchDir {
            path: inside.clone(),
        },
    ] {
        frame(operation.clone())
            .validate_request(&root, None)
            .unwrap();
        let escaped = match operation {
            Operation::WatchDir { .. } => Operation::WatchDir {
                path: outside.clone(),
            },
            _ => Operation::UnwatchDir {
                path: outside.clone(),
            },
        };
        assert_eq!(
            frame(escaped).validate_request(&root, None),
            Err(ProtocolError::Path)
        );
        assert!(ReplyProgress::new(frame(operation)).is_ok());
    }
    // A notification is never a request.
    assert_eq!(
        frame(Operation::GitChanged).validate_request(&root, None),
        Err(ProtocolError::UnexpectedResponse)
    );

    let watch = frame(Operation::WatchDir {
        path: inside.clone(),
    });
    let answer = |request: &WireEnvelope, operation| {
        WireEnvelope::new(request.identity.clone(), request.request_id, operation)
    };
    let acknowledged = |operation| answer(&watch, operation).validate_reply(&watch);
    acknowledged(Operation::Changed {
        path: inside.clone(),
    })
    .unwrap();
    assert!(acknowledged(Operation::Changed { path: root.clone() }).is_err());
    assert!(acknowledged(Operation::GitChanged).is_err());
    acknowledged(Operation::Error {
        code: ErrorCode::Limit,
    })
    .unwrap();
    let unwatch = frame(Operation::UnwatchDir { path: inside });
    answer(&unwatch, Operation::Ack)
        .validate_reply(&unwatch)
        .unwrap();
    assert!(answer(&unwatch, Operation::Changed { path: root })
        .validate_reply(&unwatch)
        .is_err());

    // Additive within major 1: an older companion omits the field.
    let old = r#"{"target":"LinuxX64","tmux":false,"opencode":false,"shell_cwd":false,"helper_version":"0.2.0"}"#;
    let capabilities: Capabilities = serde_json::from_str(old).unwrap();
    assert!(!capabilities.watch_dirs);
    assert_eq!(PROTOCOL_MAJOR, 1);
}
