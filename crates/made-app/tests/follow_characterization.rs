//! Characterization of the directory-follow branch table of
//! `SessionFollowCoordinator`, written against the code before the
//! integration-module refactor (agent-integration-modules, phase 0).
//!
//! Every row of the "Follow policy" table in the change design is enumerated
//! for both directory-follow policies: the rejecting policy (which tests "had a
//! directory" before the session channel, rejects an unusable directory and
//! retains while outranked) and the ignoring policy (which tests the session
//! channel first, ignores an unusable directory and ignores reports while the
//! session channel holds). The expectations below do not change during the
//! refactor; only the adapter block does.

use std::fs;
use std::path::{Path, PathBuf};

use made_bridge::state::{SessionInfo, SessionSnapshot};

// BEGIN characterization adapter
use made_app::integrations::{CLAUDE_CODE, GROK, OPENCODE};
use made_app::session_follow::{
    RootSource, SessionFollowCoordinator, SessionFollowOutcome, SessionFollowUpdate,
};

type Coordinator = SessionFollowCoordinator;
type Outcome = SessionFollowOutcome;
type Update = SessionFollowUpdate;

/// The Claude Code entry point under the name the phase 0 tests used.
trait ClaudeFollow {
    fn follow_claude_directory(&mut self, directory: Option<&Path>) -> SessionFollowOutcome;
}

impl ClaudeFollow for SessionFollowCoordinator {
    fn follow_claude_directory(&mut self, directory: Option<&Path>) -> SessionFollowOutcome {
        self.follow_directory(CLAUDE_CODE, directory)
    }
}

/// One directory-follow entry point, addressed by its declared policy.
#[derive(Clone, Copy)]
struct Follower {
    follow: fn(&mut Coordinator, Option<&Path>) -> Outcome,
}

/// Rank 200, policy `{ Reject, Retain, Retain }`.
const POLICY_REJECT: Follower = Follower {
    follow: |c, d| c.follow_directory(CLAUDE_CODE, d),
};
/// Rank 300, policy `{ Ignore, Ignore, Retain }`.
const POLICY_IGNORE: Follower = Follower {
    follow: |c, d| c.follow_directory(GROK, d),
};

fn new_coordinator(launch: &Path, terminal: &Path) -> Coordinator {
    SessionFollowCoordinator::new(launch, terminal).unwrap()
}

/// Stable identifier of the effective root owner.
fn source_id(c: &Coordinator) -> &'static str {
    c.effective_root_source().as_str()
}

fn session_channel_owns(c: &Coordinator) -> bool {
    c.effective_root_source() == RootSource::Agent(OPENCODE)
}
// END characterization adapter

const REJECT_DETAIL: &str = "Claude Code directory is not usable";

fn fixture_dir(temp: &tempfile::TempDir, name: &str) -> PathBuf {
    let dir = temp.path().join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("marker.txt"), name).unwrap();
    dir.canonicalize().unwrap()
}

fn info(id: &str, directory: &Path) -> SessionInfo {
    SessionInfo {
        session_id: id.to_owned(),
        title: id.to_owned(),
        active: true,
        busy: false,
        attention: false,
        unread: false,
        prompt_pulse: 0,
        renaming: false,
        directory: Some(directory.to_path_buf()),
        subpath: None,
        project_id: None,
    }
}

/// How the session channel holds the root.
#[derive(Clone, Copy, Debug)]
enum Channel {
    ActiveSession,
    ViewDirectory,
}

const CHANNELS: [Channel; 2] = [Channel::ActiveSession, Channel::ViewDirectory];

/// Root, generation and owner at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    root: PathBuf,
    generation: u64,
    source: &'static str,
}

struct Rig {
    temp: tempfile::TempDir,
    c: Coordinator,
    launch: PathBuf,
    terminal: PathBuf,
    revision: u64,
}

impl Rig {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let launch = fixture_dir(&temp, "launch");
        let terminal = fixture_dir(&temp, "terminal");
        let c = new_coordinator(&launch, &terminal);
        Self {
            temp,
            c,
            launch,
            terminal,
            revision: 0,
        }
    }

    fn dir(&self, name: &str) -> PathBuf {
        fixture_dir(&self.temp, name)
    }

    fn missing(&self) -> PathBuf {
        self.temp.path().join("missing-directory")
    }

    fn follow(&mut self, who: Follower, directory: Option<&Path>) -> Outcome {
        (who.follow)(&mut self.c, directory)
    }

    fn state(&self) -> State {
        State {
            root: self.c.current_root().to_path_buf(),
            generation: u64::from(self.c.generation()),
            source: source_id(&self.c),
        }
    }

    /// Make the session channel hold the root at a fresh directory.
    fn hold_channel(&mut self, channel: Channel) -> PathBuf {
        let dir = self.dir("channel");
        self.revision += 1;
        let snapshot = match channel {
            Channel::ActiveSession => SessionSnapshot {
                revision: self.revision,
                active_session_id: Some("s".to_owned()),
                tabs_enabled: true,
                view_directory: None,
                sessions: vec![info("s", &dir)],
            },
            Channel::ViewDirectory => SessionSnapshot {
                revision: self.revision,
                active_session_id: None,
                tabs_enabled: true,
                view_directory: Some(dir.clone()),
                sessions: Vec::new(),
            },
        };
        let outcome = self.c.apply_snapshot(snapshot);
        assert_eq!(outcome.root, dir, "{channel:?}");
        assert!(session_channel_owns(&self.c), "{channel:?}");
        assert_eq!(source_id(&self.c), "opencode");
        dir
    }

    /// Owner and root after the session channel releases, on a copy.
    fn after_channel_release(&self) -> (PathBuf, &'static str) {
        let mut copy = self.c.clone();
        let outcome = copy.release_active_session();
        (outcome.root, source_id(&copy))
    }

    /// Owner and root after `who` releases its directory, on a copy.
    fn after_dropping(&self, who: Follower) -> (PathBuf, &'static str) {
        let mut copy = self.c.clone();
        let outcome = (who.follow)(&mut copy, None);
        (outcome.root, source_id(&copy))
    }

    /// Outcome changed nothing: same root, same generation, no switch.
    fn assert_kept(&self, outcome: &Outcome, before: &State, label: &str) {
        assert!(!outcome.root_switched, "{label}: switched");
        assert_eq!(outcome.root, before.root, "{label}: root");
        assert_eq!(
            u64::from(outcome.generation),
            before.generation,
            "{label}: generation"
        );
        assert_eq!(&self.state(), before, "{label}: state");
    }

    /// Outcome switched to `root` with a newer generation.
    fn assert_switched(&self, outcome: &Outcome, before: &State, root: &Path, label: &str) {
        assert!(outcome.root_switched, "{label}: not switched");
        assert_eq!(outcome.root, root, "{label}: root");
        assert!(
            u64::from(outcome.generation) > before.generation,
            "{label}: generation not advanced"
        );
        assert_eq!(self.c.current_root(), root, "{label}: current root");
    }
}

// ---------------------------------------------------------------------------
// Rejecting policy
// ---------------------------------------------------------------------------

#[test]
fn reject_policy_session_channel_holds_and_directory_is_usable_retains_it() {
    for channel in CHANNELS {
        let mut rig = Rig::new();
        let held = rig.dir("held");
        let channel_dir = rig.hold_channel(channel);
        let before = rig.state();

        let out = rig.follow(POLICY_REJECT, Some(&held));

        assert_eq!(out.update, Update::ActiveSession, "{channel:?}");
        rig.assert_kept(&out, &before, &format!("{channel:?}"));
        assert_eq!(rig.state().root, channel_dir);
        assert_eq!(
            rig.after_channel_release(),
            (held.clone(), "claude-code"),
            "{channel:?}: the directory is retained"
        );
    }
}

#[test]
fn reject_policy_session_channel_holds_and_directory_is_unusable_rejects() {
    for channel in CHANNELS {
        // With a previously retained directory: it stays.
        let mut rig = Rig::new();
        let previous = rig.dir("previous");
        rig.follow(POLICY_REJECT, Some(&previous));
        rig.hold_channel(channel);
        let before = rig.state();
        let missing = rig.missing();

        let out = rig.follow(POLICY_REJECT, Some(&missing));

        assert_eq!(
            out.update,
            Update::RejectedSnapshot {
                detail: REJECT_DETAIL.to_owned()
            },
            "{channel:?}"
        );
        rig.assert_kept(&out, &before, &format!("{channel:?} with directory"));
        assert_eq!(
            rig.after_channel_release(),
            (previous, "claude-code"),
            "{channel:?}: previous directory unchanged"
        );

        // Without a retained directory: nothing is retained.
        let mut rig = Rig::new();
        rig.hold_channel(channel);
        let before = rig.state();
        let missing = rig.missing();

        let out = rig.follow(POLICY_REJECT, Some(&missing));

        assert_eq!(
            out.update,
            Update::RejectedSnapshot {
                detail: REJECT_DETAIL.to_owned()
            },
            "{channel:?}"
        );
        rig.assert_kept(&out, &before, &format!("{channel:?} without directory"));
        assert_eq!(
            rig.after_channel_release(),
            (rig.terminal.clone(), "shell"),
            "{channel:?}: nothing retained"
        );
    }
}

#[test]
fn reject_policy_session_channel_holds_and_release_with_a_directory_clears_it() {
    for channel in CHANNELS {
        let mut rig = Rig::new();
        let held = rig.dir("held");
        rig.follow(POLICY_REJECT, Some(&held));
        rig.hold_channel(channel);
        let before = rig.state();

        let out = rig.follow(POLICY_REJECT, None);

        assert_eq!(out.update, Update::ActiveSession, "{channel:?}");
        rig.assert_kept(&out, &before, &format!("{channel:?}"));
        assert_eq!(
            rig.after_channel_release(),
            (rig.terminal.clone(), "shell"),
            "{channel:?}: the directory was cleared"
        );
    }
}

#[test]
fn reject_policy_session_channel_holds_and_release_without_a_directory_is_a_no_op() {
    for channel in CHANNELS {
        let mut rig = Rig::new();
        rig.hold_channel(channel);
        let before = rig.state();

        let out = rig.follow(POLICY_REJECT, None);

        // The "had a directory" check precedes the channel check.
        assert_eq!(out.update, Update::NoActiveSession, "{channel:?}");
        rig.assert_kept(&out, &before, &format!("{channel:?}"));
    }
}

#[test]
fn reject_policy_with_no_higher_holder_applies_a_usable_directory() {
    let mut rig = Rig::new();
    let held = rig.dir("held");
    let before = rig.state();

    let out = rig.follow(POLICY_REJECT, Some(&held));

    assert_eq!(out.update, Update::NoActiveSession);
    rig.assert_switched(&out, &before, &held, "first");
    assert_eq!(rig.state().source, "claude-code");

    // The same directory again switches nothing.
    let before = rig.state();
    let out = rig.follow(POLICY_REJECT, Some(&held));
    assert_eq!(out.update, Update::NoActiveSession);
    rig.assert_kept(&out, &before, "same directory");

    // A different directory switches again.
    let other = rig.dir("other");
    let before = rig.state();
    let out = rig.follow(POLICY_REJECT, Some(&other));
    assert_eq!(out.update, Update::NoActiveSession);
    rig.assert_switched(&out, &before, &other, "other directory");
}

#[test]
fn reject_policy_takes_the_root_from_an_open_lower_ranked_holder() {
    let mut rig = Rig::new();
    let lower = rig.dir("lower");
    let held = rig.dir("held");
    rig.follow(POLICY_IGNORE, Some(&lower));
    assert_eq!(rig.state().source, "grok");
    let before = rig.state();

    let out = rig.follow(POLICY_REJECT, Some(&held));

    assert_eq!(out.update, Update::NoActiveSession);
    rig.assert_switched(&out, &before, &held, "over the ignoring holder");
    assert_eq!(rig.state().source, "claude-code");
}

#[test]
fn reject_policy_with_no_higher_holder_rejects_an_unusable_directory() {
    // With a retained directory: it stays and stays current.
    let mut rig = Rig::new();
    let previous = rig.dir("previous");
    rig.follow(POLICY_REJECT, Some(&previous));
    let before = rig.state();
    let missing = rig.missing();

    let out = rig.follow(POLICY_REJECT, Some(&missing));

    assert_eq!(
        out.update,
        Update::RejectedSnapshot {
            detail: REJECT_DETAIL.to_owned()
        }
    );
    rig.assert_kept(&out, &before, "with directory");
    assert_eq!(rig.state().root, previous);
    assert_eq!(rig.state().source, "claude-code");

    // Without one: nothing changes and no owner appears.
    let mut rig = Rig::new();
    let before = rig.state();
    let missing = rig.missing();
    let out = rig.follow(POLICY_REJECT, Some(&missing));
    assert_eq!(
        out.update,
        Update::RejectedSnapshot {
            detail: REJECT_DETAIL.to_owned()
        }
    );
    rig.assert_kept(&out, &before, "without directory");
    assert_eq!(rig.state().root, rig.launch);
    assert_eq!(rig.state().source, "shell");
}

#[test]
fn reject_policy_owner_release_clears_and_falls_back() {
    // Shell fallback.
    let mut rig = Rig::new();
    let held = rig.dir("held");
    rig.follow(POLICY_REJECT, Some(&held));
    let before = rig.state();

    let out = rig.follow(POLICY_REJECT, None);

    assert_eq!(out.update, Update::NoActiveSession);
    let terminal = rig.terminal.clone();
    rig.assert_switched(&out, &before, &terminal, "shell");
    assert_eq!(rig.state().source, "shell");

    // Fallback to the lower-ranked holder first.
    let mut rig = Rig::new();
    let lower = rig.dir("lower");
    let held = rig.dir("held");
    rig.follow(POLICY_IGNORE, Some(&lower));
    rig.follow(POLICY_REJECT, Some(&held));
    let before = rig.state();

    let out = rig.follow(POLICY_REJECT, None);

    assert_eq!(out.update, Update::NoActiveSession);
    rig.assert_switched(&out, &before, &lower, "lower holder");
    assert_eq!(rig.state().source, "grok");
}

#[test]
fn reject_policy_release_with_nothing_held_is_a_no_op() {
    let mut rig = Rig::new();
    let before = rig.state();

    let out = rig.follow(POLICY_REJECT, None);

    assert_eq!(out.update, Update::NoActiveSession);
    rig.assert_kept(&out, &before, "nothing held");
    assert_eq!(rig.state().root, rig.launch);
}

// ---------------------------------------------------------------------------
// Ignoring policy
// ---------------------------------------------------------------------------

#[test]
fn ignore_policy_session_channel_holds_and_directory_is_usable_ignores_it() {
    for channel in CHANNELS {
        let mut rig = Rig::new();
        let held = rig.dir("held");
        let channel_dir = rig.hold_channel(channel);
        let before = rig.state();

        let out = rig.follow(POLICY_IGNORE, Some(&held));

        assert_eq!(out.update, Update::ActiveSession, "{channel:?}");
        rig.assert_kept(&out, &before, &format!("{channel:?}"));
        assert_eq!(rig.state().root, channel_dir);
        assert_eq!(
            rig.after_channel_release(),
            (rig.terminal.clone(), "shell"),
            "{channel:?}: the directory is not retained"
        );
    }
}

#[test]
fn ignore_policy_session_channel_holds_and_directory_is_unusable_ignores_it() {
    for channel in CHANNELS {
        let mut rig = Rig::new();
        rig.hold_channel(channel);
        let before = rig.state();
        let missing = rig.missing();

        let out = rig.follow(POLICY_IGNORE, Some(&missing));

        assert_eq!(out.update, Update::ActiveSession, "{channel:?}");
        rig.assert_kept(&out, &before, &format!("{channel:?}"));
    }
}

#[test]
fn ignore_policy_session_channel_holds_and_release_keeps_the_directory() {
    for channel in CHANNELS {
        let mut rig = Rig::new();
        let held = rig.dir("held");
        rig.follow(POLICY_IGNORE, Some(&held));
        rig.hold_channel(channel);
        let before = rig.state();

        let out = rig.follow(POLICY_IGNORE, None);

        assert_eq!(out.update, Update::ActiveSession, "{channel:?}");
        rig.assert_kept(&out, &before, &format!("{channel:?}"));
        assert_eq!(
            rig.after_channel_release(),
            (held, "grok"),
            "{channel:?}: the directory is kept"
        );
    }
}

#[test]
fn ignore_policy_session_channel_holds_and_release_without_a_directory() {
    for channel in CHANNELS {
        let mut rig = Rig::new();
        rig.hold_channel(channel);
        let before = rig.state();

        let out = rig.follow(POLICY_IGNORE, None);

        // The channel check precedes the "had a directory" check.
        assert_eq!(out.update, Update::ActiveSession, "{channel:?}");
        rig.assert_kept(&out, &before, &format!("{channel:?}"));
    }
}

#[test]
fn ignore_policy_session_channel_check_precedes_the_higher_directory_holder() {
    for channel in CHANNELS {
        let mut rig = Rig::new();
        let reject_dir = rig.dir("reject");
        let ignore_dir = rig.dir("ignore");
        rig.follow(POLICY_REJECT, Some(&reject_dir));
        rig.hold_channel(channel);
        let before = rig.state();

        let out = rig.follow(POLICY_IGNORE, Some(&ignore_dir));

        assert_eq!(out.update, Update::ActiveSession, "{channel:?}");
        rig.assert_kept(&out, &before, &format!("{channel:?}"));
        assert_eq!(
            rig.after_channel_release(),
            (reject_dir.clone(), "claude-code")
        );
        let mut copy = rig.c.clone();
        copy.release_active_session();
        copy.follow_claude_directory(None);
        assert_eq!(source_id(&copy), "shell", "{channel:?}: not retained");
    }
}

#[test]
fn ignore_policy_higher_directory_holder_retains_a_usable_directory() {
    let mut rig = Rig::new();
    let higher = rig.dir("higher");
    let held = rig.dir("held");
    rig.follow(POLICY_REJECT, Some(&higher));
    let before = rig.state();

    let out = rig.follow(POLICY_IGNORE, Some(&held));

    assert_eq!(out.update, Update::NoActiveSession);
    rig.assert_kept(&out, &before, "usable");
    assert_eq!(rig.state().source, "claude-code");
    assert_eq!(
        rig.after_dropping(POLICY_REJECT),
        (held, "grok"),
        "retained for when the higher holder exits"
    );
}

#[test]
fn ignore_policy_higher_directory_holder_ignores_an_unusable_directory() {
    let mut rig = Rig::new();
    let higher = rig.dir("higher");
    let previous = rig.dir("previous");
    rig.follow(POLICY_IGNORE, Some(&previous));
    rig.follow(POLICY_REJECT, Some(&higher));
    let before = rig.state();
    let missing = rig.missing();

    let out = rig.follow(POLICY_IGNORE, Some(&missing));

    assert_eq!(out.update, Update::NoActiveSession);
    rig.assert_kept(&out, &before, "unusable");
    assert_eq!(
        rig.after_dropping(POLICY_REJECT),
        (previous, "grok"),
        "the previous directory is unchanged"
    );
}

#[test]
fn ignore_policy_higher_directory_holder_release_clears_the_directory() {
    let mut rig = Rig::new();
    let higher = rig.dir("higher");
    let held = rig.dir("held");
    rig.follow(POLICY_IGNORE, Some(&held));
    rig.follow(POLICY_REJECT, Some(&higher));
    let before = rig.state();

    let out = rig.follow(POLICY_IGNORE, None);

    assert_eq!(out.update, Update::NoActiveSession);
    rig.assert_kept(&out, &before, "release");
    assert_eq!(
        rig.after_dropping(POLICY_REJECT),
        (rig.terminal.clone(), "shell"),
        "the directory was cleared"
    );
}

#[test]
fn ignore_policy_with_no_higher_holder_applies_a_usable_directory() {
    let mut rig = Rig::new();
    let held = rig.dir("held");
    let before = rig.state();

    let out = rig.follow(POLICY_IGNORE, Some(&held));

    assert_eq!(out.update, Update::NoActiveSession);
    rig.assert_switched(&out, &before, &held, "first");
    assert_eq!(rig.state().source, "grok");

    let before = rig.state();
    let out = rig.follow(POLICY_IGNORE, Some(&held));
    assert_eq!(out.update, Update::NoActiveSession);
    rig.assert_kept(&out, &before, "same directory");
}

#[test]
fn ignore_policy_with_no_higher_holder_ignores_an_unusable_directory() {
    // With a retained directory: it stays and stays current.
    let mut rig = Rig::new();
    let previous = rig.dir("previous");
    rig.follow(POLICY_IGNORE, Some(&previous));
    let before = rig.state();
    let missing = rig.missing();

    let out = rig.follow(POLICY_IGNORE, Some(&missing));

    assert_eq!(out.update, Update::NoActiveSession);
    rig.assert_kept(&out, &before, "with directory");
    assert_eq!(rig.state().root, previous);

    // Without one.
    let mut rig = Rig::new();
    let before = rig.state();
    let missing = rig.missing();
    let out = rig.follow(POLICY_IGNORE, Some(&missing));
    assert_eq!(out.update, Update::NoActiveSession);
    rig.assert_kept(&out, &before, "without directory");
    assert_eq!(rig.state().root, rig.launch);
}

#[test]
fn ignore_policy_owner_release_clears_and_falls_back() {
    let mut rig = Rig::new();
    let held = rig.dir("held");
    rig.follow(POLICY_IGNORE, Some(&held));
    let before = rig.state();

    let out = rig.follow(POLICY_IGNORE, None);

    assert_eq!(out.update, Update::NoActiveSession);
    let terminal = rig.terminal.clone();
    rig.assert_switched(&out, &before, &terminal, "shell");
    assert_eq!(rig.state().source, "shell");
}

#[test]
fn ignore_policy_release_with_nothing_held_is_a_no_op() {
    let mut rig = Rig::new();
    let before = rig.state();

    let out = rig.follow(POLICY_IGNORE, None);

    assert_eq!(out.update, Update::NoActiveSession);
    rig.assert_kept(&out, &before, "nothing held");
    assert_eq!(rig.state().root, rig.launch);
}

// ---------------------------------------------------------------------------
// Release orders with all three owners
// ---------------------------------------------------------------------------

/// Session channel over the rejecting holder over the ignoring holder.
fn all_three(channel: Channel) -> (Rig, PathBuf, PathBuf) {
    let mut rig = Rig::new();
    let lower = rig.dir("lower");
    let higher = rig.dir("higher");
    rig.follow(POLICY_IGNORE, Some(&lower));
    rig.follow(POLICY_REJECT, Some(&higher));
    rig.hold_channel(channel);
    (rig, higher, lower)
}

#[test]
fn release_order_session_channel_then_rejecting_then_ignoring() {
    for channel in CHANNELS {
        let (mut rig, higher, lower) = all_three(channel);
        let before = rig.state();

        let out = rig.c.release_active_session();
        assert_eq!(out.update, Update::NoActiveSession, "{channel:?}");
        rig.assert_switched(&out, &before, &higher, &format!("{channel:?} channel"));
        assert_eq!(rig.state().source, "claude-code");

        let before = rig.state();
        let out = rig.follow(POLICY_REJECT, None);
        assert_eq!(out.update, Update::NoActiveSession, "{channel:?}");
        rig.assert_switched(&out, &before, &lower, &format!("{channel:?} reject"));
        assert_eq!(rig.state().source, "grok");

        let before = rig.state();
        let out = rig.follow(POLICY_IGNORE, None);
        assert_eq!(out.update, Update::NoActiveSession, "{channel:?}");
        let terminal = rig.terminal.clone();
        rig.assert_switched(&out, &before, &terminal, &format!("{channel:?} ignore"));
        assert_eq!(rig.state().source, "shell");
    }
}

#[test]
fn release_order_session_channel_then_ignoring_then_rejecting() {
    for channel in CHANNELS {
        let (mut rig, higher, _lower) = all_three(channel);
        let before = rig.state();

        let out = rig.c.release_active_session();
        assert_eq!(out.update, Update::NoActiveSession, "{channel:?}");
        rig.assert_switched(&out, &before, &higher, &format!("{channel:?} channel"));
        assert_eq!(rig.state().source, "claude-code");

        // The ignoring holder releases while the rejecting holder holds.
        let before = rig.state();
        let out = rig.follow(POLICY_IGNORE, None);
        assert_eq!(out.update, Update::NoActiveSession, "{channel:?}");
        rig.assert_kept(&out, &before, &format!("{channel:?} ignore"));

        // The rejecting holder then falls back to the shell: nothing remains.
        let before = rig.state();
        let out = rig.follow(POLICY_REJECT, None);
        assert_eq!(out.update, Update::NoActiveSession, "{channel:?}");
        let terminal = rig.terminal.clone();
        rig.assert_switched(&out, &before, &terminal, &format!("{channel:?} reject"));
        assert_eq!(rig.state().source, "shell");
    }
}
