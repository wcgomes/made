# Change Spec: opencode-session-follow-v1

> **Scope:** This active, undelivered v1 change owns product, runtime, integration, security, and performance behavior. UI/UX/visual behavior — Calm Editor layout and chrome, home/picker presentation and selection, workspace-tab and session-map presentation, full-left preview presentation, explorer layout, diagnostics presentation, wordmark, glass/native backdrop, motion, responsive layout, keyboard focus, and accessibility presentation — is owned by `specs/changes/made-ui-ux-v1/spec.md`. Requirements below cross-reference that contract instead of duplicating it. This change is not archived and no new version is created.

## ADDED Requirements

### Requirement: Workspace Launch

Made MUST accept an explicit workspace folder at launch through a launch argument or equivalent mechanism. When no workspace folder and no `--picker` request is provided, Made MUST open the current user's home directory as the workspace without blocking on a folder picker; this default MUST use the platform's user-home resolution and behave the same way on macOS, Linux, and Windows. The in-app folder picker MUST remain available through an explicit UI action and a `--picker` launch flag. An explicit workspace path (`--workspace <path>` or positional) MUST take precedence over `--picker`. Made MUST open the selected workspace folder and present a usable terminal whose shell starts in that folder and an explorer rooted at that folder. If the home directory cannot be opened, Made MUST show a visible workspace error, keep a usable terminal in a fallback directory, and MUST NOT replace startup with a blocking picker or crash. Shell and explorer startup MUST NOT depend on OpenCode installation, startup, version detection, plugin readiness, or connection. Made MUST NOT automatically launch OpenCode. The test-only smoke harness without a folder MAY retain its no-workbench startup behavior so automated checks do not start a real home shell. Picker rendering, selection, close, keyboard, responsive, and add-workspace presentation are owned by the UI spec; this requirement owns launch resolution, process boundaries, and fallback behavior.

#### Scenario: Workspace chosen via picker or argument

- Given Made is launched with a workspace folder argument, or the user explicitly opens the folder picker and selects a folder
- When the workspace selection is confirmed
- Then the terminal shell starts with that folder as its working directory and the explorer shows that folder as its root

#### Scenario: No workspace provided

- Given Made is launched without a workspace folder or `--picker`
- When the app starts
- Then Made opens the current user's home directory directly with a usable terminal and explorer and does not show a blocking folder picker

#### Scenario: Picker requested explicitly

- Given Made is launched with `--picker` or the user activates the Open folder action
- When the picker is shown and the user confirms a folder
- Then the folder picker is the explicit selection surface, no workspace process starts before confirmation, and the confirmed folder opens in the current or a new workspace tab as appropriate

#### Scenario: Explicit workspace overrides picker

- Given Made is launched with `--picker` together with `--workspace <path>` or a positional path
- When Made starts
- Then it opens the explicit workspace folder and does not show the picker

#### Scenario: Workspace remains usable before OpenCode starts

- Given OpenCode is absent or has never been started
- When the user types in the terminal or browses the explorer
- Then the shell and explorer are immediately usable in the chosen folder, no OpenCode launch/detection/connection is awaited, and normal shell use is not presented as degraded

#### Scenario: Invalid workspace folder

- Given the explicitly selected workspace folder is missing or unreadable
- When Made starts
- Then Made shows a clear workspace error, keeps the terminal usable, and does not crash

#### Scenario: Default home workspace unavailable

- Given Made is launched with no workspace folder or `--picker`, and the current user's home directory is missing, unreadable, or cannot be resolved
- When Made starts
- Then Made shows a visible workspace error, keeps a usable terminal in a fallback directory, does not block on the folder picker, and does not crash

### Requirement: OpenCode Integration Loading

Made MUST observe the active OpenCode v2 session identity and working directory as the user switches sessions, and, while OpenCode reports session tabs enabled, MUST observe the connected sessionless view directory exposed by the OpenCode TUI when no session is active. Made MUST load its integration without persistently modifying the user's OpenCode configuration. Routine integration chrome is owned by the UI spec; this requirement owns integration state and Diagnostics content.

#### Scenario: Plugin loads on first OpenCode launch

- Given OpenCode v2 is installed with session tabs enabled and the user starts it in the Made terminal
- When the OpenCode TUI becomes ready
- Then Made receives the active session identity and working directory, or, when session tabs are enabled and no session is active, the connected view directory, and reports connected state in Diagnostics

#### Scenario: Existing user plugins are preserved

- Given the user's OpenCode configuration contains existing extensions
- When the user starts OpenCode in the Made terminal
- Then those extensions remain available, the Made integration is added for that launch, and the user's configuration file content is unchanged after OpenCode exits

#### Scenario: No OpenCode session yet

- Given OpenCode is running with session tabs enabled but no session tab is open and OpenCode reports a usable view directory
- When Made receives integration state
- Then Diagnostics reports connected with no active session and the explorer follows that view directory, not the latest known shell cwd

#### Scenario: No OpenCode session and no usable view directory

- Given OpenCode is running with session tabs enabled but no session tab is open and no usable view directory is reported
- When Made receives integration state
- Then Diagnostics reports connected with no active session and the explorer follows the latest known shell cwd, falling back to the launch folder only until cwd is known

### Requirement: Shell Directory Follow

For supported launch-local shell integration, Made MUST follow real directory changes in bash, zsh, fish and PowerShell, including paths with spaces and non-ASCII characters, without requiring user configuration edits. Directory reports MUST be validated as local absolute paths, bounded, and treated as untrusted data, never commands. Made MUST NOT scrape prompts or titles, or inject commands into an already-running shell to discover cwd. Hooks MUST be scoped to that shell launch, preserve existing startup/profile behavior and prompt hooks, and leave user shell files byte-for-byte unchanged. If a shell, launch mode or policy cannot safely support reporting, Made MUST keep the terminal usable at the latest known cwd or initial folder and explain the capability in Diagnostics without claiming live follow. cmd MUST have this explicit fallback unless a safe reporting mechanism is demonstrated; receiving OSC 7 alone MUST NOT be described as guaranteed cmd or arbitrary-shell integration.

#### Scenario: Real shell directory change

- Given each supported bash, zsh, fish or PowerShell launch starts in a chosen folder with launch-local reporting
- When the user runs cd into another local directory, including a path with spaces or Unicode
- Then the owning explorer follows the reported directory within the 500 ms SHOULD target without restarting the shell or requiring OpenCode

#### Scenario: Shell hooks preserve user setup

- Given existing user profiles, prompt hooks and startup behavior
- When a supported shell starts, changes directory and exits under Made
- Then those behaviors remain intact, cwd reports are emitted, user shell files are unchanged, and only that launch's app-owned bootstrap resources are cleaned up

#### Scenario: Unsupported or restricted cwd reporting

- Given cmd, an unknown shell, or a launch mode/policy where safe hooks cannot be installed
- When the shell starts and the user changes directory
- Then the shell remains usable, Made does not claim guaranteed live follow, the latest known cwd or launch folder remains the fallback, and Diagnostics explains capability and reason

#### Scenario: Malformed or unavailable shell directory

- Given a shell report containing malformed, remote, relative or oversized data, or a local directory that is missing/unreadable
- When Made processes the report
- Then invalid data cannot change the root or execute anything, unavailable local roots receive contextual feedback without showing unrelated contents as current, and the terminal remains usable

#### Scenario: Active OpenCode overrides shell then returns

- Given the shell has reported directory A and connected OpenCode has an active session in B
- When the shell reports C while integration is active, then integration disconnects or OpenCode exits
- Then B wins while active, C is retained independently, and the explorer switches to C after disconnect rather than retaining B

### Requirement: Independent Workspace Tabs

Made MUST let users open an additional workspace folder in a new tab, switch workspace tabs, and close a tab in the same native window. Each tab MUST own its shell/process lifecycle, terminal state, explorer/root generations, shell cwd, bridge/connection state including the connected view directory, OpenCode session map, preview selection/mode, explorer visibility/width and focus state. Switching tabs MUST retain live shell and OpenCode processes and MUST NOT leak cwd, preview content, selection, connection state, shortcuts or focus between tabs. Workspace tab and optional OpenCode session-map presentation are owned by the UI spec; this requirement owns tab lifecycle, ownership, and isolation. Closing a tab MUST terminate/reap only its owned processes and release its watcher, bridge, queues, secrets and bootstrap resources; late results MUST NOT reach a surviving or newly created tab. Closing the last tab MUST return to the folder picker. No tab persistence across app restart is required.

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

#### Scenario: Hidden workspaces remain current and bounded

- Given an inactive workspace produces sustained terminal output and connection changes
- When another tab is used and the inactive tab is later selected
- Then output and connection state have progressed with bounded queues/scrollback, no hidden grid was painted, no input was sent to the inactive tab, and current state is displayed without an unbounded replay

### Requirement: Active Session Follow

While integration is connected with session tabs enabled and an active OpenCode session directory, the explorer MUST follow that directory in its owning workspace tab, taking precedence over the connected view directory and shell cwd. While session tabs are enabled and no active session directory is present, Made MUST follow a usable connected view directory when one is reported, and otherwise resume the latest known shell cwd, then the launch folder if none is known. When session tabs are unavailable, Made MUST keep the degraded shell-cwd fallback and MUST NOT use the connected view directory. Disconnect, normal OpenCode exit, unsupported integration, session tabs becoming unavailable, or a new connection epoch MUST clear the view directory before applying that fallback; an old agent or view root MUST NOT override it. Shell cwd updates MUST continue to be recorded while OpenCode takes precedence. When the active session changes, Made MUST update the explorer root as soon as the change is received and SHOULD complete it within 500 ms for a local workspace. Made MUST NOT redirect to a repository's main checkout by default. An unavailable active directory MUST be represented as unavailable, not silently replaced with unrelated root content.

#### Scenario: Switch between sessions in the same worktree

- Given two OpenCode sessions share the same working directory
- When the user switches between them
- Then the explorer root stays the same and the active-session indication updates

#### Scenario: Switch to a session in a different worktree

- Given two OpenCode sessions use different worktrees
- When the user switches to the session in the other worktree
- Then the explorer switches to that worktree root and the terminal content is unchanged, with the switch completing within the 500 ms SHOULD target

#### Scenario: Active session returns to main checkout

- Given the active session's working directory is the main checkout
- When the session becomes active
- Then the explorer shows the main checkout because it is the active directory, not because Made defaults to it

#### Scenario: Rapid tab switching

- Given the user switches session tabs repeatedly
- When the final active session is established
- Then the explorer shows the final session's directory and no stale session update overwrites it

#### Scenario: Active session directory unavailable

- Given the active session's working directory has been removed or is unreadable
- When Made attempts to follow it
- Then Made shows an unavailable-root message, keeps the terminal usable, and does not crash

### Requirement: Connected View Directory Follow

While integration is connected with session tabs enabled and no active OpenCode session, the explorer MUST follow the OpenCode TUI's connected view directory when OpenCode reports a usable local absolute directory for the folder selected in its Home/new-session view, taking precedence over shell cwd. When OpenCode reports session tabs unavailable, Made MUST keep the degraded shell-cwd fallback, MUST NOT follow the view directory, and MUST clear any retained view-directory state. The view directory MUST be treated as untrusted data and validated like a session directory; an absent, null, relative, malformed, oversized, or unavailable view directory MUST NOT itself become the root. A reported but unusable view directory MUST fall back to the latest known shell cwd, then the launch folder, and MUST be recorded in Diagnostics without crashing or presenting unrelated content as the view directory. When the view directory changes, Made MUST update the owning explorer root as soon as the change is received and SHOULD complete it within 500 ms for a local workspace. While an active session exists, the active session directory MUST take precedence and view-directory changes MUST NOT change the root. On disconnect, normal OpenCode exit, unsupported integration, session tabs becoming unavailable, or a new connection epoch, Made MUST clear the view directory and resume the latest known shell cwd, then the launch folder if none is known. The view directory is observed only through the OpenCode TUI plugin; a folder selected in the OpenCode desktop app's own picker is not observable by this integration and is outside this requirement.

#### Scenario: Connected view directory before any session

- Given OpenCode is connected with session tabs enabled, no session tab is active, and OpenCode reports a usable view directory for the folder selected in its Home/new-session view
- When Made receives integration state
- Then the owning explorer follows that view directory and the latest known shell cwd does not override it

#### Scenario: View directory changes in the new-session view

- Given OpenCode is connected with session tabs enabled and no active session and the explorer follows view directory V1
- When the user changes the workspace or folder in OpenCode's Home/new-session view to V2
- Then Made updates the owning explorer root to V2 as soon as the change is received and SHOULD complete it within 500 ms for a local workspace

#### Scenario: Active session directory wins over view directory

- Given OpenCode is connected with session tabs enabled, an active session directory B, and a usable connected view directory V
- When Made receives integration state or V changes
- Then the explorer follows B, not V, and V does not replace B while the session remains active

#### Scenario: View directory ignored when session tabs are unavailable

- Given OpenCode is connected but reports session tabs unavailable and a usable-looking view directory
- When Made receives integration state
- Then Made keeps the degraded shell-cwd fallback, does not follow the view directory, and Diagnostics identifies the v1 tabs constraint

#### Scenario: Disconnected or no usable view directory

- Given integration is disconnected, or OpenCode reports no usable view directory
- When Made computes the workspace root
- Then the explorer resumes the latest known shell cwd, falling back to the launch folder only until cwd is known, and an old view or agent root does not override that fallback

#### Scenario: Unusable view directory

- Given OpenCode is connected with session tabs enabled and reports an absolute view directory that is missing or unreadable
- When Made receives integration state
- Then the unusable directory cannot become the root, the latest known shell cwd or launch folder remains usable, Diagnostics records the unusable view directory, and Made does not crash

#### Scenario: Invalid view path stays non-fatal

- Given OpenCode's Home/new-session view reports an empty, relative, NUL-bearing, traversal-bearing, or oversized path
- When the plugin builds integration state
- Then the invalid value is treated as absent, the plugin reports a non-fatal error without disconnecting, and Made follows the latest known shell cwd rather than an invalid view root

### Requirement: Worktree Awareness

Made MUST treat an OpenCode-managed worktree as the active workspace when the active session's working directory is that worktree. Made MUST make the worktree root, branch or worktree metadata, and active session inspectable in Diagnostics and the optional session map. Explorer root identity and session-map presentation are owned by the UI spec. Made MUST NOT create, remove, merge, or otherwise manage worktrees.

#### Scenario: OpenCode-created worktree becomes active

- Given OpenCode creates a worktree and opens a session in it
- When that session becomes active
- Then the explorer shows the worktree root, its branch/worktree and active-session details are inspectable in Diagnostics and the optional session map, and the main checkout is not shown or modified

#### Scenario: Worktree metadata unavailable

- Given the active directory is a worktree but its branch or worktree metadata cannot be read
- When Made displays the workspace
- Then the explorer still shows the worktree root, Diagnostics reports the active session and unknown metadata, and Made does not crash

#### Scenario: Worktree removal by OpenCode

- Given OpenCode removes the active worktree
- When Made receives the updated session state
- Then Made does not recreate the worktree, shows the root as unavailable, and keeps the terminal usable

### Requirement: Session Map

Made MUST provide optional session data for open OpenCode root-session tabs for the selected workspace only. The data MUST be scoped to that workspace and MUST NOT be confused with workspace tabs. The data MUST update when sessions open, close, or change active state, and MUST carry busy, attention, and unread state. Made SHOULD focus the selected session in that workspace's OpenCode instance when the user activates it from the session map. Initial visibility, tab presentation, badges, overflow, empty state, and focus-error presentation are owned by the UI spec.

#### Scenario: Multiple open sessions

- Given multiple OpenCode sessions are open in different worktrees
- When the session map is visible
- Then it lists every open root session with its title and path and marks exactly one active session

#### Scenario: Session closes

- Given an OpenCode session is closed
- When Made receives the updated state
- Then the session is removed from the map within 1 second and the active indication remains consistent

#### Scenario: Click focuses session

- Given the user activates a non-active session in the session map
- When the focus request succeeds
- Then OpenCode makes that session active and Made's explorer follows its directory

#### Scenario: Focus command fails

- Given the user activates a session that no longer exists or focus does not complete
- When the focus request fails or times out
- Then Made shows a non-blocking error, keeps the current workspace, and refreshes session state

### Requirement: Read-Only File Preview

Made MUST open a file from the explorer in a read-only preview state owned by the workspace tab. The terminal process MUST remain running and consume output while preview is selected; hidden terminal input MUST NOT receive preview keystrokes, and returning MUST restore the same live terminal without relaunch or lost output. A root change MUST clear the prior root's preview and reject stale preview results. The preview MUST NOT allow editing, saving, or any file modification. Binary files and files above the documented preview size limit MUST show a clear, safe message instead of raw content. Made SHOULD refresh an open preview when the file changes on disk. Full-left presentation, preview header/actions, return control, meta, gutter, syntax highlighting, and safe-state styling are owned by the UI spec.

#### Scenario: Binary file

- Given a binary file
- When the user selects it
- Then Made shows a clear binary-file message and does not render raw binary content

#### Scenario: Large file

- Given a file above the documented preview size limit
- When the user selects it
- Then Made shows a clear size-limit message with the file size and does not attempt to render the full file

#### Scenario: External change refresh

- Given a text file is open in the preview
- When the file changes on disk
- Then Made refreshes the preview within 1 second or shows that the preview is stale

#### Scenario: File disappears

- Given the previewed file is deleted or becomes unreadable
- When Made refreshes
- Then Made shows an unavailable-file message and keeps the rest of the app usable

#### Scenario: Root change clears preview

- Given a preview and an in-flight preview read under an old root
- When shell or OpenCode follow changes the owning root
- Then the old preview is cleared to the terminal surface, late reads cannot restore old content, and no other workspace's preview is affected

#### Scenario: Preview keeps terminal process and input isolated

- Given a running terminal command and a file open in preview
- When the user types while preview is selected and the command continues producing output
- Then the shell process stays alive, its output continues to be consumed, and no preview keystroke reaches the hidden terminal

### Requirement: Explorer Live Updates

While the explorer is visible, Made MUST reflect filesystem changes in the active root without requiring a manual refresh. A created file MUST appear in the explorer within 1 second, a deleted file MUST disappear from the explorer within 1 second, and a renamed file MUST remove the old path and show the new path. When the active root changes, Made MUST discard stale loads and events from the previous root so they cannot populate the new tree. An unreadable directory MUST show an error row and MUST NOT crash the explorer. Made SHOULD preserve expansion and selection where possible across updates. Manual-refresh absence and explorer chrome are owned by the UI spec.

#### Scenario: Created file appears

- Given the explorer is showing the active root
- When a user or agent creates a file
- Then the new file appears in the tree within 1 second

#### Scenario: Deleted file disappears

- Given a file is visible in the explorer
- When the file is deleted
- Then the file disappears from the tree within 1 second

#### Scenario: Renamed file updates

- Given a file is visible in the explorer
- When the file is renamed
- Then the old path disappears and the new path appears

#### Scenario: Active root switch discards stale events

- Given the explorer is switching to a new active root
- When a stale load or filesystem event from the previous root arrives
- Then the event cannot populate or modify the new tree

#### Scenario: Unreadable directory shows an error row

- Given a directory in the active root cannot be read
- When the explorer renders it
- Then the explorer shows an error row and does not crash

#### Scenario: Expansion and selection preserved

- Given the user has expanded directories and selected a file
- When a live update changes another part of the tree
- Then the expansion and selection are preserved where possible

### Requirement: Degraded Integration

OpenCode integration is optional. If it is unavailable, disabled, incompatible, disconnected, or OpenCode is missing or older than v2, Made MUST keep the terminal and explorer usable with shell-directory fallback as defined in Active Session Follow. Normal shell use and normal OpenCode exit MUST NOT be treated as an app-level degraded state. Attempted integration failures MUST remain inspectable with actionable reasons in Diagnostics; actual shell, filesystem, or user-action failures MUST retain contextual non-blocking feedback. Made MUST NOT crash or block terminal input. Made SHOULD reconnect automatically when integration becomes available, but MUST NOT launch OpenCode automatically. Made MUST launch OpenCode only when the user explicitly chooses OpenCode from the `+` new-tab menu, which starts the detected executable directly as a new home-directory tab. Routine chrome suppression is owned by the UI spec.

#### Scenario: Plugin unavailable

- Given the OpenCode integration is not present
- When the user uses OpenCode in the Made terminal
- Then the terminal remains fully usable, the explorer follows shell-directory fallback, and Diagnostics records integration unavailability

#### Scenario: OpenCode missing or older than v2

- Given OpenCode is not installed or the installed version is older than v2
- When the user tries to use the integration
- Then the terminal remains usable and Diagnostics provides an unavailable or unsupported integration state with an actionable reason

#### Scenario: OpenCode started with extension loading disabled

- Given OpenCode is started with extension loading disabled
- When Made cannot connect
- Then Diagnostics names disabled integration, shell follow continues, and Made does not crash

#### Scenario: Bridge disconnects mid-session

- Given integration was connected
- When the connection drops while OpenCode is still running
- Then Made resumes the latest known shell cwd (launch folder only if unknown), keeps the terminal usable, and records the disconnected reason in Diagnostics

#### Scenario: Incompatible protocol version

- Given the integration reports a protocol version Made does not support
- When Made receives it
- Then Made rejects the state, records unsupported integration in Diagnostics, and keeps the terminal and shell-following explorer usable

#### Scenario: OpenCode exits normally

- Given OpenCode exits normally
- When the integration disconnects
- Then Made resumes shell-directory fallback without treating the workspace as degraded and leaves the same shell usable for a new user-initiated launch

#### Scenario: Reconnect

- Given integration returns after a disconnect
- When a valid connection with session tabs enabled is established
- Then Diagnostics reports connected and Made resumes following the active session or, when no session is active and session tabs are enabled, the connected view directory in that workspace without applying stale connection state

#### Scenario: Session tabs unavailable

- Given OpenCode reports that session tabs are disabled
- When Made receives the state
- Then Diagnostics identifies the v1 tabs requirement, Made keeps the terminal and shell-following explorer usable, and Made does not follow the connected view directory

#### Scenario: Configuration composition fails

- Given Made cannot compose the per-launch OpenCode configuration without altering user files
- When OpenCode starts
- Then OpenCode can run without Made integration, Diagnostics reports the composition failure, shell use remains normal, and user files stay unchanged

### Requirement: Terminal Fidelity

The embedded terminal MUST run the real OpenCode v2 TUI and MUST support the terminal features it relies on: truecolor, alternate screen, mouse reporting, bracketed paste, resize, Unicode and emoji fallback, focus reporting, title updates, scrollback, selection, copy and paste, keyboard encoding, and basic IME composition.

#### Scenario: OpenCode TUI renders

- Given OpenCode v2 is installed
- When the user starts it in the Made terminal
- Then the OpenCode TUI renders in its alternate screen with correct truecolor output and layout and accepts keyboard input

#### Scenario: Resize

- Given OpenCode is running
- When the user resizes the Made window
- Then the TUI reflows without corruption and the terminal reports the new size to OpenCode

#### Scenario: Mouse

- Given OpenCode enables mouse reporting
- When the user clicks or scrolls in the terminal
- Then OpenCode receives the corresponding mouse events

#### Scenario: Bracketed paste

- Given the user pastes multiline text
- When the terminal sends the paste
- Then OpenCode receives it as a bracketed paste rather than as individual keystrokes

#### Scenario: Unicode and emoji

- Given the user enters CJK text or emoji
- When the terminal renders it
- Then glyphs and widths are correct or a documented fallback is used, and the TUI does not corrupt

#### Scenario: IME composition

- Given the user composes text with a basic IME
- When the composition is committed
- Then OpenCode receives the composed text exactly once and no duplicate characters appear

#### Scenario: Scrollback and copy

- Given scrollback contains earlier output
- When the user scrolls back, selects text, and copies it
- Then the selected text is placed on the system clipboard

#### Scenario: Title and focus reporting

- Given OpenCode changes the terminal title or focus state
- When the change occurs
- Then Made reflects the title and reports focus changes to OpenCode

### Requirement: Cross-Platform Delivery

Made MUST build from one source tree for macOS arm64, macOS x64, Linux x64 on X11 and Wayland, and Windows x64 using the native OpenCode v2 distribution. Platform-specific behavior MUST be isolated so the observable workflow is the same on every supported platform. The UI structure and responsive presentation are owned by the UI spec. CI MUST produce development artifacts for every supported target.

#### Scenario: CI builds all targets

- Given the main branch
- When CI runs
- Then it produces development artifacts for macOS arm64, macOS x64, Linux x64, and Windows x64

#### Scenario: Linux X11 and Wayland

- Given a Linux x64 build
- When it runs under X11 and under Wayland
- Then the app starts, the terminal runs OpenCode, and session follow works in both display-server sessions

#### Scenario: Windows native OpenCode

- Given a Windows x64 build
- When the user runs the native OpenCode v2 distribution
- Then the terminal, session follow, explorer, and preview work without a WSL dependency

#### Scenario: macOS

- Given a macOS build for arm64 and x64
- When the user runs OpenCode v2
- Then the terminal and session follow work on both architectures

#### Scenario: No per-OS UI rewrite

- Given the supported platform builds
- When their UI is compared
- Then the same panels, workflows, and session-follow behavior are present, with no per-OS UI rewrite

### Requirement: Security and Local-Only Integration

Made MUST accept integration connections only over the loopback interface, on a randomly assigned port, authenticated by a per-workspace-instance secret. Credentials and routing MUST isolate workspace tabs: a valid client for one tab MUST NOT mutate, focus, replace the connection of, or disclose state from another tab. Made MUST reject unauthenticated or unauthorized clients before disclosing state. Made MUST NOT persistently modify user shell or OpenCode configuration files. Made MUST load integration code from an app-owned location. Made MUST restrict workspace-content reads to the owning tab's current root and paths the user explicitly selects, and MUST reject path traversal outside those roots. Explicit folder selection, the default home workspace, and validated local cwd, session, or view-directory reports authorize root changes only for their owning tab; they MUST NOT authorize arbitrary file reads in other tabs. Read-only user configuration composition and app-owned resources remain separate from workspace-content reads. The integration bridge MUST NOT be reachable from non-loopback interfaces.

#### Scenario: Unauthorized bridge client rejected

- Given the integration bridge is running
- When a client connects without the per-instance secret
- Then Made rejects the connection, discloses no session state, and records the rejection

#### Scenario: Non-loopback bind

- Given the integration bridge is listening
- When the user inspects its listening sockets
- Then it is bound only to a loopback address and not to a wildcard or external interface

#### Scenario: User config unchanged

- Given the user has an OpenCode configuration file
- When the user runs OpenCode in Made and the session ends
- Then the file's content is byte-for-byte unchanged and no Made-managed content was persisted into it

#### Scenario: Path traversal blocked

- Given a path outside the active root
- When a file request attempts traversal or a symlink escape
- Then Made rejects the request and shows no content from outside the root

#### Scenario: Plugin from app-owned path

- Given Made loads its integration code
- When the loaded location is inspected
- Then it resolves inside the app-owned installation or resource area and not inside the user's configuration directory

#### Scenario: Read-only preview enforced

- Given a file is open in the preview
- When the user attempts an edit or save action
- Then no file content is written and the preview remains read-only

### Requirement: Responsiveness and Lightness

Made MUST NOT use a webview runtime. Extreme responsiveness and fluid interaction are base v1 acceptance criteria; Calm Editor visual quality is owned by the UI spec. Shell use, optional OpenCode, tab switching, preview roundtrip, and explorer hide/resize MUST NOT block input or perform synchronous tree/preview scans on the UI thread. On a supported reference machine, Made SHOULD meet these targets: startup to an interactive terminal and browsable explorer within 2 seconds for a 10,000-file workspace, reported cwd, active-session, or view-directory change to explorer update within 500 ms, terminal input-to-render within 50 ms, and sustained output at 60 frames per second without losing input. Hidden tabs and preview-covered terminals MUST consume bounded output and connection state without rendering hidden grids; activation MUST show current state without replaying an unbounded UI backlog. Evidence MUST distinguish automated/model and software-rendered checks from real hardware-accelerated GUI measurements, with machine, backend, workload, tab count and unverified platforms recorded; software evidence alone MUST NOT claim hardware smoothness.

#### Scenario: Startup budget

- Given a supported reference machine and a 10,000-file workspace
- When Made starts
- Then the terminal is interactive and the explorer is available for browsing within 2 seconds, excluding first-run OS prompts

#### Scenario: Session switch budget

- Given integration is connected
- When the active session changes
- Then the explorer updates within 500 ms

#### Scenario: Input latency

- Given OpenCode is running
- When the user presses a key
- Then the resulting terminal update is rendered within 50 ms under normal load

#### Scenario: Sustained output

- Given OpenCode produces sustained output
- When the user types and scrolls
- Then the terminal remains responsive, input is not lost, and rendering stays near 60 frames per second or degrades without blocking input

#### Scenario: No webview

- Given a running Made instance
- When its process tree is inspected
- Then no webview runtime is present

#### Scenario: Complete terminal-first performance evidence

- Given a documented reference machine, backend, a 10,000-file workspace and at least three live workspace tabs with sustained output in an inactive tab
- When real shell cd follow, optional OpenCode follow, tab switching, preview open/return and explorer hide/resize are measured and visually reviewed at both required window sizes
- Then evidence records startup, cwd/session-follow latency, input latency and frame pacing against the stated targets, verifies bounded hidden-tab processing with no hidden grid painting, and separates hardware GUI evidence from software/model checks and unverified platforms

### Requirement: Diagnostics

Made MUST expose optional integration status (inactive, connected, unavailable/degraded, or unsupported), the selected workspace identity, latest known shell cwd and cwd-reporting capability/reason, effective root and its source, active session or, when session tabs are enabled, connected view directory, workspace/worktree, integration version when connected, and access to integration logs in Diagnostics. Normal shell use MUST be inactive, not an app-level degraded state. Technical statuses, versions, branch/worktree metadata, watcher state, metrics and connection details MUST remain accessible without a debugger. User-action failures MUST remain discoverable. Logs and copied diagnostics MUST NOT contain workspace-instance secrets. Diagnostics panel layout, technical-detail placement, and routine chrome are owned by the UI spec.

Diagnostics logs MUST be bounded and retained as one active log plus one previous log. For current-version appends that take the app-owned lock, the active log MUST stay at most 1 MiB; Made MUST rotate before a write would exceed it. A mixed-version or non-cooperating writer can exceed this bound and is a documented residual. The previous log may exceed 1 MiB only when it preserves a legacy active file that was already oversized; its bytes MUST NOT be truncated, rewritten, or deleted. A single entry message MUST be capped at 4 KiB on a UTF-8 boundary with a visible truncation marker, retained log text MUST be capped at 512 KiB with oldest entries evicted before newer ones, at most 500 entries MUST be retained, and every record written to the active log MUST be smaller than 1 MiB. Every append MUST be serialized by an app-owned lock with a bounded wait. If the lock is unavailable or times out, or a required rotation cannot be performed safely, the append MUST be refused and the active and previous logs left untouched. Startup MUST keep a non-blocking warning for a legacy active log already above 1 MiB and for an unsafe previous-log or lock path. Write and rotation failures MUST be reported on stderr; Diagnostics shows a warning or error only while the app is still rendering (the startup warnings and any issue present before the window closes). Final-exit write failures are not visible in-app and remain a documented evidence gap.

#### Scenario: Connected diagnostics

- Given integration is connected
- When the user opens diagnostics
- Then Made shows connected status, the active session or connected view directory, the workspace or worktree path, and the integration version

#### Scenario: Connected diagnostics without a session

- Given integration is connected with session tabs enabled, no active session, and a usable connected view directory
- When the user opens diagnostics
- Then Made shows connected status, the connected view directory, the effective root and its source, and the integration version

#### Scenario: Degraded diagnostics

- Given integration is degraded
- When the user opens diagnostics
- Then Made shows the integration failure reason, owning workspace, latest shell cwd, effective root and its source, without treating the shell workspace as degraded

#### Scenario: Unsupported integration

- Given the integration is incompatible or unavailable
- When the user opens diagnostics
- Then Made shows an unsupported or unavailable status with an actionable reason

#### Scenario: Logs redact token

- Given integration logs are available
- When the user inspects them
- Then the per-instance secret is absent and the log entries identify the relevant session or connection

#### Scenario: Bounded log rotation preserves history

- Given the active diagnostics log is near 1 MiB and a current-version clean shutdown writes a record that takes the lock
- When that record would exceed 1 MiB
- Then the active log becomes the single previous log before the new record is appended, no existing bytes are truncated, rewritten, or deleted, the active log stays at most 1 MiB, and the previous log may exceed 1 MiB only when it preserves a legacy oversized file

#### Scenario: Oversized log content stays bounded

- Given a log entry message exceeds 4 KiB or retained log text would exceed 512 KiB
- When entries are recorded and Diagnostics is inspected or copied
- Then the message is truncated on a UTF-8 boundary with a marker, oldest entries are evicted before newer ones, at most 500 entries remain, and the record written to the active log stays below 1 MiB

#### Scenario: Log retention failure is reported

- Given the diagnostics log lock is unavailable or times out, the previous-log path is unsafe, or a required rotation fails
- When a diagnostics log record is written, including at final shutdown
- Then the record is refused, the active and previous logs are left untouched, the failure is reported on stderr, and Diagnostics shows an error only while the app is still rendering; final-exit failures are not visible in-app

#### Scenario: Legacy oversized log warning

- Given an existing active diagnostics log is above 1 MiB at startup
- When the workspace starts and Diagnostics is opened before shutdown
- Then a non-blocking warning states that it will be kept as the previous log on the next shutdown write, and future writes are not refused

#### Scenario: Unsafe log path warning

- Given the previous-log or lock path exists as a symlink or non-regular file at startup
- When the workspace starts and Diagnostics is opened before shutdown
- Then a non-blocking warning names the unsafe path and states that rotation or writes will be refused

#### Scenario: Shell-only diagnostics

- Given a shell-only workspace with supported or unsupported cwd reporting
- When the user opens Diagnostics
- Then its workspace identity, reporting capability/reason, latest shell cwd, effective root/source and inactive optional integration are inspectable and secrets are absent from displayed/copied logs

### Requirement: V1 Scope Boundary

V1 MUST support only OpenCode v2 and MUST NOT integrate with other agent TUIs or ACP. V1 MUST NOT require remote, SSH, or WSL-hosted OpenCode. V1 MUST NOT provide an AI or chat UI that replaces the OpenCode TUI. Made MUST NOT create, remove, merge, or orchestrate worktrees; worktree lifecycle remains owned by OpenCode. V1 MUST NOT require a separate OpenCode server process.

#### Scenario: Unsupported agent not offered

- Given the Made interface
- When the user looks for agent integrations
- Then only OpenCode v2 is offered and no Claude Code, Codex, Grok, ACP, or other agent integration is presented

#### Scenario: No replacement chat UI

- Given the Made interface
- When the user looks for an AI or chat surface
- Then no AI or chat UI replaces the OpenCode TUI

#### Scenario: Worktree lifecycle remains in OpenCode

- Given a worktree
- When the user inspects Made's controls
- Then no create, remove, merge, or orchestration action is available, and worktree changes appear only after OpenCode reports them

#### Scenario: Direct launch

- Given a user wants to use OpenCode with Made
- When they run the OpenCode command directly in the Made terminal
- Then integration works without starting a separate server or attach process
