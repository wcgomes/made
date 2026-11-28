# Change Spec: ssh-tui-exit-shell

## MODIFIED Requirements

### Requirement: Independent Workspace Tabs

Made MUST let users open an additional workspace folder in a new tab, switch workspace tabs, and close a tab in the same native window. Each tab MUST own its shell/process lifecycle, terminal state, explorer/root generations, shell cwd, bridge/connection state including the connected view directory, OpenCode session map, preview selection/mode, explorer visibility/width and focus state. Switching tabs MUST retain live shell and OpenCode processes and MUST NOT leak cwd, preview content, selection, connection state, shortcuts or focus between tabs. Workspace tab and optional OpenCode session-map presentation are owned by the UI spec; this requirement owns tab lifecycle, ownership, and isolation. Closing a tab MUST terminate/reap only its owned processes and release its watcher, bridge, queues, secrets and bootstrap resources; late results MUST NOT reach a surviving or newly created tab. Closing the last tab MUST return to the folder picker. When a local tab's terminal child exits with status 0 and neither its output nor its monitor failed, Made MUST NOT show an exit banner. A tab whose terminal child is a TUI, whether launched from the `+` menu or at startup, MUST then start the interactive shell Made starts for a plain terminal tab in the same terminal pane, keeping the screen and scrollback the TUI left (for example its last printed lines) above the new prompt, leaving the alternate screen and the input modes the TUI set, and using the pane's current size. The shell MUST start in the tab's current explorer root, after the directory reports the TUI made before it exited are applied, or in the tab's launch folder when that root is no longer a usable directory. The exited TUI's per-terminal integration state and its directory follow MUST be released, so no status, session dots or follow of that TUI attach to the new shell, and the tab presents as a plain terminal tab; state carried by a session channel, such as OpenCode's connection, active session and session map, is released when the integration observes the disconnect, a moment later. A plain shell tab whose shell exits with status 0 MUST close exactly as its close action closes it, including returning to the folder picker when it was the last tab. A non-zero exit, a signal, a monitor or output failure, or output that closes without an exit status MUST keep the tab open with the exit banner that names the program and what ended it, and its Restart action. Remote (SSH) tabs follow the corresponding rule of `Remote terminal and clipboard parity (SSH-05)` in `specs/ssh-remote-workspaces.md`, which applies this same behavior to a remote TUI, offers the exit banner's `Open shell` action for unclean ends while the SSH transport is connected, and keeps the Reconnect action when the transport is lost. No tab persistence across app restart is required.

#### Scenario: Open and switch independent workspaces

- Given workspace A has a running command
- When the user opens folder B in a new tab and switches between A and B
- Then both shells remain alive with separate cwd, explorer, session state and input focus, and each tab restores its own preview mode, selection and explorer layout

#### Scenario: Workspace bridge isolation

- Given two workspaces have independently authenticated OpenCode connections
- When one sends state, focus results, reconnects, or presents another workspace's routing identity with its own credential
- Then only its own accepted state can change, cross-workspace routing is rejected without disclosure, and the other connection and explorer remain unchanged

#### Scenario: Close workspace releases only its resources

- Given two tabs with running processes and pending loads
- When the user closes one tab and a late output, bridge, tree or preview result arrives
- Then only that tab's resources are terminated/released, late results are discarded, and the surviving tab remains live and unchanged

#### Scenario: Close final workspace

- Given a single workspace tab
- When the user closes it
- Then its resources are released and Made returns to the folder picker without launching another shell or OpenCode

#### Scenario: TUI exits cleanly

- Given a local tab whose terminal child is a TUI whose explorer follows folder F and which prints a last line, and in another run F is removed before the TUI exits
- When the TUI exits with status 0
- Then the same pane keeps that last line above a new interactive shell in F (in the tab's launch folder in the other run) at the pane's size, off the alternate screen, shows no exit banner, presents the plain terminal label and glyph, and keeps no status, session dots or directory follow of the exited TUI once its integrations observe the exit

#### Scenario: Shell exits cleanly

- Given two local tabs with running shells
- When the user runs `exit` in one, and later in the remaining tab
- Then each tab closes exactly as its close action closes it, the other tab stays live, and closing the last tab returns to the folder picker

#### Scenario: Unclean exit keeps the banner

- Given a local shell tab and a local TUI tab
- When the child exits non-zero or by a signal, its monitor or output fails, or its output closes without an exit status
- Then the tab stays open with the exit banner that names the program and what ended it, and its Restart action

#### Scenario: Hidden workspaces remain current and bounded

- Given an inactive workspace produces sustained terminal output and connection changes
- When another tab is used and the inactive tab is later selected
- Then output and connection state have progressed with bounded queues/scrollback, no hidden grid was painted, no input was sent to the inactive tab, and current state is displayed without an unbounded replay

### Requirement: Remote terminal and clipboard parity (SSH-05)

The remote terminal MUST run the server's real shell or the explicitly selected server OpenCode TUI and preserve the existing terminal fidelity: truecolor, alternate screen, resize propagation to the remote PTY, mouse/focus reporting, bracketed text paste and limits/sanitization, Unicode/emoji fallback, IME commit-once, scrollback, selection/copy, and keyboard semantics. Detection/test MUST NOT launch OpenCode; only explicit launch or user execution may do so. Host clipboard text MUST NOT transfer without a paste action. A remote program's OSC 52 copy MUST reach the host clipboard under the same rules as a local terminal (OSC 52 clipboard copy in `specs/opencode-session-follow.md`); OSC 52 queries MUST remain refused, so a remote program cannot read the host clipboard. Made MUST NOT paste a host-private image path as if it existed on the server or claim that forwarding Ctrl+V gives a remote program access to the host clipboard.

For a remote terminal only, both explicit image-paste chords MUST upload the clipboard image as a private PNG and paste the completed remote absolute UTF-8 path through existing text/bracketed-paste semantics; the default chord MUST NOT forward `0x16` and claim native remote clipboard retrieval. The first successful image paste per pane/launch MUST show the notice "Image uploaded to the server; its file path was pasted." Local default/native (`Cmd+V` / `Ctrl+V`) and fallback (`Cmd+Shift+V` / `Ctrl+Shift+V`) behavior MUST remain unchanged. Both remote chords MUST prefer image over coexisting text, preserve the 128 MiB RGBA/16384-pixel limits, reject a PNG exceeding 128 MiB encoded size, and transfer only upon that explicit paste; no clipboard polling/synchronization is permitted. Empty/unreadable/oversized clipboard, disconnected transport or failed upload MUST show that nothing was pasted, and MUST NOT paste text as a substitute, a host path or a partial remote path. An interrupted upload MUST NOT be retried or its path pasted automatically on reconnect; a new explicit paste is required.

The remote private `clipboard-images` directory MUST be `0700`, PNGs `0600`, created without overwrite/symlink following and never in the workspace. Cleanup MUST run on helper start and after each successful upload, remove only app-generated regular `clipboard-*.png` files older than seven days and then oldest files until at most 64 remain, preserve the current image and ignore unrelated/symlink entries. Cleanup failures MUST NOT invalidate a successful paste or remove unrelated files; failed partial uploads MUST be removed when reachable and swept on the next helper start. Successful images MUST survive tab close for the retention policy rather than being deleted while a consumer may use the path. When no helper runs or the server is unreachable, immediate physical deletion is not guaranteed: the next helper start MUST reconcile cleanup, and no completed-deletion claim may be made without confirmation.

When a remote tab's terminal child is a TUI that Made launched from the launch bar (OpenCode, Grok Build or Claude Code) and it exits with status 0 while the tab's SSH attachment stays connected and neither its output nor its monitor failed, Made MUST NOT show an exit banner and MUST apply the local behavior of `Independent Workspace Tabs` in `specs/opencode-session-follow.md` to that tab: the remote interactive shell that `Terminal SSH` starts MUST start on the same server in the same terminal pane, keeping the screen and scrollback the TUI left above the new prompt, leaving the alternate screen and the input modes the TUI set, and using the pane's current size. The shell MUST start in the tab's current remote explorer root, after the directory reports the TUI made before it exited are applied, or in the tab's remote launch folder when that root is no longer a usable directory on the server, and MUST NOT require the user to authenticate or confirm host trust again. The exited TUI's per-terminal integration state and directory follow MUST be released under the same rules as locally, so no status, session dots or follow of that TUI attach to the new shell, and the tab MUST present as a plain remote terminal tab that keeps its SSH host badge, `SSH: <server display name> (<[user@]host[:port]>)` tooltip line and accessible-name part. A remote shell, whether started by `Terminal SSH` or after a TUI exit, that exits with status 0 under the same clean conditions MUST close its tab exactly as the tab's close action closes it, including the same result when it was the last tab. A non-zero exit, a signal, a monitor or output failure, output that closes without an exit status, or a remote shell that cannot be started after a clean TUI exit MUST keep the tab open with the exit banner that names the program and what ended it, and, while the tab's SSH transport is still connected, with an `Open shell` action in place of Reconnect. Activating `Open shell` MUST start the same remote interactive shell, on the same server in the same pane, under the same rules as after a clean TUI exit (the tab's current remote explorer root, else the remote launch folder; the screen kept; no new authentication or trust prompt; the ended program's integration state released; plain remote presentation with the SSH host badge). The action's label MUST be program-neutral under `Program-neutral wording` in `specs/made-ui-ux.md`, and if that shell cannot be started the banner MUST stay with an actionable reason. When the SSH transport is lost or disconnected, the banner MUST keep its Reconnect action under SSH-07 and MUST NOT offer `Open shell`. No exit banner action, and no other path in a remote tab, MAY start a local process or replay terminal input.

#### Scenario: Real remote TUI fidelity

- Given a supported server with real OpenCode v2 and a connected Made workspace
- When the user launches it explicitly, resizes, uses mouse/focus, pastes multiline text, commits IME text, renders Unicode, scrolls/selects/copies and returns from alternate screen
- Then the TUI receives the correct remote terminal events, text paste retains the existing 1 MiB limit/sanitization, no IME character is duplicated, and rendering/input behavior matches the existing local terminal contract

#### Scenario: Remote TUI exits cleanly

- Given a remote tab on server A launched from the launch bar as a TUI whose explorer follows remote folder F and which printed a last line, and in another run F is removed on the server before the TUI exits
- When the TUI exits with status 0 and the SSH attachment stays connected
- Then the same pane keeps that last line above a new remote interactive shell on A in F (in the tab's remote launch folder in the other run) at the pane's size, off the alternate screen, with no exit banner and no new authentication or trust prompt, presents the plain remote terminal label and glyph with the SSH host badge, and keeps no status, session dots or directory follow of the exited TUI

#### Scenario: Remote shell exits cleanly

- Given two remote tabs with running shells, one started by `Terminal SSH` and one started after a clean TUI exit
- When the user runs `exit` in each
- Then each tab closes exactly as its close action closes it, the other tab stays live, and closing the last tab gives the same result as closing it by its close action

#### Scenario: Unclean remote end offers Open shell while connected

- Given a remote shell tab and a remote TUI tab on a server whose SSH transport stays connected, and a variant whose follow-up shell cannot be started after a clean TUI exit
- When the child exits non-zero or by a signal, its monitor or output fails, or its output closes without an exit status
- Then the tab stays open with the exit banner that names the program and what ended it and a program-neutral `Open shell` action instead of Reconnect, and activating it starts the remote interactive shell on the same server in the same pane in the tab's current remote explorer root (the remote launch folder when that root is unusable) with the screen kept, no new authentication or trust prompt, no integration state of the ended program, and the plain remote presentation with the SSH host badge; when the shell cannot be started the banner stays with an actionable reason

#### Scenario: Lost transport keeps Reconnect and nothing runs locally

- Given a remote tab whose child ended, or whose SSH transport is lost or disconnected
- When the banner is shown and the user inspects its actions
- Then a lost or disconnected transport offers Reconnect and no `Open shell`, and in no case does any banner action start a local process in the remote tab or replay terminal input

#### Scenario: Default image paste cannot masquerade as native remote clipboard access

- Given the host clipboard contains an image and a remote TUI is active
- When the user invokes the default chord (`Cmd+V` on macOS or `Ctrl+V` elsewhere), which locally forwards `0x16` for child-native clipboard reading
- Then Made explicitly uses remote PNG upload plus completed server-path paste, shows the uploaded-path notice on the first success, sends no native image `0x16`, and makes no claim of native host clipboard access by the remote process; the local default chord remains native

#### Scenario: Fallback image paste requires a server-valid path and separate authorization

- Given the host clipboard contains an image and a remote TUI is active
- When the user invokes the fallback chord (`Cmd+Shift+V` on macOS or `Ctrl+Shift+V` elsewhere), which locally saves a private PNG and pastes its absolute path
- Then Made uploads a private PNG only for that paste, pastes its completed server-valid path, applies the retention/cleanup policy above, and never represents this as native clipboard retrieval

#### Scenario: Remote image upload failures and retention

- Given image-plus-text, empty/oversized clipboard, an interrupted upload, and a private remote directory with old/excess app PNGs, a symlink, unrelated files and a deletion-denied variant
- When each remote paste chord is exercised and helper startup/upload/close/reconnect cleanup occurs
- Then image priority and size/private-file limits hold, failed/disconnected paste sends no payload or automatic retry, only eligible app PNGs are cleaned by seven-day/64-file policy, the current image survives close, cleanup failure leaves its path usable, and no image/text bytes enter logs

#### Scenario: Remote browser open requires the same user confirmation as local

- Given an attached remote shell or TUI tab whose child runs `$BROWSER <url>`, as Claude Code does for its login URL
- When the remote child asks to open an absolute `http`/`https` URL
- Then the tab shows the same inline URL confirmation card as a local pane, labeled `from <server>`, nothing opens on the host until the user chooses Open, and Copy and Dismiss behave as they do locally
