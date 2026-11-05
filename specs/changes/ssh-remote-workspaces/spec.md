# Change Spec: ssh-remote-workspaces

Clarified delta covering the runtime/integration, UI and explorer domains, following the user's approval of Q1–Q6 recorded in proposal.md, and the later user approval to extend this open change in place with the remote OpenCode tab-parity defect fix and full remote Grok Build parity (SSH-06, SSH-10), including a consent-gated official Grok plugin install. Requirement identifiers below are stable traceability labels. Existing local guarantees and scenarios remain in force; remote guarantees below extend the experience to an explicitly selected SSH authority, not to arbitrary network paths. Engineering defaults below make those approved outcomes testable; implementation/verification results are not claimed by this contract.

Remote filesystem/content correctness and remote enforcement guarantees below assume a trusted server and correctly operating helper. This assumption MUST NOT weaken the required client-generated operation confinement, helper filesystem guards or client protection against hostile responses defined in SSH-08. Made MUST NOT claim to attest the filesystem or provenance of content returned by a compromised server/helper, even when SSH authentication and response syntax are valid. Client-side authority/tab isolation and hostile-response validation remain required regardless of server honesty.

## ADDED Requirements

### Requirement: SSH server entry and launch choices (SSH-01)
Made MUST expose a `Servidores SSH` icon-only header action immediately beside the Explorer hide/show action, with matching tooltip/accessibility name, visible keyboard focus and at least a 24×24 target. It MUST offer server registration, an explicit connection test and remote OpenCode and grok detection with success/failure feedback. The `+` flow MUST retain existing local choices and offer `Terminal SSH`, and, for a selected registered server, `OpenCode SSH` and `Grok SSH` when that program has been detected on the server; `OpenCode SSH` MUST be offered only when OpenCode has been detected on that server and `Grok SSH` only when grok has been detected there. Detection MUST NOT start the OpenCode TUI, start grok, or install either program's plugin, and it MUST use the same bounded metadata-only remote PATH rules as the existing probe, ignoring empty and relative PATH entries. Failure or absence MUST NOT prevent a supported remote terminal/explorer workspace or unrelated local workspaces from operating. A previously detected executable that disappears before launch MUST cause contextual failure, not local execution or substitution of a different server. Choosing `Grok SSH` MUST start the detected remote grok executable with the single argument `dashboard` directly as a new tab bound to that server and its selected remote directory, with no shell first and no separate folder picker, while the current tab keeps running and no plugin configuration is injected into that launch; typing `grok` manually in a supported remote shell MUST remain valid, MUST NOT be replaced by the menu, and MUST NOT be given the `dashboard` argument.

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

#### Scenario: Grok SSH row and direct dashboard launch
- Given server A has grok on its remote PATH and server B does not, and both are otherwise supported reachable servers
- When the user opens each server's remote launch choices, chooses `Grok SSH` on A, and separately types `grok` manually in A's remote shell
- Then only A offers `Grok SSH`, choosing it starts A's detected grok executable with the single argument `dashboard` directly in a new tab bound to A and its selected remote directory with no shell first or separate picker while the current tab keeps running and no plugin configuration is injected, the manual shell `grok` is not given the `dashboard` argument and remains a valid way to start Grok, and B's choices omit Grok without affecting its terminal

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

Initial helper/plugin staging MUST require explicit per-server/version consent before app-owned remote files are written; consent rejection, unsupported artifacts, missing supported-target capability, missing checksum tool, or transfer/integrity failure MUST fail clearly without mutating personal configuration or silently offering terminal-only parity. Existing pre-provisioned matching private artifacts MAY be verified and reused. Made MUST NOT install OpenCode. Made MUST NOT install, probe, execute, configure, or fall back to tmux; a server with tmux absent and a server with a user-installed tmux MUST be treated identically, and that user installation MUST NOT become a dependency, fallback, or hidden prerequisite. The frozen major-1 `Capabilities.tmux` field MUST remain for wire compatibility, MUST be encoded false, and MUST be ignored by every consumer; its presence is not permission to probe or require tmux. Optional OpenCode absence/version/integration failure MUST still leave an otherwise supported terminal/explorer usable. Remote config composition MUST read only remote user config/inline settings and inject only the remote app-owned plugin and remote loopback bridge for that launch; host provider/config/environment secrets MUST NOT be uploaded. Direct OpenCode and manual `opencode` inside a supported remote shell MUST both receive this launch-local integration environment. User shell, OpenCode, SSH, and any pre-existing tmux configuration bytes MUST remain unchanged.

Initial staging and exact reuse MUST remain unchanged: a missing version target is staged once after per-server/version consent, and an existing target whose shipped artifacts verify successfully is reused without rewriting. When an existing same-version target contains a missing, corrupt or different artifact set, Made MUST detect the mismatch, MUST NOT overwrite, delete or repair anything before a separate explicit `Update companion` approval for the current attempt, and MUST report a sanitized update-required state naming the affected version and target instead of a generic `ssh connection failed` or an app-version bump recommendation. The existing per-server/version staging consent remains valid for initial staging and exact reuse but does not authorize changed bytes; refusal or cancellation of the update approval MUST perform no remote write or delete and MUST leave the prior state actionable and retryable.

An approved same-version update MUST run through the fixed staging script: write all 11 shipped artifacts into a private temporary sibling of the existing target, verify every size, hash and required permission/executable mode there, then replace the existing target path through the narrowest practical atomic or rollback-safe swap; only after the verified swap may Made remove the temporary and replaced app-owned artifact directories so no per-build copies accumulate. Any failure before the verified swap MUST leave the original target byte-for-byte untouched; after classifying a failed update, Made MUST preserve the actionable failing phase and reason and keep the original reusable or update-required as appropriate. This update path MUST NOT add an arbitrary remote delete, rename or exec API. The companion and Grok-install commands MUST continue resolving the same current version and target path after an update, `plugins/grok-made` MUST remain present there for the consented installer, and exact reuse of the updated set MUST be verified on the next attempt.

Updating artifacts MUST NOT terminate a running companion, its supervisor, its PTY or its terminals: an already-running process keeps its own OS process lifecycle and open files, while subsequent launches resolve the updated path; reattach semantics, protocol major 1 and existing tabs MUST remain unchanged. A staging or companion-open failure before attach MUST surface a bounded Explorer unavailable/failure state with the actionable update or provisioning reason, MUST NOT display a perpetual `Loading folder…`, MUST NOT claim the Explorer is usable when it is not, and MUST NOT spawn a companion after a failed stage; optional OpenCode/Grok integration failure is not this provisioning state and still leaves a healthy terminal/filesystem usable. Routine SSH chrome MUST NOT be added for this state.

Remote OpenCode workspace-tab presentation and diagnostics MUST match the local presentation for these flows: while the owning remote OpenCode integration is connected, or a directly launched remote OpenCode process is still alive, the workspace tab MUST use the exact local `OpenCode` label, the vendored 16px `opencode.svg` leading icon and the same busy/attention/idle session dots derived from that tab's remote session map, with the local size, placement, cap, pulse, colors, tooltip-summary and accessible-name treatment, and revealed diagnostics MUST report that tab's remote integration status honestly instead of the local host integration state. Manual `opencode` in a supported remote shell and direct `OpenCode SSH` launch MUST receive this presentation identically. When the remote integration disconnects or remote OpenCode exits normally, stale session activity and dots MUST be cleared and the tab MUST return to the normal remote presentation, without losing terminal continuity and without changing server or tab scoping; the server identity MUST remain inspectable under SSH-09 without violating the local path-free tooltip rule. This paragraph changes only labeling/activity presentation and diagnostics sourcing; SSH-07 attach, resnapshot and input gating remain unchanged.

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

#### Scenario: Same-version companion mismatch requires approved update
- Given an existing app-owned same-version target with one changed, missing or corrupt artifact beside intact files, plus intact-matching and absent-target variants
- When the user attempts the remote workspace and then approves, refuses or cancels `Update companion`
- Then detection reports a sanitized version/target update-required state rather than a generic SSH failure or app-version bump advice, nothing is written or deleted before approval, refusal or cancellation writes nothing and leaves the attempt retryable, and the intact-matching variant is reused without rewriting

#### Scenario: Approved companion update is verified, swapped and reused
- Given an approved update, a companion and terminal still running from the old artifacts, and the fixed Grok-install path
- When one update succeeds, another fails before its swap, and a later attempt reuses the result
- Then all 11 artifacts are written to a private temporary sibling and verified for size, hash and permissions, the existing target is replaced atomically or rollback-safely, temporary and replaced app-owned directories are removed without per-build accumulation, the pre-swap failure leaves the original untouched with its actionable phase preserved, the running process keeps its OS lifecycle, later companion and Grok-install launches resolve the same updated version/target path, and the next attempt verifies and reuses the exact updated set

#### Scenario: Pre-attach provisioning failure bounds the Explorer
- Given a staging or companion-open failure before attach, such as an unapproved mismatch, corrupt transfer or failed update
- When the remote workspace is presented
- Then the Explorer shows a bounded unavailable/failure state with the actionable reason instead of perpetual `Loading folder…`, does not claim usability or stream a partial tree, no companion is spawned after the failed stage, and no routine SSH chrome is added

#### Scenario: Remote OpenCode tab parity with the local presentation
- Given a remote workspace whose OpenCode integration is connected with sessions working, awaiting attention and idle, and a second remote OpenCode tab launched directly whose process is alive before the bridge handshakes
- When the tab strip and revealed diagnostics render for both tabs
- Then each tab shows the exact `OpenCode` label, the vendored 16px `opencode.svg` icon and one busy/attention/idle dot per remote session with the local treatment, the directly launched tab keeps that presentation while its process is alive, diagnostics report each tab's own remote integration status rather than the host integration state, and the server identity remains inspectable without a workspace path in the tooltip

#### Scenario: Remote OpenCode exit or disconnect restores remote presentation
- Given a remote OpenCode tab showing the `OpenCode` label, icon and session dots
- When the remote integration disconnects or the remote OpenCode exits normally, and the tab later reattaches per SSH-07
- Then stale dots and session activity are cleared at the transition, the tab returns to the server-name label and terminal glyph, no pre-disconnect session state is applied after reconnect before a fresh snapshot, and the same PTY session and terminal continuity are preserved

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
SSH workspaces MUST authenticate the server and MUST NOT disable host-key verification or silently accept a changed key. Made MUST NOT upload SSH private keys or host credential files, forward an agent, or deliver authentication secrets to a remote helper/plugin/workspace; any authentication secret exchange MUST be confined to the SSH authentication method explicitly agreed under Q2. Host-key trust changes and any remote helper/plugin provisioning MUST require explicit authorization under the clarified workflow. Made MUST NOT install OpenCode, persistently modify user SSH/shell/OpenCode/Grok configuration, execute untrusted path/name/report text as commands, or probe, execute, or configure tmux. The single authorized exception is the SSH-10 consented official Grok plugin install into the remote user's existing Grok home: it is the only permitted write outside app-owned remote storage, every other Grok-home byte including `config.toml` MUST remain unchanged, `GROK_HOME` MUST NOT be redirected or set for the remote user, and remote Grok ingestion MUST be bounded reads of only the two Grok-native state files named by SSH-10, never a general remote file API or arbitrary read operation. Remote app-owned resources MUST have private ownership/access and be distinguished from user workspace/config files. Workspace content MUST travel only through the selected authenticated SSH transport; integration listeners MUST remain loopback-only, with per-workspace credentials/routing enforced before any state disclosure. Remote filesystem enforcement MUST happen on the remote authority and reject traversal, outside-root/symlink escapes, non-regular preview reads and cross-server/tab access. Requests/results/queues MUST be bounded; malformed or oversized remote responses MUST fail without crashing/blocking the UI. Local keys, tokens, passwords, clipboard payloads and file content MUST NOT appear in diagnostics, copied logs or notices. SSH server content remains untrusted even after authentication.

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

#### Scenario: Consented Grok install is the only personal-Grok-home mutation
- Given a remote Grok home with existing settings, sessions, `config.toml` and unrelated files, plus a refused-consent variant and a failed-install variant
- When the SSH-10 consented install runs, is refused, or fails
- Then the only change in the successful case is the official installer's own grok-made installation under the remote Grok home, `config.toml` and every unrelated file stay byte-identical, refusal and failure perform no write, ingestion reads only `active_sessions.json` and `made-grok-status.jsonl`, and no `GROK_HOME` redirection or other personal configuration modification occurs

### Requirement: Local preservation and truthful parity evidence (SSH-09)
All existing local workflows MUST remain available and MUST NOT wait for SSH test/detection/connectivity or change local path/security semantics. Remote work MUST remain asynchronous and bounded so stalled servers cannot block input/tab switching/preview return/Explorer controls in other workspaces. Diagnostics MUST identify selected authority, cwd/root/source, shell reporting capability, transport versus OpenCode integration status, remote Grok Build tier status under SSH-10, supported versions and actionable failure reasons without adding routine technical chrome. Acceptance MUST execute real local and remote workflows in a controlled environment, not only inspect source or mock a PTY. Evidence MUST record client/server OS/architecture/shell/OpenCode versions, network profile, workload, tab count, window size and backend, and distinguish automated/model/software-rendered evidence from native GUI evidence and untested combinations.

Initial remote targets MUST be Linux glibc x86_64/aarch64 (glibc 2.31 or newer) and macOS x86_64/arm64 (macOS 12 or newer), with OpenSSH server, POSIX `sh`, sha256sum or shasum, and writable private user app data. tmux MUST NOT be a target prerequisite. The companion MUST operate without a display server, GUI runtime, remote system clipboard, or tmux. Made MUST NOT install tmux or a Rust toolchain on the remote, or in a development container, to satisfy this change. bash, zsh and fish MUST support launch-local cwd reporting on these targets when installed; other shells retain honest launch-directory/latest-cwd fallback. Windows remotes and Linux musl targets are unsupported and MUST fail capability preflight explicitly. Client targets remain the existing macOS arm64/x64, Linux x64 X11/Wayland and Windows x64, using installed OpenSSH 8.4 or newer and working interactive prompt support. Remote OpenCode v2 is optional and must be detected on the remote user's executable path; no host executable is reused. Remote grok is likewise optional and detected only on the remote PATH under SSH-10; remote python3 is required only by the SSH-10 consented hook install, its absence MUST be a declared honest failure that leaves the inventory tier working, and Made MUST NOT install python3 on the remote or in a development container.

Controlled acceptance MUST use at least two allocated client/server CPU cores, 4 GiB RAM, a 10,000-file fixture, three live tabs, RTT at most 50 ms, throughput at least 20 Mbit/s and no packet loss. With prerequisites/artifacts present and authentication completed, remote terminal plus first root listing SHOULD become usable within five seconds; reported cwd/session/view changes and visible file create/delete/rename/preview-change-or-stale MUST appear within two seconds; remote Git status SHOULD update within three seconds. Authentication, explicit consent and first artifact transfer MUST be measured separately and excluded only from the five-second warm-start budget. Nonterminal RPC MUST time out within ten seconds unless it is an explicitly displayed/cancellable auth/provision/upload operation; stalled work MUST NOT block the UI. Under a 250-ms RTT/1% loss profile and a 60-second outage, correctness, stale/unavailable feedback, bounded work and same-session reattach remain required, not the low-latency remote budgets. Existing local responsiveness targets remain unchanged; native hardware evidence and all unverified combinations MUST be labeled honestly.

#### Scenario: Local regression under a stalled remote server
- Given an existing local shell/OpenCode workspace and an unreachable/stalled remote server
- When the user starts a remote test, then uses local cwd/session follow, explorer/Git/preview, image/text paste, tab switching and layout controls
- Then local behavior and targets remain unchanged, no network call blocks the UI, and diagnostics attribute failures only to the correct remote authority

#### Scenario: Controlled end-to-end parity evidence
- Given disposable controlled SSH servers matching the approved Q4 matrix, real remote shell/Git/OpenCode fixtures and the local baseline
- When the SSH-01 through SSH-09 scenarios and inherited local scenarios are executed, including actual interrupted SSH, wrong host key, bad authentication, denied provisioning, cross-tab spoofing and traversal/symlink attacks
- Then scenario-to-test evidence and measured budgets are recorded, real versus simulated/native evidence is labeled, and unsupported/untested behavior is not certified as parity

### Requirement: Remote Grok Build session follow and tab parity (SSH-10)
Remote Grok support MUST mirror the local Grok Build contract for the covered flows. Remote grok detection MUST be the same bounded metadata-only PATH lookup performed on the server: it MUST ignore empty and relative PATH entries, MUST NOT spawn grok, MUST NOT check a version, MUST NOT install the plugin, and MUST NOT delay terminal or explorer startup. Choosing `Grok SSH` MUST start the detected remote grok absolute path with the single argument `dashboard` as SSH-01 defines, and typing `grok` in a remote shell MUST remain valid, MUST NOT be replaced, and MUST NOT be given the `dashboard` argument. While remote OpenCode is not connected in the tab and the tab's remote target is a living grok process, or the tab's remote shell has at least one open Grok session, the tab MUST use the exact local `Grok Build` label and the official 16px `grok.svg` mark in the leading icon slot, even before any session exists; its tooltip MUST be the session summary when sessions exist and MUST omit the workspace path. When that grok is no longer running and the tab has no open Grok sessions, the tab MUST return to the server-name label and terminal glyph. When remote OpenCode is connected, the label, leading icon and dots MUST stay OpenCode's even when Grok sessions exist.

Made MUST treat the remote `$GROK_HOME/active_sessions.json` (`~/.grok/active_sessions.json` when `GROK_HOME` is unset on the server) as the inventory of sessions that remote Grok still has open, refreshed about once a second by the owning companion. A row belongs to this tab only when its process id is the tab's managed terminal process or a descendant of it, determined by live remote PID ancestry at read time; a stale, exited or foreign pid MUST NOT count. The inventory supplies session id, directory and attachment order and does not supply status. An unreadable or missing file, a truncated or malformed row, more than one matching interpretation of a malformed row, or a row outside this tab MUST NOT move the explorer or change tab presentation, and ingestion MUST stay bounded. This inventory tier requires no plugin, no consent and no remote write.

Hook ingestion requires a grok-made plugin equal to or newer than the shipped one installed in the remote Grok home: when such a plugin is present, whether the SSH-10 consented install put it there or the user installed it themselves (in which case Made performs no Grok-home write and needs no consent), Made MUST ingest the remote `$GROK_HOME/made-grok-status.jsonl` lines appended after the current attachment starts, applying the same event semantics as the local contract: session start and prompt submission move the explorer without waiting for the next inventory refresh, a session start shows idle, a prompt submission shows working, questions and permission requests show attention, finished stops and idles show idle, failures show attention, cancellations show idle, session end removes the dot, and parallel sessions keep distinct dots. Hook data is server-global, so Made MUST apply an event only to a tab whose process ancestry owns that session, and MUST NOT leak sessions, dots or explorer moves between tabs on the same server.

Remote explorer follow MUST behave identically to the local Grok contract: the newest attachment for this tab moves the explorer to that session's directory, a prompt in an older session keeps the explorer there instead of being stolen back, focusing an already-open session without a new attach or prompt MUST NOT move the explorer, and when no Grok session remains open for the tab the explorer MUST return to the remote shell-cwd fallback. Remote OpenCode keeps precedence: a connected remote OpenCode session, or its usable sessionless view directory under SSH-06, MUST keep the root and dots, and an open Grok session MUST win over the remote shell cwd only while OpenCode is not connected. Every Grok-reported directory MUST be validated against the owning remote authority like any SSH-02 report and MUST NOT be canonicalized, opened or resolved against the host filesystem. Repeated A→B→C switches between Grok sessions, OpenCode sessions and shell directories MUST land on the correct root each time.

Open/close lifecycle MUST be honest: a session appears when it first becomes owned, disappears when its process is gone or its session end is reported, and disconnect or exit clears that tab's stale Grok activity and dots before restoring the presentation above. On reconnect the client MUST discard previous-epoch Grok state and apply a fresh snapshot; events and snapshots from a previous attachment, stale revisions, or identities that do not match the owning tab MUST be discarded, and late or duplicated events MUST NOT double-apply. Corrupt, truncated or oversized ingestion MUST fail bounded and honest without breaking the terminal or filesystem.

Remote Grok activity has exactly two honest capability tiers. SSH-10 consent is Made's authorization for its own Grok-home write, running the official installer; it is not a precondition for reading hook state that an already-installed plugin supplies. When no grok-made equal to or newer than the shipped one is installed in the remote Grok home, which is the case when consent was never given or was refused so Made's install never ran, when the consented install failed, when remote python3 is missing, or when only an older copy is installed, Made MUST keep the inventory tier: owned session inventory, cwd follow, open/close lifecycle and running indication continue, while hook-driven activity/status dots and prompt-event timing are unavailable; sessions seen only in the inventory MUST show idle, and diagnostics MUST say visibly and honestly that session-status hooks are unavailable and why, without degrading the terminal or filesystem. Made runs the official `grok plugin install <staged-plugin> --trust` with the grok executable that detection found only after a separate explicit per-server/version consent and only after staging the shipped `plugins/grok-made` package into the private remote version directory and verifying its hashes under the SSH-06 staging rules; a pre-existing or user-self-installed grok-made equal to or newer than the shipped one MAY supply full hook activity without that consent because Made performs no Grok-home write in that case, and it MUST NOT be treated as permission to run the installer. That action MUST be bounded and idempotent per server and version: an installed grok-made version equal to or newer than the shipped one MUST skip the command, a missing or older install MUST run it once, and a newer installed copy MUST be left in place. Made MUST NOT run the installer silently, MUST NOT hand-edit the user's Grok `config.toml` or any other Grok-home file, MUST NOT redirect or set `GROK_HOME` for the remote user, and MUST NOT pass OpenCode's plugin environment to Grok. The installer persistently installs and enables the plugin in the remote user's existing Grok home so existing settings, sessions and authentication remain visible; because plugin hooks attach when Grok starts, they apply only to Grok sessions started after the install, and Made MUST disclose that limit. The installer MUST declare missing remote python3 as an honest failure reason; uninstall remains the user's own `grok plugin uninstall grok-made --confirm` and Made MUST NOT run it. A failed or refused install MUST still let the user start and use Grok with the inventory tier whenever no grok-made equal to or newer than the shipped one is otherwise installed, and a refused consent MUST always leave the terminal and inventory follow usable with no Made Grok-home write.

Revealed About diagnostics MUST show the same single Grok Build row treatment for a remote workspace, driven by that tab's remote detection, install and running state rather than the host's, using the realized Grok wording for shared states and a bounded honest reason for remote-only states such as inventory-only mode, a failed or absent install, or missing remote python3; the row MUST stay hidden until diagnostics are revealed and out of routine chrome, and copied diagnostics MUST include it.

#### Scenario: Remote Grok detection, direct launch and sessionless branding
- Given server A with grok on its remote PATH and no session open, and server B without grok
- When the user opens the remote launch choices and chooses `Grok SSH` on A
- Then only A offers the row, the launch starts A's grok executable with the single argument `dashboard` directly in a new tab bound to A and its selected remote directory with no shell first or separate picker, the current tab keeps running, no plugin configuration is injected, the tab is branded `Grok Build` with the official mark before any session exists, and a manual `grok` typed in A's remote shell is not given that argument

#### Scenario: Inventory-only follow without consent
- Given a remote tab that never granted the plugin consent, with unreadable-inventory, foreign-pid-row and malformed-row variants available
- When sessions start and end for this tab's process tree and the once-a-second inventory refresh runs
- Then owned sessions appear and move the explorer to their directories, foreign, stale and malformed rows change nothing, each owned session shows an idle dot, session end removes the dot and the explorer returns to the remote shell cwd, no plugin is installed or consent requested by the inventory, and the terminal and filesystem remain usable

#### Scenario: Consented install enables hook activity
- Given a supported server with grok and python3 present, no grok-made installed, and an explicit per-server/version consent granted
- When Made stages the shipped `plugins/grok-made`, verifies hashes, runs the official `grok plugin install <staged-plugin> --trust` with the detected grok executable, and the user starts a new Grok session and submits a prompt
- Then the install runs exactly once for that version without editing `config.toml`, the new session moves the explorer without waiting for the inventory refresh, its dot shows working and then attention on a question, hooks apply only to sessions started after the install and that limit is disclosed, and the dots match the local Grok Build treatment

#### Scenario: Install idempotence, refusal and failure keep honest tiers
- Given one server whose installed grok-made equals the shipped version, one with a newer installed copy, one where consent is refused, one where the install command fails, and one without remote python3
- When the consent flow is exercised on each
- Then equal and newer installs skip the command and stay in place, refusal performs no write, failed and python3-missing servers record an honest degraded reason, none of them hand-edits Grok configuration or redirects `GROK_HOME`, every variant still lets Grok start with inventory-tier follow, and no variant silently retries or claims hook readiness

#### Scenario: OpenCode precedence and Grok tab presentation
- Given a remote tab with a connected remote OpenCode session and two open Grok sessions, and a second remote tab whose OpenCode exited while two Grok sessions remain
- When Grok events and session changes arrive on both tabs
- Then the OpenCode tab keeps the `OpenCode` label, icon, dots and root, the other tab shows the `Grok Build` label, official mark and Grok dots, each Grok session's directory wins over the shell cwd only in its own tab, and no session, dot or root crosses between the tabs

#### Scenario: Two tabs on one server do not leak Grok sessions
- Given two remote tabs on one server, each with its own Grok session, and the server-global hook log receiving events for both
- When events for both sessions are appended and one tab disconnects
- Then each tab applies only events owned by its own process ancestry, including identical session identifiers disambiguated by ancestry, the disconnected tab clears its stale activity and presentation without touching the other tab, and the surviving tab keeps its own root, dots and terminal

#### Scenario: Disconnect, reconnect and stale Grok state
- Given a remote Grok tab following session B when the SSH transport drops, with hook events appended during the outage
- When the tab reconnects before lease expiry and receives a fresh snapshot
- Then the tab discards pre-disconnect activity and dots, discards events and revisions from the previous attachment, applies the fresh snapshot only after identity and revision checks pass, does not move the explorer for corrupt or truncated ingestion, and preserves terminal continuity under SSH-07

#### Scenario: Remote Grok diagnostics stay honest and hidden until revealed
- Given remote workspaces in the inventory-only, installing, ready, not-running, install-failed and python3-missing states
- When the user reveals About diagnostics and copies the report
- Then exactly one Grok Build row per workspace appears beside the OpenCode pill driven by that tab's remote state, inventory-only and failure states state the honest reason instead of claiming readiness, the row stays out of header/tab/explorer/terminal chrome, and the copied report includes the Grok and OpenCode sentences

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
