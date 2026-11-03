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
