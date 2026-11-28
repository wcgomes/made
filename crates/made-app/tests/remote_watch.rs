//! Remote change notifications in the workspace model: per-directory
//! subscriptions, pushes after the acknowledgement, the lost-update race of a
//! change that arrives while a read is in flight, and the periodic safety net.
//! Pure controller/fake-transport evidence; not real SSH acceptance.
use made_app::remote_workspace::*;
use made_remote::protocol::{
    AuthorityId, Chunk, DirectoryEntry, EntryKind, ErrorCode, GitRead, Identity, Operation,
    PreviewKind, RemotePath, TabCapability, WireEnvelope, MAX_DIR_WATCHES,
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

fn path(value: &str) -> RemotePath {
    RemotePath::new(value.as_bytes().to_vec()).unwrap()
}

fn answer(request: &WireEnvelope, operation: Operation) -> WireEnvelope {
    WireEnvelope::new(request.identity.clone(), request.request_id, operation)
}

/// A workspace served to a settled snapshot, remembering the id of every
/// subscription it asked for.
struct Tab {
    workspace: RemoteWorkspace,
    watches: HashMap<RemotePath, u64>,
    /// Folders that exist, as the listings report them.
    folders: Vec<RemotePath>,
    /// Whether the root is a Git repository.
    repository: bool,
}

impl Tab {
    fn new(watch_dirs: bool) -> Self {
        Self::with(watch_dirs, true)
    }

    fn with(watch_dirs: bool, repository: bool) -> Self {
        let mut workspace = RemoteWorkspace::new(
            Identity {
                authority: AuthorityId([7; 16]),
                tab: TabCapability::random().unwrap(),
                epoch: 1,
                generation: 1,
            },
            path("/project"),
        )
        .unwrap();
        workspace.set_watch_dirs(watch_dirs);
        workspace.begin_resnapshot().unwrap();
        let mut tab = Self {
            workspace,
            watches: HashMap::new(),
            folders: Vec::new(),
            repository,
        };
        tab.serve();
        tab
    }

    /// Expand a folder that exists.
    fn expand(&mut self, folder: &str) {
        let folder = path(folder);
        if !self.folders.contains(&folder) {
            self.folders.push(folder.clone());
        }
        self.workspace.expand(folder).unwrap();
    }

    /// Answer everything queued, as a healthy companion would. Returns what
    /// was sent. Reads of Git and OpenCode report themselves unavailable.
    fn serve(&mut self) -> Vec<WireEnvelope> {
        let mut sent = Vec::new();
        for _ in 0..1000 {
            // The workbench reconciles on every pump.
            self.workspace.reconcile().unwrap();
            let Some(request) = self.workspace.next_command() else {
                return sent;
            };
            let operation = match &request.operation {
                Operation::RootValidate { path } => Operation::RootResult {
                    requested: path.clone(),
                    path: path.clone(),
                    generation: request.identity.generation,
                },
                Operation::List { path } => Operation::Listed {
                    path: path.clone(),
                    entries: self
                        .folders
                        .iter()
                        .filter(|folder| folder.is_direct_child_of(path))
                        .map(|folder| DirectoryEntry {
                            path: folder.clone(),
                            kind: EntryKind::Directory,
                        })
                        .collect(),
                    truncated: false,
                    done: true,
                },
                Operation::Preview { path } | Operation::Refresh { path } => {
                    Operation::PreviewResult {
                        path: path.clone(),
                        kind: PreviewKind::Text,
                        offset: 0,
                        data: Chunk::new(b"text".to_vec()).unwrap(),
                        done: true,
                    }
                }
                Operation::Watch { path } => Operation::Changed { path: path.clone() },
                Operation::WatchDir { path } => {
                    self.watches.insert(path.clone(), request.request_id);
                    Operation::Changed { path: path.clone() }
                }
                // A repository, so Git is worth asking about.
                Operation::GitRead {
                    command: GitRead::ShowToplevel,
                } if self.repository => Operation::GitResult {
                    command: GitRead::ShowToplevel,
                    offset: 0,
                    data: Chunk::new(b"/project\n".to_vec()).unwrap(),
                    done: true,
                },
                Operation::GitRead { .. } | Operation::GrokState | Operation::SessionState => {
                    Operation::Error {
                        code: ErrorCode::Unavailable,
                    }
                }
                Operation::UnwatchDir { .. } => Operation::Ack,
                // Not answered: no reply, no state change.
                Operation::Cancel { .. } => {
                    sent.push(request);
                    continue;
                }
                other => panic!("unexpected verb {other:?}"),
            };
            self.workspace.accept(answer(&request, operation)).unwrap();
            sent.push(request);
        }
        panic!("unbounded command feedback");
    }

    fn push(&mut self, stream: &RemotePath, operation: Operation) -> Result<(), String> {
        let id = *self.watches.get(stream).expect("subscribed");
        let identity = self.workspace.identity().clone();
        self.workspace
            .accept(WireEnvelope::new(identity, id, operation))
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn changed(&mut self, stream: &str, entry: &str) {
        self.push(&path(stream), Operation::Changed { path: path(entry) })
            .unwrap();
    }

    fn drain(&mut self) -> Vec<WireEnvelope> {
        let mut sent = Vec::new();
        while let Some(request) = self.workspace.next_command() {
            sent.push(request);
        }
        sent
    }
}

fn listed_in(sent: &[WireEnvelope], wanted: &str) -> bool {
    sent.iter()
        .any(|request| matches!(&request.operation, Operation::List { path } if *path == self::path(wanted)))
}

fn git_requested(sent: &[WireEnvelope]) -> usize {
    sent.iter()
        .filter(|request| matches!(request.operation, Operation::GitRead { .. }))
        .count()
}

fn directory(tab: &Tab, wanted: &str) -> Freshness {
    tab.workspace.projection().directories[&path(wanted)].freshness
}

/// Past `GIT_REFRESH_GAP`, so a stale Git refreshes on the next reconcile.
fn past_git_gap() {
    std::thread::sleep(Duration::from_millis(550));
}

#[test]
fn the_root_is_subscribed_per_directory_and_pushes_after_the_ack_are_applied() {
    let mut tab = Tab::new(true);
    assert!(tab.watches.contains_key(&path("/project")));
    // The subscription's acknowledgement completed its request, but its
    // pushes are read: this is the notification stream (previously every
    // push after the acknowledgement was rejected).
    assert_eq!(tab.workspace.pending_count(), 0);
    tab.changed("/project", "/project/new.txt");
    assert_eq!(directory(&tab, "/project"), Freshness::Stale);
    tab.workspace.reconcile().unwrap();
    let sent = tab.drain();
    assert!(listed_in(&sent, "/project"));
}

#[test]
fn only_the_directory_a_push_names_reloads() {
    let mut tab = Tab::new(true);
    for dir in ["/project/a", "/project/b"] {
        tab.expand(dir);
        tab.serve();
    }
    assert!(tab.watches.contains_key(&path("/project/a")));
    assert!(tab.watches.contains_key(&path("/project/b")));

    tab.changed("/project/a", "/project/a/file.txt");
    assert_eq!(directory(&tab, "/project/a"), Freshness::Stale);
    assert_eq!(directory(&tab, "/project/b"), Freshness::Current);
    assert_eq!(directory(&tab, "/project"), Freshness::Current);

    // A push naming the directory itself reloads it and the listing it is in.
    tab.serve();
    tab.changed("/project/b", "/project/b");
    assert_eq!(directory(&tab, "/project/b"), Freshness::Stale);
    assert_eq!(directory(&tab, "/project"), Freshness::Stale);
    assert_eq!(directory(&tab, "/project/a"), Freshness::Current);
}

#[test]
fn a_push_cannot_name_what_its_subscription_does_not_cover() {
    let mut tab = Tab::new(true);
    tab.expand("/project/a");
    tab.serve();
    // Deeper than a non-recursive watch can see, a sibling, and outside.
    for entry in ["/project/a/deeper/x", "/project/b/x", "/outside/x"] {
        assert!(tab
            .push(
                &path("/project/a"),
                Operation::Changed { path: path(entry) }
            )
            .is_err());
    }
    // Git state belongs to the root's subscription alone.
    assert!(tab
        .push(&path("/project/a"), Operation::GitChanged)
        .is_err());
    assert!(tab.push(&path("/project"), Operation::GitChanged).is_ok());
    // Anything else is not a notification.
    assert!(tab.push(&path("/project/a"), Operation::Ack).is_err());
    assert_eq!(directory(&tab, "/project/a"), Freshness::Current);
}

#[test]
fn collapsing_releases_the_subscription_and_late_pushes_are_dropped() {
    let mut tab = Tab::new(true);
    tab.expand("/project/a");
    tab.serve();
    let stale_id = tab.watches[&path("/project/a")];
    tab.workspace.collapse(&path("/project/a"));
    tab.workspace.reconcile().unwrap();
    let sent = tab.drain();
    assert!(sent.iter().any(
        |request| matches!(&request.operation, Operation::UnwatchDir { path: p } if *p == path("/project/a"))
    ));
    assert_eq!(tab.workspace.take_released_streams(), vec![stale_id]);
    assert!(!tab.workspace.is_stream(stale_id));
    // The companion may still have one in flight: it names no stream.
    let identity = tab.workspace.identity().clone();
    assert!(tab
        .workspace
        .accept(WireEnvelope::new(
            identity,
            stale_id,
            Operation::Changed {
                path: path("/project/a/late"),
            },
        ))
        .is_err());
    // Expanding again subscribes again.
    tab.expand("/project/a");
    let again = tab.serve();
    assert!(again.iter().any(
        |request| matches!(&request.operation, Operation::WatchDir { path: p } if *p == path("/project/a"))
    ));
}

#[test]
fn a_refused_subscription_is_not_retried_and_the_table_is_bounded() {
    let mut tab = Tab::new(true);
    tab.expand("/project/refused");
    // The subscription goes ahead of the listing; the companion's watch table
    // is full.
    let watch = tab.workspace.next_command().unwrap();
    let request = tab.workspace.next_command().unwrap();
    assert!(matches!(request.operation, Operation::List { .. }));
    tab.workspace
        .accept(answer(
            &request,
            Operation::Listed {
                path: path("/project/refused"),
                entries: vec![],
                truncated: false,
                done: true,
            },
        ))
        .unwrap();
    assert!(matches!(watch.operation, Operation::WatchDir { .. }));
    tab.workspace
        .accept(answer(
            &watch,
            Operation::Error {
                code: ErrorCode::Limit,
            },
        ))
        .unwrap();
    for _ in 0..3 {
        tab.workspace.reconcile().unwrap();
    }
    assert!(
        tab.drain()
            .iter()
            .all(|request| !matches!(request.operation, Operation::WatchDir { .. })),
        "a refusal is not asked again straight away"
    );
    // ... but it is asked again later, when the table may have room.
    tab.workspace.set_watch_retry(Duration::ZERO);
    tab.workspace.reconcile().unwrap();
    assert!(
        tab.drain().iter().any(|request| matches!(
            &request.operation,
            Operation::WatchDir { path: p } if *p == path("/project/refused")
        )),
        "a refused subscription is retried"
    );
    tab.workspace.set_watch_retry(Duration::from_secs(30));

    // Far more folders than the table holds: never more than
    // `MAX_DIR_WATCHES` (the root included) are subscribed at once, and the
    // newest folders are the ones that keep their subscription.
    let mut live = std::collections::HashSet::new();
    let mut peak = 0;
    let total = MAX_DIR_WATCHES + 8;
    for index in 0..total {
        tab.expand(&format!("/project/d{index:02}"));
        for request in tab.serve() {
            match &request.operation {
                Operation::WatchDir { path } => {
                    live.insert(path.clone());
                }
                Operation::UnwatchDir { path } => {
                    live.remove(path);
                }
                _ => {}
            }
            peak = peak.max(live.len());
        }
    }
    assert!(peak <= MAX_DIR_WATCHES + 1, "{peak}");
    assert!(live.contains(&path(&format!("/project/d{:02}", total - 1))));
    assert!(
        !live.contains(&path("/project/d00")),
        "the oldest was released"
    );
}

/// D3: a change that arrives while a read is in flight must not be hidden by
/// that read's result.
#[test]
fn a_change_during_a_listing_is_not_lost_to_the_result() {
    let mut tab = Tab::new(true);
    tab.workspace.load_directory(path("/project")).unwrap();
    let list = tab.workspace.next_command().unwrap();
    assert_eq!(directory(&tab, "/project"), Freshness::Loading);

    tab.changed("/project", "/project/created.txt");
    // The result may predate the create: it is shown, but not trusted.
    tab.workspace
        .accept(answer(
            &list,
            Operation::Listed {
                path: path("/project"),
                entries: vec![],
                truncated: false,
                done: true,
            },
        ))
        .unwrap();
    assert_eq!(directory(&tab, "/project"), Freshness::Stale);
    tab.workspace.reconcile().unwrap();
    let repeat = tab.workspace.next_command().unwrap();
    assert!(matches!(&repeat.operation, Operation::List { path: p } if *p == path("/project")));
    // Without a change in between the result is trusted as before.
    tab.workspace
        .accept(answer(
            &repeat,
            Operation::Listed {
                path: path("/project"),
                entries: vec![],
                truncated: false,
                done: true,
            },
        ))
        .unwrap();
    assert_eq!(directory(&tab, "/project"), Freshness::Current);
}

#[test]
fn a_change_during_a_preview_is_not_lost_to_the_result() {
    let mut tab = Tab::new(true);
    tab.workspace.open_preview(path("/project/f.txt")).unwrap();
    let preview = tab.workspace.next_command().unwrap();
    tab.changed("/project", "/project/f.txt");
    tab.workspace
        .accept(answer(
            &preview,
            Operation::PreviewResult {
                path: path("/project/f.txt"),
                kind: PreviewKind::Text,
                offset: 0,
                data: Chunk::new(b"old".to_vec()).unwrap(),
                done: true,
            },
        ))
        .unwrap();
    let projection = tab.workspace.projection().preview.as_ref().unwrap();
    assert_eq!(projection.freshness, Freshness::Stale);
    assert_eq!(projection.data, b"old", "shown meanwhile, not trusted");
    tab.workspace.reconcile().unwrap();
    assert!(tab
        .drain()
        .iter()
        .any(|request| matches!(request.operation, Operation::Refresh { .. })));
}

#[test]
fn a_change_during_a_git_refresh_repeats_it() {
    let mut tab = Tab::new(true);
    past_git_gap();
    tab.workspace.refresh_git().unwrap();
    let reads = tab.drain();
    assert_eq!(git_requested(&reads), 8);
    // A notification while the eight reads are out.
    tab.push(&path("/project"), Operation::GitChanged).unwrap();
    for read in &reads {
        let Operation::GitRead { command } = read.operation else {
            continue;
        };
        tab.workspace
            .accept(answer(
                read,
                Operation::GitResult {
                    command,
                    offset: 0,
                    data: Chunk::new(Vec::new()).unwrap(),
                    done: true,
                },
            ))
            .unwrap();
    }
    assert!(tab
        .workspace
        .projection()
        .git
        .iter()
        .all(|git| git.freshness == Freshness::Stale));
    past_git_gap();
    tab.workspace.reconcile().unwrap();
    assert_eq!(git_requested(&tab.drain()), 8, "Git is read again");
}

#[test]
fn git_refreshes_are_rate_limited_under_a_burst() {
    let mut tab = Tab::new(true);
    past_git_gap();
    tab.workspace.refresh_git().unwrap();
    let reads = tab.drain();
    for read in &reads {
        let Operation::GitRead { command } = read.operation else {
            continue;
        };
        tab.workspace
            .accept(answer(
                read,
                Operation::Error {
                    code: ErrorCode::Unavailable,
                },
            ))
            .unwrap_or_else(|error| panic!("{command:?}: {error}"));
    }
    // Immediately after a refresh a notification only marks Git stale.
    tab.push(&path("/project"), Operation::GitChanged).unwrap();
    tab.workspace.reconcile().unwrap();
    assert_eq!(git_requested(&tab.drain()), 0);
    past_git_gap();
    tab.workspace.reconcile().unwrap();
    assert_eq!(git_requested(&tab.drain()), 8);
}

#[test]
fn the_safety_net_reloads_uncovered_directories_every_few_seconds() {
    // A companion without per-directory subscriptions: nothing is covered
    // except what the root-only watch reports.
    let mut tab = Tab::new(false);
    tab.expand("/project/a");
    tab.serve();
    let start = Instant::now();
    tab.workspace.refresh_safety_net(start);
    tab.workspace
        .refresh_safety_net(start + Duration::from_secs(2));
    tab.workspace.reconcile().unwrap();
    assert!(tab.drain().is_empty(), "not before the interval");

    tab.workspace
        .refresh_safety_net(start + Duration::from_secs(3));
    tab.workspace.reconcile().unwrap();
    let sent = tab.drain();
    assert!(listed_in(&sent, "/project") && listed_in(&sent, "/project/a"));

    // While those reads are out the next pass starts no duplicate.
    tab.workspace
        .refresh_safety_net(start + Duration::from_secs(6));
    tab.workspace.reconcile().unwrap();
    assert!(tab.drain().is_empty());
    for request in &sent {
        let Operation::List { path } = &request.operation else {
            continue;
        };
        // The root still lists the expanded folder.
        let entries = if *path == self::path("/project") {
            vec![made_remote::protocol::DirectoryEntry {
                path: self::path("/project/a"),
                kind: made_remote::protocol::EntryKind::Directory,
            }]
        } else {
            vec![]
        };
        tab.workspace
            .accept(answer(
                request,
                Operation::Listed {
                    path: path.clone(),
                    entries,
                    truncated: false,
                    done: true,
                },
            ))
            .unwrap();
    }
    assert_eq!(directory(&tab, "/project/a"), Freshness::Current);
}

#[test]
fn covered_directories_and_non_repositories_cost_the_safety_net_almost_nothing() {
    let mut tab = Tab::new(true);
    tab.expand("/project/a");
    tab.serve();
    let start = Instant::now();
    tab.workspace.refresh_safety_net(start);
    for seconds in [3, 6, 9, 12] {
        tab.workspace
            .refresh_safety_net(start + Duration::from_secs(seconds));
        tab.workspace.reconcile().unwrap();
        let sent = tab.drain();
        assert!(sent.is_empty(), "covered at {seconds}s: {sent:?}");
    }
    // No repository was seen, so no Git read either; the slow pass reloads the
    // covered directories as a last resort.
    tab.workspace
        .refresh_safety_net(start + Duration::from_secs(15));
    tab.workspace.reconcile().unwrap();
    let sent = tab.drain();
    assert_eq!(git_requested(&sent), 0);
    assert!(listed_in(&sent, "/project") && listed_in(&sent, "/project/a"));
}

#[test]
fn a_repository_is_refreshed_by_the_safety_net_for_changes_no_subscription_sees() {
    let mut tab = Tab::new(true);
    past_git_gap();
    tab.workspace.refresh_git().unwrap();
    for read in tab.drain() {
        let Operation::GitRead { command } = read.operation else {
            continue;
        };
        let data: &[u8] = if command == GitRead::ShowToplevel {
            b"/project\n"
        } else {
            b""
        };
        tab.workspace
            .accept(answer(
                &read,
                Operation::GitResult {
                    command,
                    offset: 0,
                    data: Chunk::new(data.to_vec()).unwrap(),
                    done: true,
                },
            ))
            .unwrap();
    }
    assert!(tab.workspace.projection().git_root_known);
    let start = Instant::now();
    tab.workspace.refresh_safety_net(start);
    tab.workspace
        .refresh_safety_net(start + Duration::from_secs(3));
    past_git_gap();
    tab.workspace.reconcile().unwrap();
    assert_eq!(git_requested(&tab.drain()), 8);
}

#[test]
fn a_root_change_forgets_every_subscription() {
    let mut tab = Tab::new(true);
    tab.expand("/project/a");
    tab.serve();
    let ids: Vec<u64> = tab.watches.values().copied().collect();
    tab.workspace.begin_resnapshot().unwrap();
    for id in ids {
        assert!(!tab.workspace.is_stream(id));
    }
    assert!(!tab.workspace.take_released_streams().is_empty());
}

/// M1: a read made before the subscription was requested may predate a
/// change, so the acknowledgement of a subscription reloads its folder once.
/// (The late `Capabilities` of a companion is how a folder gets read first.)
#[test]
fn a_subscription_acknowledgement_reloads_what_was_read_before_it() {
    let mut tab = Tab::new(false);
    tab.expand("/project/a");
    tab.serve();
    assert_eq!(directory(&tab, "/project/a"), Freshness::Current);
    // The companion turns out to subscribe per directory; the folder, read
    // already, is now subscribed.
    tab.workspace.set_watch_dirs(true);
    tab.workspace.reconcile().unwrap();
    let watch = tab
        .drain()
        .into_iter()
        .find(|request| {
            matches!(&request.operation, Operation::WatchDir { path: p } if *p == path("/project/a"))
        })
        .expect("the folder is subscribed");
    tab.watches.insert(path("/project/a"), watch.request_id);
    tab.workspace
        .accept(answer(
            &watch,
            Operation::Changed {
                path: path("/project/a"),
            },
        ))
        .unwrap();
    assert_eq!(directory(&tab, "/project/a"), Freshness::Stale);
    tab.workspace.reconcile().unwrap();
    assert!(listed_in(&tab.drain(), "/project/a"), "read again once");
}

/// R2-2: a folder is subscribed before it is listed, so the listing is read
/// with the watch already requested and the acknowledgement needs no second
/// read.
#[test]
fn a_folder_expanded_with_its_subscription_is_listed_once() {
    let mut tab = Tab::new(true);
    tab.expand("/project/a");
    let sent = tab.serve();
    let operations: Vec<_> = sent
        .iter()
        .map(|request| match &request.operation {
            Operation::WatchDir { path: p } if *p == path("/project/a") => "watch",
            Operation::List { path: p } if *p == path("/project/a") => "list",
            _ => "other",
        })
        .filter(|name| *name != "other")
        .collect();
    assert_eq!(
        operations,
        ["watch", "list"],
        "subscribed first, listed once"
    );
    assert_eq!(directory(&tab, "/project/a"), Freshness::Current);
}

/// R2-3: directory reloads leave room for the Git refresh instead of taking
/// every request slot.
#[test]
fn reloading_many_folders_does_not_starve_git() {
    let mut tab = Tab::new(true);
    let folders: Vec<String> = (0..10).map(|n| format!("/project/s{n}")).collect();
    for folder in &folders {
        tab.expand(folder);
        tab.serve();
    }
    past_git_gap();
    // Every folder and Git go stale together.
    for folder in &folders {
        tab.changed(folder, &format!("{folder}/x"));
    }
    tab.workspace.reconcile().unwrap();
    let first = tab.drain();
    assert_eq!(
        git_requested(&first),
        8,
        "Git's eight reads fit beside the folder reloads"
    );
    assert!(tab.workspace.pending_count() <= IN_FLIGHT_BUDGET);
}

/// The first Git read is still deciding whether the root is a repository when
/// a change arrives: the change is not forgotten, and Git is read again once
/// it is known to be one.
#[test]
fn a_change_during_the_first_git_read_is_not_lost() {
    let mut workspace = RemoteWorkspace::new(
        Identity {
            authority: AuthorityId([9; 16]),
            tab: TabCapability::random().unwrap(),
            epoch: 1,
            generation: 1,
        },
        path("/project"),
    )
    .unwrap();
    workspace.set_watch_dirs(true);
    workspace.begin_resnapshot().unwrap();
    let validate = workspace.next_command().unwrap();
    workspace
        .accept(answer(
            &validate,
            Operation::RootResult {
                requested: path("/project"),
                path: path("/project"),
                generation: 1,
            },
        ))
        .unwrap();
    let mut reads = Vec::new();
    let mut watch = None;
    while let Some(request) = workspace.next_command() {
        match &request.operation {
            Operation::WatchDir { .. } => watch = Some(request),
            Operation::GitRead { .. } => reads.push(request),
            Operation::List { path } => {
                workspace
                    .accept(answer(
                        &request,
                        Operation::Listed {
                            path: path.clone(),
                            entries: vec![],
                            truncated: false,
                            done: true,
                        },
                    ))
                    .unwrap();
            }
            _ => {}
        }
    }
    let watch = watch.expect("the root is subscribed");
    workspace
        .accept(answer(
            &watch,
            Operation::Changed {
                path: path("/project"),
            },
        ))
        .unwrap();
    assert_eq!(git_requested(&reads), 8);
    assert!(!workspace.projection().git_root_known);

    // A change arrives while the first Git read is out.
    let identity = workspace.identity().clone();
    workspace
        .accept(WireEnvelope::new(
            identity,
            watch.request_id,
            Operation::Changed {
                path: path("/project/x"),
            },
        ))
        .unwrap();
    for read in &reads {
        let Operation::GitRead { command } = read.operation else {
            continue;
        };
        workspace
            .accept(answer(
                read,
                Operation::GitResult {
                    command,
                    offset: 0,
                    data: Chunk::new(if command == GitRead::ShowToplevel {
                        b"/project\n".to_vec()
                    } else {
                        Vec::new()
                    })
                    .unwrap(),
                    done: true,
                },
            ))
            .unwrap();
    }
    assert!(workspace.projection().git_root_known);
    assert!(
        workspace
            .projection()
            .git
            .iter()
            .all(|git| git.freshness == Freshness::Stale),
        "the change that arrived meanwhile makes the answer stale"
    );
    past_git_gap();
    workspace.reconcile().unwrap();
    let again: Vec<_> = std::iter::from_fn(|| workspace.next_command()).collect();
    assert_eq!(git_requested(&again), 8);
}

/// Nit: a repository Git will not read is asked about less and less often.
#[test]
fn the_safety_net_backs_off_from_a_repository_git_refuses() {
    let mut tab = Tab::new(true);
    let start = Instant::now();
    tab.workspace.refresh_safety_net(start);
    let mut asked = Vec::new();
    for seconds in [3, 6, 9, 12, 15, 18, 21] {
        tab.workspace
            .refresh_safety_net(start + Duration::from_secs(seconds));
        past_git_gap();
        tab.workspace.reconcile().unwrap();
        let reads = tab.drain();
        if git_requested(&reads) > 0 {
            asked.push(seconds);
            // Git refuses every read of the repository.
            for read in reads {
                if let Operation::GitRead { .. } = read.operation {
                    tab.workspace
                        .accept(answer(
                            &read,
                            Operation::Error {
                                code: ErrorCode::Unavailable,
                            },
                        ))
                        .unwrap();
                }
            }
        }
    }
    // 3 s, then every 6 s, then every 12 s: not every pass.
    assert!(asked.len() <= 4, "{asked:?}");
    assert!(asked.len() >= 2, "{asked:?}");
}

/// M3: the folder of the open preview is subscribed even when it is not an
/// expanded folder, and the preview follows its changes.
#[test]
fn the_previewed_files_folder_is_subscribed_and_its_changes_reload_the_preview() {
    let mut tab = Tab::new(true);
    tab.workspace
        .open_preview(path("/project/hidden/deep/f.txt"))
        .unwrap();
    tab.serve();
    assert!(tab.watches.contains_key(&path("/project/hidden/deep")));
    assert_eq!(
        tab.workspace
            .projection()
            .preview
            .as_ref()
            .unwrap()
            .freshness,
        Freshness::Current
    );
    tab.changed("/project/hidden/deep", "/project/hidden/deep/f.txt");
    assert_eq!(
        tab.workspace
            .projection()
            .preview
            .as_ref()
            .unwrap()
            .freshness,
        Freshness::Stale
    );
    tab.workspace.reconcile().unwrap();
    assert!(tab
        .drain()
        .iter()
        .any(|request| matches!(request.operation, Operation::Refresh { .. })));

    // Closing the preview releases the folder.
    tab.serve();
    tab.workspace.close_preview().unwrap();
    let sent = tab.serve();
    assert!(sent.iter().any(|request| matches!(
        &request.operation,
        Operation::UnwatchDir { path: p } if *p == path("/project/hidden/deep")
    )));
}

/// M3: an unsubscribed preview folder is reread by the safety net.
#[test]
fn the_safety_net_rereads_a_preview_no_subscription_covers() {
    let mut tab = Tab::new(false);
    tab.workspace.open_preview(path("/project/f.txt")).unwrap();
    tab.serve();
    let start = Instant::now();
    tab.workspace.refresh_safety_net(start);
    tab.workspace
        .refresh_safety_net(start + Duration::from_secs(3));
    tab.workspace.reconcile().unwrap();
    assert!(tab
        .drain()
        .iter()
        .any(|request| matches!(request.operation, Operation::Refresh { .. })));
}

/// M4: a mass collapse stays inside the request budget, leaves room for the
/// heartbeat, and every release still goes out.
#[test]
fn a_mass_collapse_releases_every_folder_within_the_request_budget() {
    let mut tab = Tab::new(true);
    let folders: Vec<String> = (0..20).map(|n| format!("/project/m{n:02}")).collect();
    for folder in &folders {
        tab.expand(folder);
        tab.serve();
    }
    assert_eq!(tab.watches.len(), 21);
    for folder in &folders {
        tab.workspace.collapse(&path(folder));
    }
    tab.workspace.reconcile().unwrap();
    // One reconcile: the releases queued are bounded by the budget.
    let budget = IN_FLIGHT_BUDGET;
    assert!(
        tab.workspace.pending_count() <= budget,
        "{} pending",
        tab.workspace.pending_count()
    );
    let mut released = std::collections::HashSet::new();
    for _ in 0..10 {
        let batch = tab.drain();
        assert!(batch.len() <= budget, "{} in one go", batch.len());
        assert!(
            tab.workspace.pending_count() <= budget,
            "{} pending",
            tab.workspace.pending_count()
        );
        if batch.is_empty() {
            break;
        }
        for request in batch {
            if let Operation::UnwatchDir { path } = &request.operation {
                released.insert(path.clone());
            }
            tab.workspace
                .accept(answer(&request, Operation::Ack))
                .unwrap();
        }
        tab.workspace.reconcile().unwrap();
    }
    assert_eq!(released.len(), 20, "every collapsed folder is released");
}

/// m5: a folder under a collapsed one is no longer shown and gives its
/// subscription up with it.
#[test]
fn folders_under_a_collapsed_folder_free_their_subscriptions() {
    let mut tab = Tab::new(true);
    tab.expand("/project/a");
    tab.serve();
    tab.expand("/project/a/b");
    tab.serve();
    assert!(tab.watches.contains_key(&path("/project/a/b")));
    tab.workspace.collapse(&path("/project/a"));
    let sent = tab.serve();
    let released: Vec<_> = sent
        .iter()
        .filter_map(|request| match &request.operation {
            Operation::UnwatchDir { path } => Some(path.clone()),
            _ => None,
        })
        .collect();
    assert!(released.contains(&path("/project/a")));
    assert!(released.contains(&path("/project/a/b")));
}

/// M5: a root that is not a repository costs no Git reads, until one appears.
#[test]
fn a_root_without_a_repository_is_not_asked_about_git() {
    let mut tab = Tab::with(true, false);
    assert!(!tab.workspace.projection().git_root_known);
    past_git_gap();
    tab.changed("/project", "/project/new.txt");
    tab.workspace.reconcile().unwrap();
    assert_eq!(git_requested(&tab.drain()), 0, "no Git read for a change");
    let start = Instant::now();
    tab.workspace.refresh_safety_net(start);
    tab.workspace
        .refresh_safety_net(start + Duration::from_secs(3));
    tab.workspace.reconcile().unwrap();
    assert_eq!(git_requested(&tab.drain()), 0, "nor for the safety net");

    // A `.git` appearing in the root is what starts it.
    tab.changed("/project", "/project/.git");
    tab.workspace.reconcile().unwrap();
    assert_eq!(git_requested(&tab.drain()), 8);
}

/// M5: after a slow Git refresh the next one waits longer than the minimum.
#[test]
fn a_slow_git_refresh_spaces_the_next_one_out() {
    let mut tab = Tab::new(true);
    past_git_gap();
    tab.workspace.refresh_git().unwrap();
    let reads = tab.drain();
    std::thread::sleep(Duration::from_millis(1000));
    for read in &reads {
        let Operation::GitRead { command } = read.operation else {
            continue;
        };
        tab.workspace
            .accept(answer(
                read,
                Operation::GitResult {
                    command,
                    offset: 0,
                    data: Chunk::new(if command == GitRead::ShowToplevel {
                        b"/project\n".to_vec()
                    } else {
                        Vec::new()
                    })
                    .unwrap(),
                    done: true,
                },
            ))
            .unwrap();
    }
    tab.push(&path("/project"), Operation::GitChanged).unwrap();
    // The refresh took about a second: the next one is not due for two, though
    // the minimum gap has long passed.
    std::thread::sleep(Duration::from_millis(700));
    tab.workspace.reconcile().unwrap();
    assert_eq!(git_requested(&tab.drain()), 0);
    std::thread::sleep(Duration::from_millis(600));
    tab.workspace.reconcile().unwrap();
    assert_eq!(git_requested(&tab.drain()), 8);
}

/// m6: a folder whose last read failed is retried by the safety net.
#[test]
fn the_safety_net_retries_a_folder_whose_read_failed() {
    let mut tab = Tab::new(false);
    tab.expand("/project/a");
    let list = tab.workspace.next_command().unwrap();
    tab.workspace
        .accept(answer(
            &list,
            Operation::Error {
                code: ErrorCode::Unavailable,
            },
        ))
        .unwrap();
    assert!(matches!(
        directory(&tab, "/project/a"),
        Freshness::Unavailable(_)
    ));
    let start = Instant::now();
    tab.workspace.refresh_safety_net(start);
    tab.workspace
        .refresh_safety_net(start + Duration::from_secs(3));
    tab.workspace.reconcile().unwrap();
    assert!(listed_in(&tab.drain(), "/project/a"));
}
