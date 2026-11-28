# Tasks: ssh-tui-exit-shell

Readiness: `analyze` must pass before implementation; review runs `verify` against the persisted spec.

## 1. Discovery (no code)
- [x] 1.1 Find where a remote tab's child exit is observed and turned into the exit banner and Reconnect, and how the local clean-exit rule (TUI to shell, shell to close) is implemented (path: crates/made-app/src/workbench.rs, crates/made-app/src/remote_session.rs and neighbors; names to be discovered)
- [x] 1.2 Find how the supervisor owns the remote PTY and child and whether it can start a shell after the child exits within the same attachment, or whether the host must request it (path: crates/made-remote/src/supervisor.rs and the host-supervisor protocol; names to be discovered)
- [x] 1.3 Decide the mechanism and record any protocol or version impact in design.md only if the overflow rule is then met (path: specs/changes/ssh-tui-exit-shell/)

## 2. Tests first
- [x] 2.1 Write failing test: remote TUI exits 0, then a remote shell starts in the same pane in the explorer root with the screen kept, no banner, plain presentation with SSH host badge and no integration follow (path: crates/made-app/src/workbench.rs unit tests, the `RemoteBench` fixture)
- [x] 2.2 Write failing test: the same with the explorer root removed, so the remote launch folder is used (path: crates/made-app/src/workbench.rs and terminal_view.rs unit tests)
- [x] 2.3 Write failing test: remote shell exit 0 closes the tab as its close action does, including the last-tab result (path: crates/made-app/src/workbench.rs and terminal_view.rs unit tests)
- [x] 2.4 (valid for the transport-drop part only; superseded for connected-transport ends by 2.6 to 2.8) Write failing test: transport drop keeps the banner with Reconnect and runs nothing locally (path: crates/made-app/src/workbench.rs and terminal_view.rs unit tests)
- [x] 2.6 Write failing test: non-zero, signal, monitor or output failure, no exit status and shell-start failure keep the banner with a program-neutral `Open shell` action and no Reconnect while the transport is connected (path: crates/made-app/src/workbench.rs and terminal_view.rs unit tests)
- [x] 2.7 Write failing test: activating `Open shell` starts the remote shell in the same pane in the explorer root, else the remote launch folder, with screen kept, no new auth prompt, no ended-program integration state and plain remote presentation; a start failure keeps the banner with an actionable reason (path: crates/made-app/src/workbench.rs and terminal_view.rs unit tests)
- [x] 2.8 Write failing test: transport loss or disconnect offers Reconnect and no `Open shell`, and no banner action starts a local process or replays input in a remote tab (path: crates/made-app/src/workbench.rs and terminal_view.rs unit tests)
- [x] 2.9 Write failing test: the `Open shell` label, tooltip and accessible name name no program (path: crates/made-app/src/workbench.rs and terminal_view.rs unit tests)
- [x] 2.5 Write failing test for the supervisor side of starting the shell after a clean TUI exit (still valid; extend if the `Open shell` action needs a separate companion request) (path: crates/made-remote/tests/; file to be discovered)

## Checkpoint
- [x] confirm 2.1 to 2.5 fail for the expected reason
- [x] confirm 2.6 to 2.9 fail for the expected reason

## 3. Implementation
- [x] 3.1 Implement the clean TUI exit to remote shell transition per 1.3 (path: crates/made-remote/src/supervisor.rs, if needed)
- [x] 3.2 Implement the host side: skip the banner, keep the screen, set the start folder, release integration state, keep host badge presentation (path: crates/made-app/src/workbench.rs, crates/made-app/src/remote_session.rs)
- [x] 3.3 Close the tab on a clean remote shell exit (path: crates/made-app/src/workbench.rs)
- [x] 3.4 Replace Reconnect with a program-neutral `Open shell` banner action while the transport is connected, wired to the same shell start as after a clean exit, and keep Reconnect when it is lost (path: crates/made-app/src/workbench.rs, crates/made-app/src/ui.rs; names to be discovered)

## Checkpoint
- [x] verify Remote TUI exits cleanly
- [x] verify Remote shell exits cleanly
- [x] verify Unclean remote end offers Open shell while connected
- [x] verify Lost transport keeps Reconnect and nothing runs locally
- [x] verify TUI exits cleanly, Shell exits cleanly and Unclean exit keeps the banner still hold locally

## 4. Wiki
- [x] 4.1 Update wiki pages that state remote tabs keep the banner after a clean exit, by a separate wiki-scoped handoff (path: wiki; pages to be discovered)
