# Design: opencode-session-follow-v1

> **Scope:** This design covers product/runtime/integration/security/performance architecture. UI/UX/visual design — Calm Editor tokens, layout, chrome, wordmark, glass/native backdrop, tabs/session-map presentation, picker UX, preview/diagnostics presentation, motion, responsive behavior, and keyboard/accessibility presentation — is owned by `specs/changes/made-ui-ux-v1/design.md`. This design links there instead of duplicating that contract.

## Approach

Made is a native Rust desktop application composed of six parts:

- **App shell:** a single native window with an event loop, GPU surface, workspace-tab collection and immediate-mode UI for the selected workbench, optional session map, explorer, preview and diagnostics; visual composition is described in the UI design.
- **Terminal:** a VT terminal core that maintains screen state, scrollback, selection, and input encoding; a PTY adapter that runs the user's shell; and a terminal renderer optimized for a cell grid.
- **Explorer:** a lazy file tree, a filesystem watcher, a read-only preview, and read-only git metadata for branch and worktree indicators.
- **Bridge:** a loopback-only server that accepts a versioned session snapshot from the OpenCode plugin and sends focus commands back.
- **OpenCode plugin:** an app-owned CLI/TUI plugin loaded into the OpenCode process for that launch only. It observes tabs reactively, resolves the active session's working directory, and connects to the bridge.
- **Platform layer:** a thin trait-based layer for PTY/ConPTY, process lifecycle, paths, shell integration, clipboard, IME, and native dialogs if used.

The app is one source tree. macOS, Linux, and Windows differ only in the platform layer, build artifacts, and validation matrices. There is no webview, no Electron or Tauri shell, no per-OS UI fork, and no separate OpenCode server process in v1.

The terminal core is `libghostty-vt`; the PTY layer is `portable-pty`; the window/input layer is `winit`; the GPU layer is `wgpu`; UI chrome is `egui`; file watching is `notify`; git metadata comes from the `git` CLI (or a Rust git crate if the CLI is unavailable). The terminal grid itself is rendered by a dedicated renderer rather than through `egui` widgets, so input latency and frame pacing do not depend on general-purpose UI layout.

## Significant decisions

### D1: Scope and integration boundary

V1 validates a terminal-first workspace, not a general agent platform or production release. Open a folder and use its shell and following explorer immediately; optionally run `opencode` to enhance follow with active-session/worktree state. Workspace tabs and read-only preview refine this existing undelivered v1, not a new change/version; their visual presentation is owned by `made-ui-ux-v1`.

Consequences:

- Only OpenCode v2 is integrated. Claude Code, Codex, Grok, ACP, remote agents, and other TUIs are excluded from the code paths and the UI.
- Worktree creation, removal, merging, and orchestration remain OpenCode responsibilities. Made reads worktree metadata and displays it; it never mutates worktree state.
- `opencode serve` and attach/proxy workflows are not required. Direct `opencode` launch is the v1 path.
- Signing, notarization, auto-update, installers, multi-window, terminal splits and restart persistence are excluded. Independent workspace tabs are included; no separate terminal-tab hierarchy is added.
- The product is judged by UX validation and cross-platform completeness, not by production release criteria.

### D2: Plugin and loopback bridge

The integration uses the official OpenCode CLI/TUI plugin mechanism, not screen scraping, ACP, terminal-title scraping, or a server proxy.

Why this is the right boundary:

- `context.ui.tabs.list()` is reactive in a Solid computation and returns the open root-session tabs with `sessionID`, `title`, `active`, `busy`, `attention`, and `unread`. This is exactly the session-map data.
- `context.ui.tabs.focus(sessionID)` is the supported way to make a session active. This enables the session-map focus behavior without synthesizing keystrokes.
- `context.data.session.get(sessionID)` returns session info including `location.directory` (the session's working directory, which may be an OpenCode-managed worktree), `subpath`, `projectID`, and `title`. This is exactly the explorer-follow data.
- The plugin runs in the TUI process, can use WebSocket/fetch, can read environment and plugin options, and can observe tab changes reactively. No polling of the TUI screen is needed.
- The plugin is small and app-owned. It translates OpenCode state into a normalized protocol; the app does not depend on OpenCode internals.

The bridge is a loopback WebSocket server owned by each workbench. It isolates plugin and app evolution and gives focus commands an explicit request/response path; session identity is never scraped from terminal output. Standard VT parsing includes validated cwd reports, not prompt/title scraping. The bridge is not a general-purpose API and is not exposed beyond the local machine.

Terminal-title updates remain useful as a fallback diagnostic: OpenCode sets the terminal title to the session title by default, and `OPENCODE_DISABLE_TERMINAL_TITLE` can disable it. Made does not use the title as the primary integration because titles can be duplicated, truncated, disabled, or changed independently of session identity, and they do not carry the worktree directory.

### D3: Plugin loading and config merge

Made launches each terminal shell with its own optional injected inline OpenCode CLI configuration. Composition is local, bounded and off the UI thread; no OpenCode executable probe, process startup, handshake or session is a prerequisite for shell/explorer usability. A preparation failure uses the original environment and records a diagnostic, never blocks normal shell launch. OpenCode itself is only started by the user. The configuration is composed in memory for that launch and is not written to user files.

Composition order:

1. Read the user's existing OpenCode `cli.json` as a read-only base, if present.
2. Read an existing `OPENCODE_CLI_CONFIG_CONTENT` value from the environment, if present, and treat it as an overlay.
3. Apply the Made overlay:
   - add the app-owned plugin entry with options for bridge URL, per-instance token, pane id, protocol version, and app version;
   - force `tabs.mode: "on"` and `tabs.scope: "global"` for v1 so the plugin can see all open sessions and worktrees.

The composed JSON is passed to the child shell through `OPENCODE_CLI_CONFIG_CONTENT`. OpenCode inherits it when the user runs `opencode` in that terminal. The user's `cli.json` is never modified.

Merge rules:

- Existing plugin entries are preserved, including their options and order.
- The Made plugin entry is appended once; duplicate Made entries are removed by identity.
- Made options are namespaced so they cannot collide with user plugin options.
- The two v1 tab settings are forced. If the user configured different values, the Made overlay wins for that launch and diagnostics shows the forced v1 constraint.
- If the user's configuration or an existing inline configuration cannot be parsed, Made does not write a replacement. The shell retains the original environment; a user-launched OpenCode runs without Made's plugin, and Diagnostics reports "configuration could not be composed". The workspace is not labeled degraded.

The plugin path is resolved from an app-owned location: the packaged application's resource directory or the development repository's plugin directory. The path is checked to be inside the app-owned root before injection. The plugin directory is not user configuration and is not writable by the integration at runtime.

A hash of the user configuration file is captured before launch and checked after the session ends in tests and diagnostics. The hash is evidence that the integration did not persist changes; it is not used to rewrite or repair the file.

### D4: Bridge protocol and plugin contract

Transport:

- WebSocket over `ws://127.0.0.1:<ephemeral-port>/made`.
- The server binds `127.0.0.1:0`; the OS chooses the port. There is no wildcard bind and no external interface.
- Each workspace instance has a distinct loopback endpoint, pane id and 256-bit random token, passed only through that shell's environment/plugin options, held in memory and never logged or persisted. The server binds credentials to the expected pane id, rather than accepting any pane id from a client with a valid token. Separate endpoints reuse the existing single-connection bridge implementation and avoid a new multiplexing protocol.
- The plugin sends `hello` as its first message. The bridge requires a valid `hello` within one second and closes the connection with code `4401` on a missing or invalid token, or `4402` on an unsupported protocol major version. No state is sent before authentication.
- The bridge accepts one authenticated plugin connection per workspace pane. A newer authenticated connection for that same pane replaces the older one; a cross-pane hello is rejected before replacement or state disclosure. Each workspace tab has one terminal, not a shared terminal or bridge connection.
- Maximum message size is bounded; malformed JSON closes the connection with a protocol error. Unknown fields are ignored for forward compatibility.

Protocol versioning:

- `protocolVersion` is an integer major version. V1 uses `1`.
- Additive fields are allowed within a major version; consumers ignore unknown fields.
- A major mismatch produces an unsupported status in Made and a close from the bridge.
- `revision` is a monotonically increasing integer per plugin connection. Made ignores state with a revision at or below the last accepted revision. A new `hello` starts a new epoch and resets revision tracking.

Messages:

| Type | Direction | Purpose | Required fields |
|---|---|---|---|
| `hello` | plugin → app | Authenticate and identify the connection | `protocolVersion`, `token`, `paneId`, `pluginVersion`, `opencodeVersion`, `pid`, `cwd` |
| `welcome` | app → plugin | Confirm authentication and version | `protocolVersion`, `appVersion`, `heartbeatIntervalMs` |
| `state` | plugin → app | Full normalized snapshot | `revision`, `activeSessionID`, `tabsEnabled`, `sessions` |
| `focus` | app → plugin | Focus a session in OpenCode | `requestID`, `sessionID` |
| `focus_result` | plugin → app | Report focus outcome | `requestID`, `ok`, `error` (when not ok) |
| `request_state` | app → plugin | Ask for a fresh snapshot | `requestID` |
| `ping` | app → plugin | Liveness check | `nonce` |
| `pong` | plugin → app | Liveness response | `nonce` |
| `error` | plugin → app | Non-fatal plugin error | `code`, `message`, `fatal` |
| `goodbye` | plugin → app | Clean unload | `reason` |

Session snapshot fields:

| Field | Meaning |
|---|---|
| `sessionID` | OpenCode session identifier |
| `title` | Session title as reported by OpenCode; treated as untrusted text |
| `active` | Whether this is the active root-session tab |
| `busy` | Whether the session is busy |
| `attention` | Whether the session needs attention |
| `unread` | Whether the session has unread activity |
| `directory` | Absolute working directory reported by the session, which may be a worktree |
| `subpath` | Optional subpath within the project or worktree |
| `projectID` | OpenCode project identifier |

Plugin behavior:

- Read `context.options` on load: `bridgeUrl`, `token`, `paneId`, `protocolVersion`, `appVersion`.
- Validate that `bridgeUrl` is a loopback WebSocket URL before connecting. Refuse to connect to a non-loopback host.
- Connect, send `hello`, and wait for `welcome` before sending state.
- Observe `context.ui.tabs.list()` in a reactive computation. When the tab set or active tab changes, resolve the active session with `context.data.session.get(sessionID)`.
- Guard asynchronous session lookups with a generation counter so a slow response for an old active tab cannot overwrite a newer selection.
- Coalesce rapid tab changes into a bounded number of snapshots. The goal is to keep the TUI responsive while still delivering the final state quickly.
- Send a full snapshot on every accepted change, on `request_state`, and on reconnect. Full snapshots keep the app simple and make dropped messages recoverable.
- Respond to `ping` with `pong`.
- On `focus`, call `context.ui.tabs.focus(sessionID)` and send `focus_result` with the request id. A missing session or a rejected focus is reported as `ok: false`; it is not thrown into the TUI.
- On unload, send `goodbye` and close.
- Reconnect with bounded exponential backoff and jitter after an unexpected close, until the plugin is unloaded.
- Catch and log every integration error. The plugin never blocks or crashes the OpenCode TUI because the bridge is unavailable.

App behavior:

- On `hello`, validate token, pane id, protocol version, and OpenCode version. If the OpenCode major version is below 2, mark the integration unsupported and close the connection; otherwise send `welcome`.
- On `state`, validate the snapshot, discard stale revisions, update the session map, and drive the session-follow coordinator.
- On `focus_result`, correlate the request id. A timeout or failure produces a non-blocking error in the session map and a `request_state` refresh.
- Send `ping` on the heartbeat interval and mark the connection degraded after a bounded number of missed pongs.
- On `goodbye` or socket close, resume latest known shell cwd (then launch folder), record integration state in Diagnostics, and keep the terminal usable.
- On a new authenticated `hello` for the same pane, replace the old connection and treat the new snapshot as a new epoch.

### D5: Session and workspace model

The app keeps a normalized `SessionSnapshot` and a `WorkspaceRoot`.

`WorkspaceRoot` contains:

- canonical absolute path;
- display name;
- whether the path is a git worktree;
- branch name when available;
- worktree name or path when available;
- metadata status (`known`, `unknown`, `unavailable`).

Active-session resolution:

1. Resolve independently per workbench. If no connected active session directory exists, use latest known shell cwd, then the launch folder. Record root source for Diagnostics; an empty optional session map reports no active session. Initial visibility/presentation is owned by `made-ui-ux-v1`.
2. If the active session has a `directory`, canonicalize it.
3. If the canonical directory differs from the current root, switch the explorer root to that directory.
4. If `subpath` is present, show it as context and optionally reveal it in the tree. The root remains `location.directory`, because the product requirement is to show the session's worktree root, not the main checkout.
5. If the authoritative directory is missing or unreadable, show an unavailable-root message instead of presenting the previous tree as that directory. Keep the terminal usable and reject stale tree/preview results.

Worktree detection is read-only:

- Run `git -C <directory> rev-parse --show-toplevel` to find the worktree root.
- Run `git -C <directory> rev-parse --abbrev-ref HEAD` to find the branch.
- Compare `git rev-parse --git-dir` with `git rev-parse --git-common-dir`. A difference indicates a linked worktree.
- Use `git worktree list --porcelain` or the directory name to obtain a worktree label.
- If git is unavailable, the directory is not a repository, or a command fails, show the directory as a plain folder and record unknown metadata in Diagnostics. Do not block the explorer.

The explorer root remains the canonical `location.directory`. Git metadata may resolve a containing worktree root for classification and labeling, but Made does not redirect the explorer to that parent unless OpenCode reports it as the active session directory. When `location.directory` is itself the worktree root, the indicator names that root and its branch.

OpenCode exposes native worktree/workspace APIs (`context.client.worktree.list/create/remove`). V1 may use the list operation only as a read-only cross-check if needed; it never calls create or remove and does not recreate the worktree manager. The primary source of the active worktree is the active session's `location.directory`.

The app never runs `git worktree add`, `remove`, `prune`, `merge`, or any command that changes worktree state. Worktree changes appear only after OpenCode reports them through the session snapshot.

Root precedence for every transition:

1. Connected OpenCode active session directory, even if unavailable (show unavailable, not unrelated content).
2. Latest validated shell cwd; keep updating this independently while OpenCode wins.
3. Workspace folder supplied by launch argument, chosen in the in-app picker, or opened from the default home workspace.

Disconnect, normal exit, invalid/unsupported integration, or a snapshot with no active directory recomputes this precedence rather than retaining an old OpenCode root. Root-generation changes also clear the previous preview and select the terminal visual mode. Shell reports use the same root validation and generation pipeline, not a second tree mutation path.

### D6: Terminal stack

The terminal is built around a `TerminalCore` boundary:

- feed bytes from the PTY;
- resize the grid;
- expose screen cells, scrollback, cursor state, title, and current working directory;
- encode keyboard, mouse, focus, and paste input;
- support selection and scrollback queries.

`libghostty-vt` provides the VT parsing and screen-state implementation behind this boundary. The boundary exists because Windows support and IME behavior are the highest-risk parts of the terminal stack; if a platform-specific gap appears, a fallback VT implementation can be substituted without changing the explorer, bridge, or UI architecture. That is a risk mitigation, not a v1 feature.

PTY behavior:

- `portable-pty` spawns the user's shell with the workspace folder as cwd and the composed environment.
- The platform layer resolves the default shell, environment, and process group behavior per OS.
- Resize events compute the new grid size from cell metrics and send the new size to the PTY before rendering the reflow.
- Child exit is surfaced as a terminal state; the shell can be relaunched. OpenCode exit is a separate integration event.
- PTY reads are buffered and parsed off the UI thread. The UI consumes damage updates and never blocks on child output.

Input and terminal features:

- Keyboard encoding supports the legacy xterm sequences and the extended keyboard protocols OpenCode needs. The exact protocol set is validated against the real OpenCode TUI in Phase 1 and Phase 4.
- Mouse reporting uses SGR mode and respects the application's mouse mode.
- Bracketed paste is enabled and used for clipboard paste.
- Focus reporting is forwarded when the application enables it.
- Title updates from OSC 0/2 are reflected in the window title.
- OSC 7 is parsed as untrusted structured cwd data; D17 adds safe launch-local emitters rather than assuming every shell already emits it.
- Scrollback is a bounded ring buffer. Selection is grid-based; copy writes to the system clipboard through the platform layer.
- Unicode rendering uses grapheme clustering, width tables, and a system font fallback chain. Emoji and CJK are validated on each OS.
- Basic IME composition is handled through platform IME events, with a composition preview and a single committed string. Advanced IME features are out of scope.

Rendering:

- The terminal grid is rendered with a dedicated glyph atlas and damage-driven draw calls on the GPU surface.
- UI composition, layout, chrome and presentation are owned by `specs/changes/made-ui-ux-v1/design.md`. This design requires that only the selected left surface is painted — terminal grid or preview, never both — and that hidden terminals continue parsing output.
- Frame pacing targets 60 fps. When output is heavy, the renderer may drop intermediate frames but must not drop input or block the event loop.
- The terminal has priority over background tree loading and metadata refreshes.

### D7: Platform layer

The platform layer is a trait implemented by three OS modules. UI code depends on the trait, not on OS-specific APIs.

Responsibilities:

- PTY or ConPTY spawn, resize, termination, and child reaping.
- Shell resolution and environment construction.
- Process lifecycle and cleanup on window close.
- App-owned paths: resource directory, plugin directory, log directory, temp directory, and the current user's home directory for default workspace launch.
- Clipboard read and write.
- IME event normalization and composition state.
- Native file dialogs and menus if used.
- Shell cwd integration where the platform provides it.

Platform notes:

- **macOS:** POSIX PTY, native clipboard, IME through the windowing layer. Builds for arm64 and x64. Developer ID signing and notarization are not v1 requirements.
- **Linux:** POSIX PTY, X11 and Wayland windowing, clipboard and IME differences between display servers. Both X11 and Wayland are supported targets. IME is a validation risk.
- **Windows:** ConPTY, Windows process and path semantics, native clipboard and IME. The supported OpenCode distribution is the native Windows build, not WSL. ConPTY behavior and `libghostty-vt` Windows support are validation risks.

The platform layer does not own product behavior. It exposes capabilities; the app decides how to use them. There is no per-OS UI branch.

### D8: Explorer and preview

Explorer:

- Explorer layout, header, hide/restore animation, divider painting and context menu chrome are owned by `specs/changes/made-ui-ux-v1/design.md`. This design owns the tree model, root generation, watcher, path guard and per-workbench visibility/width state used by the UI.
- Per-workbench visibility and width survive tab switching; restoring reconciles filesystem changes, using a bounded dirty/rescan signal if updates were coalesced while hidden.
- The tree is rooted at the active `WorkspaceRoot`.
- Directories are loaded lazily; the app does not scan a 10,000-file workspace at startup.
- Entries are sorted with directories first, then files, using a stable case-aware order.
- Unreadable entries render as error rows; they do not crash the tree.
- V1 does not implement `.gitignore` filtering or a hidden-file policy beyond what the filesystem reports. The tree lists entries as reported; this keeps the v1 behavior predictable.
- Symlinks are shown, but navigation and preview resolution are constrained by the path guard.
- The watcher uses `notify` and a debounce window. Create, change, delete, and rename events update the tree within the one-second requirement while preserving expansion and selection where possible. Each event carries the root generation, and stale events are discarded.
- Tree loading runs off the UI thread; each load carries the root generation, and stale loads for an old root are cancelled or discarded when the root changes.

Preview:

- Opening a file selects a read-only preview state. Full-left presentation, header/actions, meta, gutter, syntax-highlighting presentation and safe-state styling are owned by `specs/changes/made-ui-ux-v1/design.md`. The terminal remains alive and receives no user input while hidden. Close/return selects the same terminal and restores its focus; no relaunch or output replay is needed. There is no editor widget or save command.
- The preview size limit is 2 MiB. Files above the limit show a clear size-limit message with the file size; they are not rendered.
- Binary detection checks for a NUL byte in the first 8 KiB. A binary file shows a binary-file message; raw bytes are not rendered.
- Invalid UTF-8 text is treated as unsupported text and shows a safe message rather than replacement-character noise.
- Text is rendered read-only. Save shortcuts are ignored and there is no write path in the preview model.
- Syntax highlighting is a SHOULD; its visual presentation and plain-text fallback are owned by the UI design. It is skipped for large or binary files.
- Auto-refresh is a SHOULD. The open file is watched; a debounced change re-reads it. If it becomes binary, too large, deleted, or unreadable, the preview switches to the appropriate safe message.
- Preview reads happen off the UI thread and are cancellable when the selection changes.

Path guard:

- Every path is canonicalized before access.
- The canonical target must be inside the canonical active root, or inside a root explicitly granted by a user action.
- Symlink escapes are rejected by resolving the target, not just by string-prefix comparison.
- V1 has no additional "open file outside root" entry point; the effective allowlist is the active root. The security policy still supports explicit user grants so a later action can be added without weakening the guard.

### D9: Degraded state machine

Integration status is tab-local and inspectable in Diagnostics; it is not overall workspace health:

- `connected`: a valid plugin connection and snapshot.
- `inactive`: ordinary shell use before integration and after normal OpenCode exit; not an app-level degraded state.
- `degraded`/unavailable: attempted integration is unavailable, disabled, timed out or lost; diagnostic-only reason, not a shell failure.
- `unsupported`: the plugin or OpenCode version is incompatible with this app.

Transitions:

| From | Event | To | Behavior |
|---|---|---|---|
| any | workspace start | `inactive` | Shell and explorer usable without waiting for OpenCode |
| `inactive` | valid `hello` and snapshot | `connected` | Apply snapshot; follow active session if present |
| `connected` | socket close, heartbeat timeout, or plugin error | `degraded` | Resume shell cwd; diagnostic reason; terminal usable |
| any | protocol major mismatch | `unsupported` | Reject state; resume shell cwd; terminal usable |
| `connected` | OpenCode exits normally | `inactive` | Resume shell cwd; shell remains usable |
| `degraded` | valid `hello` | `connected` | Restore state and follow active session |
| `unsupported` | valid compatible `hello` | `connected` | Restore state and follow active session |
| `connected` | `tabsEnabled: false` | `degraded` | Diagnostic v1 tabs constraint; resume shell cwd |
| `connected` | active directory unavailable | `connected` with unavailable root | Contextual root error; no unrelated content; terminal usable |
| `connected` | no active session directory | `connected` | Follow shell cwd; no-active-session detail in optional map/Diagnostics |

Rules:

- The app does not infer OpenCode launch/failure from prompt text or a missing connection. In the absence of evidence it reports inactive/not connected in Diagnostics, not a startup failure. Specific known failures keep their reason there; normal exit can retain disconnect history.
- No integration failure blocks terminal input or the explorer.
- Degraded and unsupported details appear in Diagnostics. Routine chrome suppression and contextual feedback presentation are owned by `specs/changes/made-ui-ux-v1/design.md`.
- Latest shell cwd is retained independently across integration epochs and used across disconnects and exits.
- Reconnect is automatic from the plugin side; the app accepts a new authenticated connection and replaces the old one.
- A stale snapshot from a previous connection cannot overwrite a newer epoch.

### D10: Security

Threat model and mitigations:

| Threat | Mitigation |
|---|---|
| Unauthorized local process connects to the bridge | Loopback-only bind, random port, 256-bit per-instance token, authenticated `hello` within one second, no state before auth, close with an auth error |
| Token leaks through logs | Token never logged; diagnostics and logs redact it; error messages use connection ids instead |
| Token leaks to another same-user process | Accepted v1 limitation: the token is passed through the child environment/options, and same-user processes may inspect it. The bridge is local-only and the token rotates per launch. This is documented as a residual risk. |
| User configuration is modified | Read-only composition; no writes to `cli.json`; hash check; degraded launch if composition is unsafe |
| Malicious plugin path | Plugin resolves from an app-owned resource directory; path is checked before injection; user config cannot override the plugin location |
| Path traversal or symlink escape | Canonicalize root and target; require target inside root; resolve symlinks; reject otherwise |
| Malicious file content | Read-only text rendering; no HTML/script execution; binary detection; size limit; no file execution |
| Malicious session title or path | Treat as untrusted text; truncate for display; no markup interpretation; validate paths before filesystem access |
| Bridge denial of service | Handshake timeout, maximum message size, one authenticated connection per pane, bounded queues |
| Network exposure | No non-loopback bind; no remote bridge; no telemetry or off-device state transfer in v1 |
| Plugin over-privilege | Plugin uses only OpenCode context APIs, the bridge, and local logging; it does not execute shell commands or read arbitrary files |

The bridge is a local trust boundary, not a multi-user service. V1 does not attempt to defend against a same-user attacker who can inspect process memory or environment. It does defend against accidental connections, other local users on a multi-user machine, and any non-loopback access.

### D11: Cross-platform delivery

One Cargo workspace builds all targets. The platform crate contains the only OS-specific code. The UI, terminal state, explorer, preview, bridge, and session-follow logic are shared.

CI matrix:

- macOS arm64
- macOS x64
- Linux x64, with X11 and Wayland smoke coverage
- Windows x64

Artifacts are development builds. Signing, notarization, installers, and auto-update are out of scope. GUI/TUI smoke tests may require platform runners; where a runner cannot provide a display or a native OpenCode install, the job records a manual validation step rather than silently skipping the target.

Platform validation includes:

- terminal startup and OpenCode TUI rendering;
- session follow across two worktrees;
- preview and explorer behavior;
- IME composition;
- clipboard;
- PTY/ConPTY resize and child cleanup;
- native OpenCode distribution on Windows.

WSL-hosted OpenCode is out of scope. Windows support means the native OpenCode v2 distribution available on Windows.

### D12: Performance and lightness

Product constraints:

- No webview runtime.
- One native app process plus one shell/process tree per live workspace; OpenCode child processes exist only when user-launched.
- Terminal responsiveness has priority over background work.

Targets (SHOULD, measured on a supported reference machine):

- startup to interactive terminal and browsable explorer: 2 seconds for a 10,000-file workspace, excluding first-run OS prompts;
- reported shell cwd or active-session change to explorer update: 500 ms;
- terminal input-to-render: 50 ms under normal load;
- sustained output: near 60 fps without losing input.

Techniques:

- Lazy tree loading; no full workspace scan at startup.
- Damage-driven terminal rendering and a glyph atlas.
- Bounded channels between PTY reader, parser, and renderer.
- Parser budget per frame; drop intermediate frames, never input.
- Cancellable tree and preview loads when the root or selection changes.
- Bounded scrollback and bounded bridge queues.
- Session-follow updates are applied immediately; tree population can show a loading state without blocking the root switch.
- Metrics instrument startup, cwd/session follow, tab activation, preview roundtrip, explorer hide/resize, input latency and frame time. Measure a 10k tree and at least three tabs including hidden sustained output; record queue/scrollback bounds and zero hidden-grid paint counts. Real hardware GUI measurements are distinct from software-rendered/model evidence; neither untested shells nor platforms inherit a pass. Visual review evidence and its native/software distinction are owned by `specs/changes/made-ui-ux-v1/design.md`.

### D13: Diagnostics and observability

Diagnostics are a first-class v1 surface because the integration crosses process boundaries.

Exposed state:

- integration status (`inactive`, `connected`, unavailable/`degraded`, `unsupported`) for the selected workspace, not app health;
- workspace identity, shell reporting capability/reason, latest known shell cwd, effective root and source;
- reason code and human-readable reason;
- active session id and title;
- active workspace or worktree path;
- worktree metadata status;
- plugin version and protocol version when connected;
- last state revision and last heartbeat;
- recent integration log entries;
- a copyable log location.

Failure reasons include:

- plugin not loaded;
- OpenCode missing or older than v2;
- extension loading disabled;
- configuration composition failed;
- protocol version mismatch;
- authentication rejected;
- bridge connection closed;
- heartbeat timeout;
- session tabs unavailable;
- active directory unavailable;
- worktree metadata unavailable;
- focus request failed or timed out.

Logs are local, structured, and bounded. The token is redacted. Logs identify the pane and connection, not the secret. The diagnostics data is available without a debugger and without reading raw terminal output; panel presentation is owned by `specs/changes/made-ui-ux-v1/design.md`.

### D14: Alternatives considered

| Alternative | Why rejected |
|---|---|
| ACP integration | Not the OpenCode TUI extension path used by v1; would add an adapter and still not expose the open-tab and worktree state the explorer needs. ACP is explicitly out of scope. |
| `opencode serve` plus attach/proxy | Adds server lifecycle, port management, and a second integration surface; the TUI tab state still needs the plugin. Direct launch is sufficient for v1. |
| Screen scraping the TUI | Brittle against rendering changes, cannot reliably identify sessions or worktrees, and cannot implement focus safely. |
| Terminal-title scraping | Titles can be disabled, duplicated, truncated, or unrelated to the worktree; suitable only as a fallback diagnostic, not the primary integration. |
| Tauri, Electron, or webview shell | Violates the no-webview and lightness goals; terminal embedding, IME, and PTY integration are harder to control. |
| Per-OS UI implementations | Duplicates product behavior and creates divergence; the platform layer isolates only OS capabilities. |
| External worktree manager | Duplicates OpenCode's native worktree APIs and risks conflicting state; v1 observes only. |
| Polling filesystem or git for the active session | Cannot reliably determine which OpenCode session is active or which worktree belongs to it; the plugin's reactive tab state is the correct source. |
| Full file editor in v1 | Expands scope into editing, saving, dirty state, LSP, and diff behavior; v1 validates the follow-the-agent workspace first. |

### D15: Risks

| Risk | Impact | Validation or mitigation |
|---|---|---|
| OpenCode v2 plugin/CLI config API changes | Integration fails on a new OpenCode release | Phase 0 gate records the observed API; protocol version gate; adapter layer; unsupported status instead of crash |
| Plugin cannot be loaded from a local path in some distributions | No session follow | Phase 0 validates loading from a normal terminal and from an injected inline config; degraded fallback |
| `location.directory` semantics differ for worktrees or subpaths | Explorer shows the wrong root | Phase 0 validates tab and worktree location changes with real OpenCode sessions |
| Config merge loses a user plugin or writes a file | User trust and security failure | Merge tests with existing plugins and inline config; hash check; no-write design |
| `libghostty-vt` Windows support is incomplete | Windows target blocked | Phase 1 platform spike; `TerminalCore` boundary permits a fallback VT implementation |
| ConPTY resize, mouse, or child cleanup issues | Windows terminal usability | Windows smoke tests in Phase 4; platform contract tests |
| Linux Wayland IME or clipboard differences | Input or copy/paste failure | X11 and Wayland validation; platform-specific IME normalization |
| macOS IME or font fallback differences | Input or rendering failure | macOS smoke tests; font fallback chain validation |
| Terminal renderer cannot sustain 60 fps | Poor TUI UX | Damage tracking, glyph atlas, parser budget, performance tests |
| Focus command is unreliable or unavailable | Session-map focus is degraded | Focus is SHOULD; timeout and non-blocking error path; explorer follow remains primary |
| Bridge token exposure to same-user processes | Local security limitation | Loopback-only, per-launch token, redacted logs, documented residual risk |
| CI runners cannot run GUI/TUI smoke tests | Cross-platform claims unverified | Manual validation record per OS; platform-specific smoke jobs where possible |
| OpenCode native Windows distribution differs from macOS/Linux | Windows parity gap | Windows-specific validation with the native distribution; no WSL fallback |

### D16: V1 constraints and assumptions

- OpenCode v2 only; no other agents and no ACP.
- Direct `opencode` launch; no `opencode serve` or attach requirement.
- `tabs.mode: "on"` and `tabs.scope: "global"` are forced for the launch so all open sessions and worktrees are visible to the plugin. This is a documented v1 constraint, not a permanent product promise.
- The user's OpenCode configuration is read-only. Integration configuration is per-launch and in-memory.
- Worktree lifecycle is owned by OpenCode; Made observes only.
- Windows support means the native OpenCode v2 distribution. WSL is out of scope.
- Preview is read-only; there is no edit, save, dirty state, LSP, or diff path.
- One window with independent workspace tabs and one terminal per workspace. No-arg startup opens the platform-resolved user home; the picker is explicit through the Open folder action or `--picker`, and an explicit path overrides it. Multi-window, splits, a second terminal-tab hierarchy and restart persistence are out of scope.
- The plugin is app-owned and versioned. The bridge protocol is versioned.
- Git metadata is best-effort; a non-git folder or missing git CLI yields a plain folder with unknown metadata, not an error.
- Terminal cwd reporting uses D17 capability-gated launch-local hooks; cmd/unsupported/restricted modes explicitly fall back to latest known cwd or initial folder without claiming guaranteed follow.
- Development builds and CI artifacts are sufficient; signing, notarization, installers, and auto-update are not v1 requirements.

### D17: Shell directory reporting

Existing OSC 7 parsing is a receiver, not proof that any shell emits cwd. Add a launch-description/capability boundary in the platform layer and consume validated cwd changes through the existing terminal core and workbench coordinator. Keep shell-specific startup semantics out of UI and root policy. Reports need bounded URI parsing, local-host checks, correct escaping/decoding and native absolute-path validation, including Windows drive paths; never evaluate report contents or shell-interpolate paths.

| Shell | Launch-local approach to validate before enabling | Capability/fallback |
|---|---|---|
| bash | App-owned startup wrapper, preserving the original interactive rc behavior, composes existing prompt hooks with an OSC 7 cwd emitter; arguments/environment carry paths, not interpolated commands | Enable only for tested interactive modes; preserve existing array/string prompt hooks without duplicate emissions |
| zsh | Isolated per-launch startup directory that chains original startup files and adds a composed chpwd/precmd hook | Preserve original ZDOTDIR/login behavior; unsupported modes fall back rather than skip user startup files |
| fish | Per-process init hook using supported launch options, composing a PWD-change/prompt event emitter without replacing user functions | Validate initial cwd and subsequent changes without duplicate hooks |
| PowerShell/pwsh | Per-process app-owned bootstrap script that composes profile/prompt behavior and emits structured cwd | Respect execution policy; never bypass policy or persist a profile change to get follow |
| cmd | No general safe cwd hook is currently demonstrated | Explicit fallback to observed cwd or launch folder; no DOSKEY prompt parsing, command injection or claim that cmd emits OSC 7 |
| unknown/restricted | No safe emitter installed | Normal terminal, explicit diagnostic capability/reason |

These are implementation candidates, not claims of demonstrated support. Test first with real shells on applicable OSes; a hook is enabled only after startup/profile preservation and config immutability checks pass. Standard supported interactive bash/zsh/fish/PowerShell launches remain required; unsafe modes may fall back. Missing validation is pending evidence, not a silently supported capability. App-owned temporary startup resources are tab-owned and removed on close; never write rc/profile files or global shell/OpenCode config. Shell restart in the same tab recreates that tab's launch plan only.

Alternative A (chosen) uses explicit launch-local hooks and honest capabilities: reliable cwd changes for supported launches, at the cost of shell-specific startup tests. Alternative B merely consumes pre-existing OSC 7: less code but fails the requested out-of-box real-cd workflow. Prompt/title scraping and typing discovery commands into the PTY are rejected because they are unsafe and ambiguous. OS process-cwd probing is not assumed portable and is not required to claim cmd support.

### D18: Workspace ownership and visibility

Evolve `AppModel` from one optional `Workbench` into an ordered collection with stable, never-reused live workspace ids and an active id; reuse `Workbench` as the complete per-tab runtime. A per-workbench bridge/endpoint and expected pane-id binding reuse today's single-client server. A shared multiplexed bridge could reduce listener count but would add routing and replacement/security complexity for little v1 benefit; do not introduce it here.

`AppModel` owns open/select/close and picker routing. `Workbench` owns cwd arbitration, integration epoch, terminal, explorer/watchers, session map, diagnostics and ephemeral layout/left-surface state. The app ticks all live workbenches with bounded fair budgets, but paints only the active workbench and its selected left surface. Output parsing and bridge liveness continue under previews and inactive tabs; hidden grids never lay out/paint merely because output arrives. Existing bounded queues/scrollback must remain bounded, with coalesced state and dirty/rescan reconciliation rather than unbounded pending UI work. Input is routed only to the selected, focused visible surface; preview shortcuts never reach its hidden terminal. Keyboard capture and pending IME/focus transitions must be explicit at tab/surface boundaries.

Tree/preview callbacks carry workspace id plus root/request generation; bridge events carry the owning endpoint and connection epoch. Closing a tab invalidates delivery before releasing its workbench, terminates/reaps only its process tree, stops its listener/watchers, cancels loads and releases credentials/bootstrap files. Audit today's `Workbench::shutdown` call to platform-wide `cleanup`: shared platform state must not make one tab kill another tab's children. The current Windows PTY contract documents immediate-child-only termination; descendant cleanup therefore requires new scoped native implementation/evidence, not an inherited pass. Final close returns to the picker; that explicit lifecycle surface is unchanged by home-default startup. Window close shuts down all workbenches. No persisted tab schema or restart migration is added.

UI presentation — Calm Editor tokens, layout, chrome, wordmark, tabs/session-map, full-left preview, diagnostics card, picker, glass/native backdrop, motion, responsive behavior, widget ids and accessibility presentation — is owned by `specs/changes/made-ui-ux-v1/design.md`. This design requires that the runtime expose stable workspace-scoped preview mode/selection, explorer visibility/width, left-surface state and focus so the UI never shares global state across tabs. Keyboard/IME input routing remains a runtime invariant.

### D19: Home-default launch and picker boundary

Launch behavior:

- No workspace argument resolves the current user's home through the platform boundary, then opens the normal terminal/explorer workspace. Startup never waits on or renders a blocking picker.
- CLI precedence is explicit workspace (`--workspace` or positional) > `--picker` > home. `--picker` and the Open folder action use the existing picker surface; an explicit workspace alongside `--picker` opens normally without showing the picker.
- An unresolvable, missing, or unreadable home opens the existing terminal-fallback state: visible workspace error, usable terminal in a fallback directory, no picker blocking, no crash. The app records the home-derived launch source internally instead of pretending the fallback is an explicit selection.
- Home resolution belongs to `made-platform` so macOS/Linux/Windows semantics stay out of UI/workspace policy. The app maps the resolved path to the existing launch/open path; no new workspace model is introduced.
- The test-only `--smoke-test` harness without a folder keeps its no-workbench behavior; it is not a product startup path and must not start a real shell in the user's home during automated checks.

Picker selection/close UX, icon-only header actions, wordmark, explorer divider, motion and accessibility presentation are owned by `specs/changes/made-ui-ux-v1/design.md`. The core design retains only launch precedence, process boundaries, home resolution, fallback behavior, and the requirement that no process starts before an explicit picker confirmation.

## Flow

```
AppModel: [workspace A] [workspace B] [+ folder]
  active Workbench: selected left surface (terminal OR read-only preview) + explorer state
  each Workbench independently:
    shell -> validated cwd ----------------------------+
    user-launched OpenCode -> plugin -> own bridge -----+-> root arbitration -> explorer
    explorer file open -> guarded async preview -> preview state
    close/return -> same live terminal state
  all live Workbenches: bounded output/state tick; only active surface paints
```

Launch flow:

1. The app accepts a workspace folder from a launch argument; otherwise it opens `--picker` or the platform-resolved home directory. An explicit path overrides `--picker`; no-arg startup does not block on the picker.
2. The app allocates a workspace id/workbench and initializes its shell/explorer in the folder; optional local config/bridge preparation cannot depend on OpenCode probes or readiness and fails open to the original shell environment.
3. The workbench owns its loopback endpoint, unique token and expected pane id and its safe shell launch plan.
4. The shell starts with launch-local hooks and any safely composed OpenCode configuration; no OpenCode is auto-launched.
5. The explorer shows the workspace root, then validated shell cwd. Initial session-map visibility and routine chrome are owned by `specs/changes/made-ui-ux-v1/design.md`.

OpenCode flow:

1. The user runs `opencode` in the Made terminal.
2. OpenCode loads the Made plugin from the injected configuration.
3. The plugin connects to the bridge and authenticates with `hello`.
4. The app sends `welcome`.
5. The plugin observes tabs, resolves the active session, and sends a `state` snapshot.
6. The owning workbench records connected integration, updates its optional session map, and follows the active session directory if present; otherwise shell cwd still wins.

Session switch flow:

1. The user focuses a different OpenCode session tab.
2. The plugin's reactive computation sees the tab change and resolves the new active session.
3. The plugin sends a coalesced `state` snapshot with a new revision.
4. The app discards stale revisions, resolves the new root, reads worktree metadata, and updates the explorer.
5. The previous tree load is cancelled if it is still running.
6. The session map marks the new active session.

Focus-from-Made flow:

1. The user activates a session in the session map.
2. The app sends `focus` with a request id.
3. The plugin calls `context.ui.tabs.focus(sessionID)`.
4. The plugin sends `focus_result`.
5. OpenCode updates the active tab; the plugin sends a new state snapshot; the explorer follows.
6. If focus fails or times out, the app shows a non-blocking error and requests fresh state.

Disconnect flow:

1. The plugin unloads, OpenCode exits, or the socket closes.
2. The owning workbench records disconnected integration (inactive on known normal exit) in Diagnostics; routine chrome presentation is owned by `made-ui-ux-v1`.
3. Root arbitration resumes latest shell cwd, then launch folder; prior-root preview is cleared.
4. The terminal remains usable.
5. The plugin reconnects when OpenCode is available again; the app replaces the old connection and resumes following.

Preview flow:

1. The user selects a file in the explorer.
2. The path guard canonicalizes the root and target and rejects escapes.
3. The preview loader reads metadata and a small prefix.
4. Binary and oversized files produce safe messages.
5. Preview state is selected; text is read-only with optional highlighting. Full-left placement and presentation are owned by `specs/changes/made-ui-ux-v1/design.md`. The hidden terminal continues processing output, not input or painting.
6. The file watcher refreshes the preview on external changes or reports staleness.
7. Close/return selects and focuses the same live terminal; a root change clears preview and invalidates stale loads.

## Architectural boundaries

Crates and modules:

- `crates/made-app`: window, workspace collection/lifecycle, visibility-aware tick/render/input routing, root precedence, session map, explorer, left-surface selection, diagnostics.
- `crates/made-terminal`: terminal core boundary, PTY adapter, input encoding, render model.
- `crates/made-platform`: platform trait and macOS, Linux, and Windows implementations.
- `crates/made-explorer`: tree model, watcher, preview loader, path guard.
- `crates/made-bridge`: protocol, bridge server, config composition, security, state validation.
- `plugins/opencode-made`: app-owned OpenCode plugin.
- Phase 0 gate: outcomes are preserved in [`wiki/records/20261025-phase-0-gate.md`](../../../wiki/records/20261025-phase-0-gate.md); the spike harness was removed after the gate.

Dependency direction:

- `made-app` depends on the domain crates and the platform trait.
- `made-terminal`, `made-explorer`, and `made-bridge` do not depend on `made-app`.
- `made-platform` is the only crate with OS-specific code.
- The OpenCode plugin depends on the OpenCode plugin context, not on Made's Rust crates.
- The bridge depends on no UI code; it exposes a normalized state model.
- The explorer depends on no OpenCode API; it receives a workspace root and a path guard.

Data flow:

- OpenCode state → plugin → bridge → session snapshot → session-follow coordinator → explorer root and session map.
- User session selection → session map → bridge → plugin → OpenCode focus.
- Shell cwd report → validation → owning root coordinator → explorer (unless connected active OpenCode directory wins).
- Explorer file open → path guard → preview loader → owning read-only preview state (presentation owned by the UI design).
- Filesystem watcher → tree/preview refresh.
- Git CLI → worktree metadata → explorer root context (presentation owned by the UI design).

No component outside the plugin calls OpenCode APIs. No component outside the platform layer calls OS-specific PTY, clipboard, or IME APIs. No component outside the bridge binds a network socket. No component outside the explorer reads workspace files for display.

## Migration and rollback

There is no data migration: Made is a new product and v1 has no persisted workspace database. The startup default changes from picker-first to home-first without persisted state; `--picker` and the Open folder action preserve the picker path, and an explicit path remains available as the rollback/compatibility path. Invalid home falls back to the existing terminal-only state rather than creating a new picker flow.

The integration is additive. If the injected configuration is removed, OpenCode runs as the user configured it and Made remains a normal shell-following workspace with diagnostic integration detail. Because Made never writes user shell/OpenCode configuration, disabling integration requires no user-file rollback. Preserve historical checked tasks; new revision work and regression checkpoints remain unchecked until verified. Existing readiness records are historical, not proof that revised behavior is delivered.

If a platform's terminal core cannot meet the requirement, the `TerminalCore` boundary allows a fallback implementation without changing bridge, explorer or root policy. A plugin failure cannot block the terminal-first workflow, but it does not waive the optional OpenCode integration acceptance scenarios required to complete v1. Similarly, unverified shell hooks/platforms remain pending rather than being certified through fallback.

## Risks

See the risk table in D15. The highest-risk items are the Phase 0 bridge gate (plugin loading, tab tracking, worktree location changes, focus, config immutability), `libghostty-vt`/ConPTY on Windows, IME on Wayland and macOS, and CI coverage of GUI/TUI behavior. Each has an explicit validation task and a degraded fallback; none is allowed to become a silent assumption.

## Security considerations

The security model is local-first:

- The bridge is a loopback-only service with a random port and a per-instance token. It is not a remote API.
- Authentication happens before any state is disclosed. Unauthenticated connections are closed and logged.
- The plugin path is app-owned. User configuration can add the plugin for a launch but cannot redirect Made to load arbitrary code as its integration.
- User configuration is read-only. Composition happens in memory and is injected into the child process only.
- The explorer and preview operate under a canonical path guard. Symlink escapes and traversal are rejected.
- The preview is read-only and has no execution path. Binary and oversized files are not rendered.
- Session titles, paths, and error strings from OpenCode are untrusted text. They are displayed as text and never interpreted as markup or commands.
- Logs redact the token. Paths and titles may appear in local logs; they are not sent off-device in v1.
- The residual risk is same-user process inspection of the token through the environment or process memory. V1 accepts this because the bridge is loopback-only and the token rotates per launch; the limitation is documented rather than hidden.
