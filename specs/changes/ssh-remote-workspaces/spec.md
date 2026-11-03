# Change Spec: ssh-remote-workspaces

Clarified delta covering the runtime/integration, UI and explorer domains, following the user's approval of Q1–Q6 recorded in proposal.md. Requirement identifiers below are stable traceability labels. Existing local guarantees and scenarios remain in force; remote guarantees below extend the experience to an explicitly selected SSH authority, not to arbitrary network paths. Engineering defaults below make those approved outcomes testable; implementation/verification results are not claimed by this contract.

Remote filesystem/content correctness and remote enforcement guarantees below assume a trusted server and correctly operating helper. This assumption MUST NOT weaken the required client-generated operation confinement, helper filesystem guards or client protection against hostile responses defined in SSH-08. Made MUST NOT claim to attest the filesystem or provenance of content returned by a compromised server/helper, even when SSH authentication and response syntax are valid. Client-side authority/tab isolation and hostile-response validation remain required regardless of server honesty.

## ADDED Requirements

### Requirement: SSH server entry and launch choices (SSH-01)
Made MUST expose a `Servidores SSH` icon-only header action immediately beside the Explorer hide/show action, with matching tooltip/accessibility name, visible keyboard focus and at least a 24×24 target. It MUST offer server registration, an explicit connection test and remote OpenCode detection with success/failure feedback. The `+` flow MUST retain existing local choices and offer `Terminal SSH` and `OpenCode SSH` for a selected registered server; `OpenCode SSH` MUST be offered only when OpenCode has been detected on that server. Detection MUST NOT start the OpenCode TUI or install it. Failure or absence MUST NOT prevent a supported remote terminal/explorer workspace or unrelated local workspaces from operating. A previously detected executable that disappears before launch MUST cause contextual failure, not local execution or substitution of a different server.

Server records MUST persist across client restart in private app-owned storage and contain only id/display name, SSH host/alias, optional user/port/key-file reference and optional initial remote directory, never private-key bytes, credentials, tokens or prompt answers. Add/edit/remove MUST be supported; record changes MUST NOT retarget/terminate existing tabs. Corrupt records MUST produce a recoverable error without overwriting the original or blocking local startup; schema-version mismatch MUST NOT be silently rewritten. Live remote tabs/secrets MUST NOT be restored across application restart.

Authentication MUST use installed host OpenSSH, existing aliases/config/ProxyJump, available keys/agent and interactive encrypted-key/password/keyboard-interactive/MFA prompts. No credentials MUST be persisted or logged; cancellation MUST stop the owning attempt only. Unknown host identity MUST display the OpenSSH fingerprint and require explicit approval before connection; additions MUST be app-owned, without rewriting personal SSH config/known_hosts. A changed trusted key MUST be refused, not accepted through the first-use approval flow. Existing user-supplied SSH config/proxy commands remain user-trusted local configuration, not data supplied by the server. Agent forwarding and configured network forwards MUST be disabled for Made connections. Missing OpenSSH or unavailable prompt support MUST fail actionably rather than falling back to insecure authentication.

#### Scenario: SSH entry remains reachable
- Given a window at 1280×800 and 720×480 with overflowing tabs, with Sessions both visible and hidden
- When the user uses pointer and keyboard to reach SSH server management and the `+` menu
- Then the SSH action is adjacent to Explorer, neither action nor `+` clips or overlaps tabs, the action has its label and focus metadata, and local creation remains available

#### Scenario: Connection test and per-server detection
- Given server A has supported OpenCode installed and server B does not, and both are otherwise supported reachable servers
- When the user registers and tests them and opens their remote launch choices
- Then each test reports its own result, `Terminal SSH` is available for either server and `OpenCode SSH` only for A, no OpenCode TUI is started by the test, and neither local detection nor A's result is reused for B

#### Scenario: Failed probe or vanished executable
- Given a test fails due to authentication, network or host-trust failure, or a detected OpenCode executable disappears before launch
- When the user inspects the result or attempts that launch
- Then Made reports an actionable scoped error, does not silently install, bypass host verification, run locally or affect another tab, and does not falsely claim a successful remote OpenCode launch

#### Scenario: Persistent metadata and interactive authentication
- Given two saved records, a corrupted-record variant, an existing alias/ProxyJump and isolated agent/key, encrypted-key, password and MFA server fixtures
- When the client restarts, records are edited/removed, and authentication is approved or cancelled through each supported method
- Then valid metadata survives without credentials, corruption preserves the original and local startup, live tabs retain their original authority, all supported prompts work without logging/storing answers, and cancelling one attempt leaves sibling tabs intact

### Requirement: Remote workspace identity and root authority (SSH-02)
Each remote workspace MUST bind its server identity, remote launch directory, shell cwd, effective root, OpenCode instance, filesystem/Git operations and terminal lifecycle to that tab. Remote absolute paths MUST be interpreted under the server's path semantics, never canonicalized or opened as host paths. The selected remote directory MUST be browsable/selectable through the remote launch flow; the remote shell or explicitly launched OpenCode MUST start there. A remote cwd, session or view-directory report MUST be bounded and untrusted; validation MUST occur against the owning remote authority. Identical path strings on different servers or on the host MUST NOT alias. The selected authority MUST be inspectable without changing the local OpenCode-active tab's path-free tooltip guarantee.

#### Scenario: Same path on host and two servers
- Given the host and servers A and B all have `/project` with different contents
- When separate remote workspaces for A and B browse/select `/project` and a local workspace uses the host path
- Then each terminal starts and each explorer/preview reads only its own selected authority, and diagnostics identify which authority owns each path

#### Scenario: Remote directory rejected or unavailable
- Given a malformed/relative/oversized remote directory report or a selected remote directory that is missing or unreadable
- When Made validates the report or opens the directory
- Then invalid reports cannot execute commands or redirect to host content, unavailable paths have contextual feedback rather than unrelated content, and other workspaces stay usable

### Requirement: Remote explorer and preview parity (SSH-03)
A connected supported remote workspace MUST expose the existing lazy directory navigation, keyboard navigation, deterministic entry icons/special markers, expansion/selection, hide/restore/resize and full-left read-only preview experience against remote data. UI/icon behavior remains governed by `specs/made-ui-ux.md` and `specs/explorer-file-icons.md`. Preview MUST retain Copy path, explicit Refresh, Close/return to the same live terminal, syntax/plain-text fallback and input isolation. Copy path MUST copy the remote path, not an invented local mirror path; its owning server remains inspectable in the workspace. Remote preview MUST refuse editing/saving, non-regular entries, traversal/symlink escape, raw binary and invalid UTF-8; it MUST retain the current 2 MiB cap and safe missing/unreadable/oversized states. Listing/tree/depth caps MUST be no weaker than the current local caps (10,000 entries per directory, 50,000 visible model rows, depth 64) and truncation MUST be visible. Remote changes MUST automatically reconcile creates/deletes/renames and preview refresh or stale state without a routine manual explorer reload. Root changes MUST clear preview and reject stale results.

Remote live-change/preview/follow budgets and degraded-link behavior are specified by SSH-09. Requests MUST carry authority, connection epoch and root generation; the client MUST reject stale replies before applying UI state, and remote filesystem operations MUST enforce SSH-08 on the owning server.

#### Scenario: Remote browse and terminal roundtrip
- Given a supported connected remote workspace with nested directories, Unicode/spaces and supported/unknown file types, while a terminal command continues producing output
- When the user expands/selects through pointer and keyboard, opens a text preview, copies its path, hides/restores/resizes Explorer and returns to terminal
- Then the existing local presentation and actions operate on server data, valid selection/expansion survive, the copied path belongs to the server, preview keystrokes never reach the hidden terminal, and the same process and accumulated bounded output remain

#### Scenario: Remote live changes and late results
- Given a visible remote tree and preview with a delayed load, and a second workspace with its own preview
- When an external server process creates, deletes, renames or edits files, then the owning root switches before the delayed load completes
- Then the current tree automatically reconciles changes within the agreed remote budget, preview refreshes or is honestly marked stale, the old preview clears, stale data cannot populate the new root, and the other workspace remains unchanged

#### Scenario: Safe remote preview and listing limits
- Given remote binary, invalid UTF-8, greater-than-2-MiB, missing, unreadable and non-regular files, an unreadable child directory and a directory exceeding the listing cap
- When the user browses/selects them
- Then safe contextual states replace raw/unsafe content, unreadable directories show error rows, truncation is explicit, reads and UI memory remain bounded, and no preview write occurs

### Requirement: Remote read-only Git parity (SSH-04)
Git metadata/status MUST be observed on the remote authority, off the UI thread, for the selected remote root only. The existing Explorer/Source Control view switching, `REPOS` head, branch/detached-id/subject, linked-worktree name/icon, grouped Merge/Staged/Changes/Untracked sections, counts, status dots/ring colors and safe row-preview behavior MUST apply to remote workspaces. Root-relative filtering, ignored-path omission, partially staged/conflict handling and stale-generation rejection MUST remain. Only the current closed read-only Git operation surface is permitted; this change MUST NOT add commit, stage, reset, checkout, worktree creation/removal or arbitrary Git argument execution. Missing Git/non-repository roots MUST degrade as locally, without preventing file browsing or terminal use.

#### Scenario: Remote repository and worktree presentation
- Given a remote repository fixture with staged/unstaged/untracked/conflicted/ignored paths, changes outside the selected subdirectory, a linked worktree and a detached HEAD variant
- When the user switches between remote tree and Source Control, inspects the head/subject and opens eligible rows
- Then existing local sections/counts/icons/status and preview semantics apply using remote metadata, outside-root and ignored paths are not exposed, and no host repository or remote write operation is used

#### Scenario: Remote Git unavailable or stale
- Given remote Git is missing, the folder is not a repository, or a Git response for an old root/connection arrives late
- When Made refreshes Git state
- Then missing/unknown metadata is honest, stale state cannot repopulate the current view, and remote file browsing/terminal plus unrelated local Git remain usable

### Requirement: Remote terminal and clipboard parity (SSH-05)
The remote terminal MUST run the server's real shell or the explicitly selected server OpenCode TUI and preserve the existing terminal fidelity: truecolor, alternate screen, resize propagation to the remote PTY, mouse/focus reporting, bracketed text paste and limits/sanitization, Unicode/emoji fallback, IME commit-once, scrollback, selection/copy, and keyboard semantics. Detection/test MUST NOT launch OpenCode; only explicit launch or user execution may do so. Host clipboard text MUST NOT transfer without a paste action. Made MUST NOT paste a host-private image path as if it existed on the server or claim that forwarding Ctrl+V gives a remote program access to the host clipboard.

For a remote terminal only, both explicit image-paste chords MUST upload the clipboard image as a private PNG and paste the completed remote absolute UTF-8 path through existing text/bracketed-paste semantics; the default chord MUST NOT forward `0x16` and claim native remote clipboard retrieval. The first successful image paste per pane/launch MUST explain that remote paste uses an uploaded PNG path for both chords. Local default/native (`Cmd+V` / `Ctrl+V`) and fallback (`Cmd+Shift+V` / `Ctrl+Shift+V`) behavior MUST remain unchanged. Both remote chords MUST prefer image over coexisting text, preserve the 128 MiB RGBA/16384-pixel limits, reject a PNG exceeding 128 MiB encoded size, and transfer only upon that explicit paste; no clipboard polling/synchronization is permitted. Empty/unreadable/oversized clipboard, disconnected transport or failed upload MUST show that nothing was pasted, and MUST NOT paste text as a substitute, a host path or a partial remote path. An interrupted upload MUST NOT be retried or its path pasted automatically on reconnect; a new explicit paste is required.

The remote private `clipboard-images` directory MUST be `0700`, PNGs `0600`, created without overwrite/symlink following and never in the workspace. Cleanup MUST run on helper start and after each successful upload, remove only app-generated regular `clipboard-*.png` files older than seven days and then oldest files until at most 64 remain, preserve the current image and ignore unrelated/symlink entries. Cleanup failures MUST NOT invalidate a successful paste or remove unrelated files; failed partial uploads MUST be removed when reachable and swept on the next helper start. Successful images MUST survive tab close for the retention policy rather than being deleted while a consumer may use the path. When no helper runs or the server is unreachable, immediate physical deletion is not guaranteed: the next helper start MUST reconcile cleanup, and no completed-deletion claim may be made without confirmation.

#### Scenario: Real remote TUI fidelity
- Given a supported server with real OpenCode v2 and a connected Made workspace
- When the user launches it explicitly, resizes, uses mouse/focus, pastes multiline text, commits IME text, renders Unicode, scrolls/selects/copies and returns from alternate screen
- Then the TUI receives the correct remote terminal events, text paste retains the existing 1 MiB limit/sanitization, no IME character is duplicated, and rendering/input behavior matches the existing local terminal contract

#### Scenario: Default image paste cannot masquerade as native remote clipboard access
- Given the host clipboard contains an image and a remote TUI is active
- When the user invokes the default chord (`Cmd+V` on macOS or `Ctrl+V` elsewhere), which locally forwards `0x16` for child-native clipboard reading
- Then Made explicitly uses remote PNG upload plus completed server-path paste, explains the remote path behavior on the first success, sends no native image `0x16`, and makes no claim of native host clipboard access by the remote process; the local default chord remains native

#### Scenario: Fallback image paste requires a server-valid path and separate authorization
- Given the host clipboard contains an image and a remote TUI is active
- When the user invokes the fallback chord (`Cmd+Shift+V` on macOS or `Ctrl+Shift+V` elsewhere), which locally saves a private PNG and pastes its absolute path
- Then Made uploads a private PNG only for that paste, pastes its completed server-valid path, applies the retention/cleanup policy above, and never represents this as native clipboard retrieval

#### Scenario: Remote image upload failures and retention
- Given image-plus-text, empty/oversized clipboard, an interrupted upload, and a private remote directory with old/excess app PNGs, a symlink, unrelated files and a deletion-denied variant
- When each remote paste chord is exercised and helper startup/upload/close/reconnect cleanup occurs
- Then image priority and size/private-file limits hold, failed/disconnected paste sends no payload or automatic retry, only eligible app PNGs are cleaned by seven-day/64-file policy, the current image survives close, cleanup failure leaves its path usable, and no image/text bytes enter logs

### Requirement: Remote OpenCode session and cwd follow (SSH-06)
Supported remote integration MUST expose the current optional session-map data/actions, activity indicators and focus correlation within the owning remote OpenCode instance. Root precedence MUST remain active OpenCode session directory, then usable connected sessionless view directory when tabs are enabled, then latest validated remote shell cwd, then remote launch directory. Shell cwd MUST continue to be recorded while OpenCode takes precedence. Integration disconnect/normal exit/unsupported state/tabs disabled/new epoch MUST clear view-directory state and resume remote shell fallback without using the host cwd. Worktree/session changes MUST not restart terminal processes or manage worktrees. Missing or incompatible OpenCode/plugin MUST be distinguished from loss of SSH transport; optional integration failure MUST NOT by itself disable a healthy remote filesystem or terminal. Safe shell hooks MUST preserve remote user startup files and existing hooks; unsupported reporting MUST be reported honestly, never replaced by prompt scraping or injected commands in a running shell.

Helper/plugin provisioning MUST require explicit per-server/version consent before app-owned remote files are written; consent rejection, unsupported artifacts, missing supported-target capability, missing checksum tool, or transfer/integrity failure MUST fail clearly without mutating personal configuration or silently offering terminal-only parity. Existing pre-provisioned matching private artifacts MAY be verified and reused. Made MUST NOT install OpenCode. Made MUST NOT install, probe, execute, configure, or fall back to tmux; a server with tmux absent and a server with a user-installed tmux MUST be treated identically, and that user installation MUST NOT become a dependency, fallback, or hidden prerequisite. The frozen major-1 `Capabilities.tmux` field MUST remain for wire compatibility, MUST be encoded false, and MUST be ignored by every consumer; its presence is not permission to probe or require tmux. Optional OpenCode absence/version/integration failure MUST still leave an otherwise supported terminal/explorer usable. Remote config composition MUST read only remote user config/inline settings and inject only the remote app-owned plugin and remote loopback bridge for that launch; host provider/config/environment secrets MUST NOT be uploaded. Direct OpenCode and manual `opencode` inside a supported remote shell MUST both receive this launch-local integration environment. User shell, OpenCode, SSH, and any pre-existing tmux configuration bytes MUST remain unchanged.

#### Scenario: Remote session precedence and focus
- Given remote shell cwd A, session directory B, connected view V and multiple OpenCode sessions/worktrees
- When the user switches/focuses sessions, shell cwd changes to C, then the active session becomes absent with V usable, then integration exits
- Then the explorer follows B, then V, then C at the appropriate transition; the map/activity/focus results remain server/tab-scoped; the terminal is unchanged; and no host directory is consulted

#### Scenario: Integration failure is not SSH failure
- Given SSH terminal/filesystem transport is healthy but integration becomes incompatible, disconnects, disables tabs or returns an unavailable active directory
- When Made handles the state
- Then existing remote shell fallback/unavailable-root rules apply, diagnostics explain the integration reason, sessionless view state cannot outlive its epoch, and terminal/filesystem access remain usable without silent config changes

#### Scenario: Consented provisioning and manual remote OpenCode
- Given matching packaged artifacts, an isolated remote user config with existing plugins/hooks, plus tmux-absent, user-installed-tmux, denied-consent, corrupt-transfer and unsupported-target variants
- When the user approves or refuses provisioning, opens a remote shell, runs OpenCode manually and directly, and switches sessions/view directories
- Then only approved private verified artifacts are staged, both tmux variants provision and launch identically without probing or invoking tmux, missing checksum/target failures fail with manual instructions rather than installation, supported launches preserve config/plugins/hooks and remote follow, no host config/secrets are sent, and failures do not claim full workspace readiness

### Requirement: Remote isolation and disconnection (SSH-07)
Remote tabs MUST preserve all independent-workspace state and bounded hidden-workspace processing, including when several tabs use the same server. Closing a tab MUST release its owned local SSH resources, reads/watch subscriptions, bridge/tunnel, queues and secrets, without terminating sibling tabs or unrelated server sessions. Late results MUST be gated by workspace, authority, connection epoch and root generation. On transport loss, Made MUST stop treating old remote data as current, mark it disconnected/stale, reject unsafe pending operations and preserve unrelated workspaces; it MUST NOT silently use a host root or replay terminal input/commands. A connection-only failure and an optional-integration-only failure MUST remain distinguishable. Reconnect MUST revalidate authority/trust and root state and reject previous-epoch results.

On disconnected close, local cancellation and release of owned local resources/secrets MUST proceed independently of remote reachability. Local cancellation of a remote read/watch subscription MUST NOT be described as confirmed server-side termination. Network loss prevents guaranteeing immediate remote cleanup; Made MUST NOT report remote processes/resources as terminated/deleted without confirmation.

An already-running remote shell/OpenCode MUST survive SSH transport loss and reattach as the same process/session, without replay of commands, terminal input, or the historical PTY byte stream, while the trusted app-owned supervisor remains running and the detached lease has not expired. That supervisor MUST be detached from the SSH frontend through made-platform OS process capabilities and MUST own that tab's PTY, loopback bridge, filesystem watchers, and bounded VT state. It MUST NOT use tmux, or any other user-installed multiplexer, as owner, fallback, or recovery path. The detached lease MUST be 24 hours since the last authenticated client heartbeat; an attached client MUST renew it every 30 seconds, including inactive tabs. On expiry the live supervisor MUST terminate only that tab's managed PTY session, owned terminal process group, bridge, watchers, and app-owned runtime resources. Server reboot, supervisor death, and deliberately daemonized processes outside the managed PTY session are not covered by the survival/cleanup guarantee. Made MUST distinguish unavailable/expired session from reattachment and require explicit user action before starting a replacement process. During lost transport the client MUST stop accepting terminal input and discard unacknowledged queued input, pending pastes and outstanding focus commands, never retransmit them. Data already delivered before the break may have executed; exactly-once delivery across the break MUST NOT be claimed.

Online close MUST request authenticated owned-session teardown, wait at most two seconds for confirmation without blocking the UI, then release local resources regardless. A missing confirmation MUST produce an honest cleanup-unconfirmed notice; the remote lease remains the safety bound while the supervisor is healthy. Closing the client application MUST apply the same policy per tab. No broad user-level process kill, and no termination of shared OpenCode services or unrelated user processes outside the managed PTY session, including a user-started tmux, is permitted. Reattachment MUST be exclusive: a new attach establishes a new transport epoch, cancels the previous attachment and its input, and MUST NOT start a second PTY child. Before marking the tab current, Made MUST restore that running session's bounded VT state, including input modes, mouse reporting, alternate screen, cursor, and the visible screen, and MUST resnapshot root/filesystem/Git/session state. Restoration MUST NOT replay shell commands, terminal input, or the historical PTY byte stream, and MUST NOT resurrect old preview/focus results. Leaving alternate screen after reattach MUST show that same session's bounded primary screen and scrollback, not a blank or host terminal.

After every `Attached`, and before any live PTY output, the resnapshot MUST travel only on the existing `TerminalOutput` verb and MUST end with an explicit `done` bit on that same verb. A fresh PTY MAY use one empty completing frame. Zero or more frames with `done` false carry resnapshot bytes only and MUST NOT be empty; the single completing frame has `done` true, MAY be empty, and its bytes, if any, are still resnapshot. Live output after that frame MUST use the same verb with `done` false and MUST NOT be applied as more resnapshot, MUST NOT close input again, and MUST NOT be inserted into the resnapshot byte stream. The bit is required on every `TerminalOutput`; a missing bit is malformed, not completion. A second `done` true before the next `Attached` is malformed, not another snapshot. Client terminal input MUST stay closed until the completing frame has been applied. An empty poll, a timeout, or a `done` false frame MUST NOT reopen input and MUST NOT complete the resnapshot. This delimiter MUST NOT add an operation, change the protocol major, or use tmux. It does not complete the `Attach` or `Open` reply; `Attached` remains that reply. A new attach starts a new resnapshot; a previous `done` does not carry over. If the completing frame never arrives, input stays closed and the terminal is not marked current.

#### Scenario: Same-server tabs and close isolation
- Given two remote tabs on one server and one local tab with active commands, previews and pending reads
- When the user switches, hides a tab, and closes one remote tab while delayed results arrive
- Then only the closed tab's owned local resources are released, no delayed event reaches a survivor or replacement tab, surviving commands and state remain intact, hidden work stays bounded with no hidden grid rendering, and any remote termination/cleanup claim requires confirmation under the policy above

#### Scenario: Disconnected close separates local release from remote cleanup
- Given a remote tab loses its network connection while its command/helper may still run on the server
- When the user closes that tab before connectivity returns
- Then owned local resources/secrets are released and pending/late results are rejected without waiting for remote confirmation, no immediate remote cleanup or process termination is claimed, and the healthy remote supervisor expires only that session after its 24-hour lease without disturbing siblings

#### Scenario: Disconnect and reconnect cannot apply old data
- Given a remote command and file/Git/session operations are in flight
- When SSH is interrupted for 60 seconds, another local tab is used, and the remote workspace reconnects before lease expiry
- Then shell/OpenCode process/session identities and command progress survive, modes, mouse reporting, and alternate screen match the running session without historical replay, old data is visibly stale until the completing resnapshot frame is applied, local input stays closed until that `done` bit is applied and only then becomes usable, no queued terminal input/focus/paste is replayed, authority/trust/root are revalidated, and old-epoch replies cannot override new state

#### Scenario: Resnapshot completion is an explicit wire bit
- Given an attached remote session whose PTY continues to produce output, and a reattach whose resnapshot is split across more than one `TerminalOutput` frame
- When the client has applied only a prefix, the transport poll is empty, the completing `done` true frame arrives before any live byte, and later output arrives with `done` false
- Then input stays closed through the empty poll, the completing frame's bytes are still resnapshot, input opens only after that frame is applied, later `done` false output is live and is not appended to the resnapshot, and no new operation or tmux is used

#### Scenario: Missing completion does not reopen input
- Given a reattach that has not delivered `TerminalOutput` with `done` true, including an empty poll and a following `done` false frame
- When the user attempts terminal input
- Then Made accepts no input, does not treat the empty poll or the `done` false frame as the end of the resnapshot, and does not mark the terminal current

#### Scenario: Survival does not depend on tmux
- Given a supported server with tmux absent, and the same fixture with a user-installed tmux that Made did not configure
- When a remote shell or OpenCode loses its SSH frontend and the same tab reattaches before lease expiry
- Then both servers keep the same child PID and session, restore input modes, mouse reporting, and alternate screen without replay, and Made never probes, launches, configures, or falls back to tmux

#### Scenario: Expired or lost session is not a successful reattach
- Given detached lease expiry under an accelerated test clock, online close without an acknowledgement, or loss of the supervisor/server itself
- When expiry/close/reconnect is processed
- Then owned-session cleanup and sibling isolation are verified when reachable, unconfirmed cleanup is labeled honestly, lost/expired sessions are not silently restarted, a replacement requires explicit user action, and no tmux session is used as a substitute survivor

### Requirement: Remote security and private state (SSH-08)
SSH workspaces MUST authenticate the server and MUST NOT disable host-key verification or silently accept a changed key. Made MUST NOT upload SSH private keys or host credential files, forward an agent, or deliver authentication secrets to a remote helper/plugin/workspace; any authentication secret exchange MUST be confined to the SSH authentication method explicitly agreed under Q2. Host-key trust changes and any remote helper/plugin provisioning MUST require explicit authorization under the clarified workflow. Made MUST NOT install OpenCode, persistently modify user SSH/shell/OpenCode configuration, execute untrusted path/name/report text as commands, or probe, execute, or configure tmux. Remote app-owned resources MUST have private ownership/access and be distinguished from user workspace/config files. Workspace content MUST travel only through the selected authenticated SSH transport; integration listeners MUST remain loopback-only, with per-workspace credentials/routing enforced before any state disclosure. Remote filesystem enforcement MUST happen on the remote authority and reject traversal, outside-root/symlink escapes, non-regular preview reads and cross-server/tab access. Requests/results/queues MUST be bounded; malformed or oversized remote responses MUST fail without crashing/blocking the UI. Local keys, tokens, passwords, clipboard payloads and file content MUST NOT appear in diagnostics, copied logs or notices. SSH server content remains untrusted even after authentication.

For Made-generated operations, the client MUST constrain requests to the authorized operation surface, owning authority/tab/current root or explicit grant and MUST validate response routing, paths and bounds before applying results. The remote helper on a trusted server MUST enforce root containment, canonical/symlink validation, file-kind checks and tab-scoped authorization where the filesystem operation executes; client checks MUST NOT replace these guards. Regardless of server honesty, hostile replies MUST NOT cause local command execution, reads on the host or another authority, cross-tab state application, or unbounded processing. A compromised server/helper can forge metadata, root-containment claims and file contents, including well-formed replies; Made MUST NOT promise to detect such lies or prove that this server did not read outside its root. This limitation does not authorize the client to generate out-of-scope requests or the trusted helper to skip any guard.

Registration/authentication, provisioning, paste and lifecycle decisions are resolved by SSH-01/05/06/07; none authorizes removing the filesystem guards or the compromised-server limitation above.

#### Scenario: Unknown or changed host key
- Given a never-trusted server and then a server whose trusted key has changed
- When a test/launch/reconnect is attempted
- Then no silent trust bypass or data disclosure occurs, changed-key access is refused with actionable feedback, and first-use approval follows the explicit Q2 workflow rather than disabling verification

#### Scenario: Remote injection and containment attacks
- Given a trusted controlled server/helper and hostnames, users, paths, filenames, cwd/session reports or responses containing option-like prefixes, shell metacharacters, controls, traversal, symlink escapes, oversized frames or a foreign workspace identity
- When test/launch/read/Git/focus processes them
- Then the client generates only authorized authority/tab/root-scoped operations and validates replies, the helper rejects traversal/symlink escape and unauthorized reads at the remote filesystem boundary, data is not evaluated as commands/options, no host/other-authority read or cross-tab state application is induced, malformed operations fail safely within bounded work, and logs remain sanitized/redacted

#### Scenario: Compromised server responses do not establish filesystem attestation
- Given a hostile server/helper sends malformed or forged responses, including well-formed claims that arbitrary bytes came from an authorized file
- When Made parses and applies those responses
- Then client routing/path/bounds validation rejects detectable violations and responses cannot induce local execution, host/other-authority reads, cross-tab state application or unbounded processing; accepting syntactically valid content does not attest its provenance or prove the compromised server respected root containment

#### Scenario: Network, config and credential audit
- Given real local and remote Made integration plus existing SSH/shell/OpenCode user configurations and unrelated server processes
- When tests, remote launch, normal close, failed provisioning and SSH loss/reconnect are exercised
- Then socket inspection finds no external/wildcard integration listener, unauthenticated and cross-tab clients disclose no state, config bytes remain unchanged, no key/agent is forwarded, owned cleanup does not delete unrelated files/processes, and persisted/copied diagnostics contain no secrets or content

### Requirement: Local preservation and truthful parity evidence (SSH-09)
All existing local workflows MUST remain available and MUST NOT wait for SSH test/detection/connectivity or change local path/security semantics. Remote work MUST remain asynchronous and bounded so stalled servers cannot block input/tab switching/preview return/Explorer controls in other workspaces. Diagnostics MUST identify selected authority, cwd/root/source, shell reporting capability, transport versus OpenCode integration status, supported versions and actionable failure reasons without adding routine technical chrome. Acceptance MUST execute real local and remote workflows in a controlled environment, not only inspect source or mock a PTY. Evidence MUST record client/server OS/architecture/shell/OpenCode versions, network profile, workload, tab count, window size and backend, and distinguish automated/model/software-rendered evidence from native GUI evidence and untested combinations.

Initial remote targets MUST be Linux glibc x86_64/aarch64 (glibc 2.31 or newer) and macOS x86_64/arm64 (macOS 12 or newer), with OpenSSH server, POSIX `sh`, sha256sum or shasum, and writable private user app data. tmux MUST NOT be a target prerequisite. The companion MUST operate without a display server, GUI runtime, remote system clipboard, or tmux. Made MUST NOT install tmux or a Rust toolchain on the remote, or in a development container, to satisfy this change. bash, zsh and fish MUST support launch-local cwd reporting on these targets when installed; other shells retain honest launch-directory/latest-cwd fallback. Windows remotes and Linux musl targets are unsupported and MUST fail capability preflight explicitly. Client targets remain the existing macOS arm64/x64, Linux x64 X11/Wayland and Windows x64, using installed OpenSSH 8.4 or newer and working interactive prompt support. Remote OpenCode v2 is optional and must be detected on the remote user's executable path; no host executable is reused.

Controlled acceptance MUST use at least two allocated client/server CPU cores, 4 GiB RAM, a 10,000-file fixture, three live tabs, RTT at most 50 ms, throughput at least 20 Mbit/s and no packet loss. With prerequisites/artifacts present and authentication completed, remote terminal plus first root listing SHOULD become usable within five seconds; reported cwd/session/view changes and visible file create/delete/rename/preview-change-or-stale MUST appear within two seconds; remote Git status SHOULD update within three seconds. Authentication, explicit consent and first artifact transfer MUST be measured separately and excluded only from the five-second warm-start budget. Nonterminal RPC MUST time out within ten seconds unless it is an explicitly displayed/cancellable auth/provision/upload operation; stalled work MUST NOT block the UI. Under a 250-ms RTT/1% loss profile and a 60-second outage, correctness, stale/unavailable feedback, bounded work and same-session reattach remain required, not the low-latency remote budgets. Existing local responsiveness targets remain unchanged; native hardware evidence and all unverified combinations MUST be labeled honestly.

#### Scenario: Local regression under a stalled remote server
- Given an existing local shell/OpenCode workspace and an unreachable/stalled remote server
- When the user starts a remote test, then uses local cwd/session follow, explorer/Git/preview, image/text paste, tab switching and layout controls
- Then local behavior and targets remain unchanged, no network call blocks the UI, and diagnostics attribute failures only to the correct remote authority

#### Scenario: Controlled end-to-end parity evidence
- Given disposable controlled SSH servers matching the approved Q4 matrix, real remote shell/Git/OpenCode fixtures and the local baseline
- When the SSH-01 through SSH-09 scenarios and inherited local scenarios are executed, including actual interrupted SSH, wrong host key, bad authentication, denied provisioning, cross-tab spoofing and traversal/symlink attacks
- Then scenario-to-test evidence and measured budgets are recorded, real versus simulated/native evidence is labeled, and unsupported/untested behavior is not certified as parity

## MODIFIED Requirements

### Requirement: Security and Local-Only Integration
For local workspaces, Made MUST accept integration connections only over the loopback interface, on a randomly assigned port, authenticated by a per-workspace-instance secret. Credentials and routing MUST isolate workspace tabs: a valid client for one tab MUST NOT mutate, focus, replace the connection of, or disclose state from another tab. Made MUST reject unauthenticated or unauthorized clients before disclosing state. Made MUST NOT persistently modify user shell or OpenCode configuration files. Made MUST load integration code from an app-owned location. Made MUST restrict workspace-content reads to the owning tab's current root and paths the user explicitly selects, and MUST reject path traversal outside those roots. Explicit folder selection, the default home workspace, and validated local cwd, session, or view-directory reports authorize root changes only for their owning local tab; they MUST NOT authorize arbitrary file reads in other tabs. Read-only user configuration composition and app-owned resources remain separate from workspace-content reads. The integration bridge MUST NOT be reachable from non-loopback interfaces. SSH workspaces extend this local-only scope exclusively through the selected authenticated SSH authority as defined by SSH-02 and SSH-08; they MUST NOT require a publicly reachable bridge, accept remote reports as host paths or weaken local authentication/containment. Remote integration provisioning requires the consent and app-owned scope of SSH-06, with the trusted-helper and compromised-server limits of SSH-08.

#### Scenario: Unauthorized bridge client rejected
- Given the integration bridge is running
- When a client connects without the per-instance secret
- Then Made rejects the connection, discloses no session state, and records the rejection

#### Scenario: Non-loopback bind
- Given the integration bridge is listening for a local or SSH workspace
- When listening sockets on both machines are inspected
- Then integration endpoints are bound only to loopback and never a wildcard or external interface

#### Scenario: User config unchanged
- Given the user has an OpenCode configuration file on the selected machine
- When the user runs OpenCode in Made and the session ends
- Then the file's content is byte-for-byte unchanged and no Made-managed content was persisted into it

#### Scenario: Path traversal blocked
- Given a path outside the active root on the selected local authority or a trusted remote server/helper
- When a file request attempts traversal or a symlink escape
- Then the owning filesystem guard rejects the request and shows no content from outside the root; client-generated remote operations and response validation also enforce SSH-08, without claiming filesystem attestation for a compromised server/helper

#### Scenario: Plugin from app-owned path
- Given Made loads its integration code on the selected authority under an authorized provisioning workflow
- When the loaded location is inspected
- Then it resolves inside the app-owned installation or resource area and not inside the user's configuration directory

#### Scenario: Read-only preview enforced
- Given a file is open in the preview
- When the user attempts an edit or save action
- Then no file content is written and the preview remains read-only

### Requirement: Minimal Workspace Layout
For the existing Minimal Workspace Layout requirement and its Quiet shell workspace, Icon-only header controls and Header placement and order scenarios in `specs/made-ui-ux.md`, the persistent icon group MUST now order Sessions (when shown), `Servidores SSH`, Explorer left-to-right, keeping Explorer rightmost. This explicitly replaces only the old `Sessions then Explorer`/`only Open folder and Explorer` action inventory/order clauses; all other layout, sizing, icon-only labeling, keyboard, overflow, wordmark, preview, explorer, motion and focus guarantees remain unchanged. The app-drawn `+` MUST retain its tab-strip placement and local folder/terminal/OpenCode choices while exposing the remote choices of SSH-01, including when no local compatible TUI is detected. Existing folder-picker confirmation semantics remain unchanged for local launches.

#### Scenario: New remote actions preserve existing shell chrome
- Given a shell-only workspace and then a workspace with Sessions available at both responsive sizes
- When the header and `+` are inspected with tab overflow and no local OpenCode installed
- Then SSH management and remote choices remain reachable as defined above, local creation still works, Explorer stays rightmost, the expanded cluster is reserved in the fitting budget, and no routine technical banner or empty session strip is added
