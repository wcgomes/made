# Proposal: Clean TUI exit returns to a shell on SSH tabs

## Intent
A local tab whose TUI exits cleanly gets a shell in the same pane and a plain shell exiting cleanly closes the tab, but a remote tab keeps an exit banner with Reconnect. The user wants closing a TUI to behave the same locally and over SSH.

## Scope
Modifies `Independent Workspace Tabs` in `specs/opencode-session-follow.md` (the exclusion of remote tabs is replaced by a pointer) and `Remote terminal and clipboard parity (SSH-05)` in `specs/ssh-remote-workspaces.md` (the remote rule and three scenarios). No other spec states the remote clean-exit banner; `specs/grok-session-follow.md`, `specs/claude-code-session-follow.md`, `specs/agent-integration-architecture.md` and `specs/made-ui-ux.md` were checked and need no change.

## Non-goals
- No change to the banner's Reconnect action when the SSH transport is lost; SSH-07 keeps it.
- No change to local behavior, to SSH-06 remote OpenCode shell-hosted exit presentation, or to SSH-07 transport loss and reconnect rules.
- No new remote file writes, remote installs or integration changes.

## Approach
The remote tab applies the local rule: on a clean TUI exit it starts the same remote shell `Terminal SSH` starts, in the same pane and server, in the tab's remote explorer root. The mechanism that starts a shell in an existing remote terminal is a discovery item in `tasks.md`.

## Decision index
- **Clean means exit 0 with a connected attachment and no monitor or output failure:** a dropped transport is not a TUI exit, so it keeps the banner with Reconnect.
- **Plain remote shell exiting 0 closes the tab:** matches the local rule and the user's ask for the same behavior local or SSH.
- **Failure to start the follow-up shell keeps the banner:** the user must never be left with a silent dead pane.
- **Unclean end with a connected transport offers `Open shell`, not Reconnect:** the user decided the banner action must give the same follow-up shell as a clean exit; Reconnect would be misleading when nothing disconnected. The label is program-neutral per `Program-neutral wording`.
- **Lost transport keeps Reconnect:** only a real disconnect needs reattachment (SSH-07).
- **No local process in a remote tab:** no banner action may run anything locally.
- **No re-authentication:** the new shell belongs to the tab's existing attachment, so no new prompt appears.
- **Spec stays outcome-only:** how the shell is started (new PTY, supervisor action or host relaunch) is not decided here.
- **Mechanism, decided in implementation (the spec stays outcome-only):** the host asks the companion's supervisor for the shell over the existing attachment. A new additive request `StartShell{directory, fallback}` is answered by `ShellStarted{path}`; the supervisor keeps the screen and scrollback, replaces only the ended child, resets the modes the program left (`LEAVE_PROGRAM_MODES`) in the output stream behind the program's last bytes, starts the shell at the pane's current size in `directory` (else `fallback`, the launch folder, else it refuses), and reports a child's exit once per child and attachment. The shell gets the environment a fresh `Terminal SSH` shell gets (the OpenCode bridge and the Claude Code plugin entry and spool are started or bound at that moment when the launch had none).
- **The capability gate is a dedicated field, not the helper version:** `Capabilities.start_shell` is additive within protocol major 1, so a released helper that predates it omits it (reads false) and is never sent `StartShell`, which it would treat as a fatal decode error. The tab waits for the capabilities reply for 2 s at most; a helper that says no gets "older than the app", one that never answered gets a neutral "not known yet" reason.
- **Bounds:** the explorer root settles for 2 s at most before the request is sent, and the companion has 10 s to answer; both end in the banner with `Open shell` and the reason, never in a silent dead pane. A refused request is never retried by itself.
- **A `StartShell` refused because a child still runs** (`InvalidRequest`) means the tab is out of step: a tab that saw its program end adopts the running shell; one that did not keeps the banner with the reason.
- **Where the tests live:** host and pane behavior in `workbench.rs` and `terminal_view.rs` unit tests (the only scripted remote client and the pane are there), supervisor, client and protocol behavior in `crates/made-remote/tests/`; `ui_model.rs` has no seam to inject a remote client or a terminal exit.
