# Change Spec: ssh-claude-code-parity

## MODIFIED Requirements

### Requirement: Remote Claude Code launch and status (SSH-11)

Remote Claude Code support MUST mirror the local Claude Code contract of `specs/claude-code-session-follow.md` for these flows: `Claude Code presence`, `Claude Code launch`, `Program status protocol in the terminal`, the session-only plugin and its `Claude Code session events`, `Claude Code session ownership and liveness`, `Claude Code explorer follow and precedence` including `/cd` follow through `claude agents --json`, `Claude Code agent-view session follow`, `Claude Code tab presentation` including the hook-derived session title and status fallback, `Claude Code channel fallback`, the `Claude Code diagnostics row` and `Claude Code privacy and observation boundary`. In that mirror, "local tab" means the remote tab, the tab's terminal process is the remote managed terminal process that SSH-07's supervisor owns, process ancestry, liveness and the listing are evaluated on the owning server, and every directory is a remote path of that server; the deltas below take precedence over the local wording. Remote claude detection MUST be the same bounded metadata-only remote PATH lookup as SSH-01: it MUST ignore empty and relative PATH entries, MUST NOT spawn claude, MUST NOT check a version, MUST NOT install or load a plugin, and MUST NOT delay terminal or explorer startup.

Choosing `Claude Code SSH` MUST start the detected remote claude absolute path with no arguments directly as a new tab bound to that server and its selected remote directory, with no shell first and no separate folder picker, while the current tab keeps running; that launch MUST NOT receive OpenCode's plugin environment and MUST NOT add any command-line argument. An out-of-date companion that cannot start Claude Code MUST yield the existing update-required state from SSH-06, never a shell fallback. Typing `claude` in a supported remote shell MUST remain valid and MUST NOT be replaced by the menu.

The shipped Claude Code plugin package, with its native session-event hook built for the server's own target (the same Linux glibc or macOS operating system and CPU architecture as the companion staged for that server) and the executable mode SSH-06 requires, MUST be staged and verified as part of the SSH-06 companion artifact set in the private remote version directory, so that the hook runs on the server with no interpreter, language runtime or separately installed package. The staging of the Claude Code package MUST be authorized by the existing SSH-06 per-server/version companion staging consent and needs no separate Claude Code consent; a refused staging consent writes no app-owned remote file, exactly as SSH-06 defines. On the server, Made MUST add the staged plugin's directory to `CLAUDE_CODE_PLUGIN_DIRS` and bind that tab's private remote spool in the environment of every `Claude Code SSH` launch and of every supported remote shell tab, so that `claude` typed in a remote shell loads the plugin as locally typed `claude` does. The variable MUST be composed on the server from the remote user's own value with the local rules: every existing entry kept in its original order, Made's entry added once and not again when already present, and no user entry overwritten, reordered or dropped. The host's value of `CLAUDE_CODE_PLUGIN_DIRS`, the host plugin path and the host spool path MUST NOT be sent to the server. Directly launched remote OpenCode and Grok tabs MUST NOT receive the Claude Code entry or spool binding, and the Grok launch still injects no plugin configuration. When the staged package is missing or unreadable for a tab, including a tab attached to a supervisor started from artifacts that predate the package, Made MUST still start the shell and Claude Code, MUST NOT add an entry that points to it, MUST NOT restart or replace the running supervisor, and MUST record that Claude Code session-status hooks are unavailable for that tab. Made MUST NOT run `claude plugin install` or any `claude` subcommand on the server other than the `claude agents --json` listing below, and Claude Code started on the server outside a Made tab MUST be unaffected. Claude Code parity MUST NOT write anything on the server outside app-owned remote storage: Made MUST NOT create, edit, delete or read any file of the remote user's Claude Code configuration or state, including everything under `~/.claude/**`, and SSH-10's consented Grok install remains the single write outside app-owned storage.

The hook MUST append records to the tab's remote spool with exactly the fields, status derivation, title sanitization, 4 KiB record bound, 1 MiB spool bound and never-block, always-succeed behavior of `Claude Code session events`. Each remote spool MUST be in private app-owned remote storage, never in the workspace or in the remote user's Claude Code home, with directory mode 0700 and file mode 0600, created without following symlinks. The spool MUST be per-session runtime state owned by that tab's supervisor, removed by that supervisor when SSH-07 owned-session teardown ends the tab's managed terminal session and on detached-lease expiry (a Claude Code session ending does not remove it), and swept at helper start only when its owning supervisor is no longer running, leaving every live supervisor's spool intact; while the server is unreachable no removal MUST be claimed. The server side MUST apply only records appended after the tab's managed session started, MUST ignore malformed, truncated or oversized records without moving the explorer or changing presentation, and MUST NOT apply a spool owned by another tab, another Made client, or an earlier managed session.

Ownership and liveness MUST be determined on the server from live remote process ancestry: a session belongs to the tab only when the Claude Code process that ran its hook is that tab's managed terminal process or a descendant of it within 32 parent hops at ingestion; records from a process outside the tab, already exited at first ingestion or unresolvable MUST be ignored, which includes Claude Code started inside a tmux server or another process outside the tab's managed process tree that inherited the tab's environment. Owned processes MUST be checked for exit about once a second, each live owned process MUST have at most one current session, and separate processes MUST remain separate sessions. Hook data from different tabs on the same server, and from different Made clients attached to the same server user, MUST NOT cross tabs.

Only owned-session state MUST cross the SSH transport to the client: session identity, session directory, derived status, the sanitized session title, the recency needed to choose among sessions, and the listing-derived directory and agent-view selection results, scoped to the owning tab, connection epoch and root generation. Prompt text, the protocol `msg` field, notification message text, tool names, inputs and outputs, transcript paths or contents, environment values and any listing field other than `pid`, `cwd`, `kind` and `name` MUST NOT be stored, logged or relayed by the server side and MUST NOT be stored, logged, displayed or copied by the client. The client MUST reject relayed state for another tab, a previous epoch or a stale revision, MUST NOT double-apply late or duplicated state, and corrupt, truncated or oversized relay data MUST fail bounded without breaking the terminal or filesystem.

Remote explorer follow MUST apply the local hook-record and listing semantics, including trigger bursts, counted listings, at most one move per trigger from the owning tab, the about 5-second listing age limit and the agent-view selection by the tab's own sanitized and normalized remote terminal title. The listing MUST run on the owning server with the remote claude absolute path found by that server's detection and exactly the arguments `agents --json`, directly without a shell, with standard input closed, as the leader of its own process group, never in parallel with its previous run, killed with its process group after about 2 seconds or when every tab it serves, or the helper, is torn down, and only on the local trigger set observed for the remote tab: the tab starts owning a live session, the user submits input to that tab (Enter or Return, or pasted text containing a newline), or the tab's terminal title, as seen while a Claude Code protocol status is live in that tab, changes. The client MUST detect these triggers for its remote tab, checking input only for a submitted line, and MUST send the server a bounded trigger that carries no input bytes and no title text; one listing worker per server user MUST serve every remote tab of that client on that server, so runs for those tabs never run in parallel and one run serves them all. No listing MUST run while no trigger is pending, an idle live session MUST run none, and when claude was not detected on that server no listing runs and hook-driven follow continues. Any listing failure MUST change nothing. The published listing result kept in private app-owned remote storage MUST be removed when the last supervisor of that server user that can run the listing ends, and swept at helper start when no such supervisor is running. Every reported or listed directory MUST be validated against the owning remote authority like any SSH-02 report and MUST NOT be canonicalized, opened or resolved against the host filesystem; a relative, missing or unreadable directory MUST NOT move the explorer. Under the SSH-09 controlled profile, hook-driven moves and `/cd` follow after Enter MUST appear within the SSH-09 two-second budget instead of the local about 250 ms and about one second.

Remote explorer root precedence MUST be: a connected remote OpenCode session directory or its usable sessionless view directory under SSH-06, then the agent-view selection or the live owned remote Claude Code session directory, then open remote Grok sessions under SSH-10, then the latest validated remote shell cwd, then the remote launch directory. A Claude Code directory reported while remote OpenCode holds the root MUST be retained for when OpenCode releases it, a usable Grok directory reported while Claude Code holds the root MUST be retained for when Claude Code exits, and with more than one live owned session the most recently started, prompted or moved session wins. Claude Code presence known only from the status protocol, without a live owned session, MUST NOT move or hold the root. Remote tab presentation MUST be: while remote OpenCode is connected or its directly launched process is alive, OpenCode's label, icon and dots under SSH-06; otherwise, while Claude Code is present, the `Claude Code` label, the Claude Code mark, one dot per live owned session or exactly one protocol dot, the local status precedence (protocol, then hook-derived, then idle), the local tooltip title rule (the tab's own remote terminal title while a protocol status is live, then the latest owned hook session title, then `Claude Code`) and accessible name, with Grok Build presentation yielding to it; otherwise SSH-10's Grok Build presentation; otherwise the server-name label and terminal glyph. The tooltip MUST omit the workspace path, the server identity MUST remain inspectable under SSH-09, and the OS window title MUST stay `Made`. Repeated switches between OpenCode, Claude Code, Grok sessions and shell directories MUST land on the correct root each time.

Disconnect MUST clear that tab's remote Claude Code protocol status, dots, title and session-driven root before the SSH-07 stale presentation applies. Remote Claude Code and its hooks keep running under SSH-07, and the server side MUST keep that tab's owned-session state while its managed session lives. After reattach, the client MUST discard previous-epoch Claude Code state, MUST apply a fresh snapshot of the tab's live owned sessions, their current directories and hook-derived status only after identity and revision checks pass, MUST wait for a fresh emission rather than replay earlier protocol status, and MUST NOT replay individual records written during the outage. Lease expiry or supervisor loss MUST end the tab's sessions without a successful-reattach claim, under SSH-07.

Revealed About diagnostics MUST show the single Claude Code row for a remote workspace with the local states, pills and sentences of `Claude Code diagnostics row`, driven by that tab's remote detection, staged-package state, presence, live owned sessions and observed protocol status rather than the host's: `Checking for Claude Code.` and `Claude Code is not on PATH.` from remote detection, `Claude Code is not running.`, `Claude Code is starting.`, `Claude Code session-status hooks are unavailable.` when the staged package is unavailable for that tab or no live owned session arrived within the grace period, `Claude Code status protocol is unavailable.`, and the ready pill with `Claude Code session follow is ready.` when a live owned session exists and the status protocol is observed. The realized remote sentence `Claude Code session-status hooks are unavailable over SSH.` is retired and MUST NOT be shown; a bounded remote-only reason, such as a tab on an older supervisor, MAY appear only in the sectioned diagnostics detail, not in the row sentence. No state mentions python3 or another interpreter, the row MUST stay hidden until diagnostics are revealed and out of routine chrome, and copied diagnostics MUST include it.

#### Scenario: Claude Code SSH row and direct launch load the staged plugin

- Given server A has claude on its remote PATH and a verified staged companion set including the Claude Code package, server B does not have claude, both supported and reachable, and A's remote user has no `CLAUDE_CODE_PLUGIN_DIRS`
- When the user opens each server's remote launch choices and chooses `Claude Code SSH` on A
- Then only A offers `Claude Code SSH`, A's detected claude starts with no arguments directly in a new tab bound to A and its selected remote directory with no shell first or separate picker while the current tab keeps running, that Claude Code loads the server-staged plugin for that session only with a spool bound to that tab, no host path or host variable value is sent, and B's choices omit Claude Code without affecting its terminal

#### Scenario: Shell-typed remote claude and existing plugin directories

- Given a supported remote shell tab on a server whose remote user's `CLAUDE_CODE_PLUGIN_DIRS` already holds two user entries
- When the user types `claude` in that remote shell
- Then Claude Code starts normally, the variable holds the two user entries in their original order plus Made's single server-staged entry, both the user's plugins and Made's plugin load, and no file of the remote user's Claude Code configuration changes

#### Scenario: Remote hook follow and hook-derived status without the protocol

- Given a remote Claude Code session with Made's staged plugin loaded and the status protocol unavailable, for example with `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1`
- When its session starts in another remote directory, the user submits a prompt, Claude Code requests a permission, the turn stops, and the user renames the session and submits a prompt
- Then the explorer moves to the session's remote directory within the SSH-09 budget, the single dot is idle, busy, attention and idle in turn, the tooltip begins with the renamed title, and the explorer, dots and title of no other tab change

#### Scenario: Remote `/cd` follow through the server listing

- Given a remote tab following a live owned Claude Code session on a server where claude was detected
- When the user types `/cd` to another readable remote directory, presses Enter and submits no prompt
- Then the listing runs on that server, the explorer switches to that remote directory within the SSH-09 budget, the next hook record carrying the same directory leaves it there, and an idle session with no trigger runs no listing

#### Scenario: Remote agent-view session follow

- Given a remote tab with a live owned session, a live protocol status, and a server listing with exactly one background entry named `Spec task` in a remote directory
- When the user opens that session from Claude Code's agent view and the remote terminal title becomes `✳ Spec task`, and later the title changes to `✳ Claude Code`
- Then the explorer follows the background entry's remote directory while the title is unchanged and returns to the owned session's directory after the title change, and duplicate names, no live protocol status or no live owned session select nothing

#### Scenario: Remote precedence matches local precedence

- Given a remote tab with open Grok sessions under SSH-10, and a second remote tab with remote OpenCode connected
- When the user starts Claude Code in each tab and later exits it in the first
- Then the first tab shows `Claude Code`, its mark and only Claude Code dots with the root on the Claude Code session directory while it is present, then returns to `Grok Build`, the Grok mark, Grok dots and the Grok session's directory, and the second tab keeps OpenCode's label, icon, dots and root throughout and moves to the Claude Code session directory if OpenCode disconnects while that session is live

#### Scenario: Foreign, tmux and other-tab processes are not claimed

- Given two remote tabs on one server, a Claude Code started inside a tmux server launched from the first tab's shell, and a Claude Code in a separate SSH login of the same server user with a copied spool binding
- When each Claude Code fires hook events
- Then each tab applies only sessions owned by its own managed process ancestry, the tmux and foreign sessions are claimed by no tab, and no label, dot, title or explorer root crosses between the tabs

#### Scenario: Remote disconnect and reconnect

- Given a remote tab following Claude Code session B with a busy dot when the SSH transport drops, while the remote Claude Code keeps running and its hooks report a prompt and a shell-command `cd` directory change during the outage
- When the tab reconnects before lease expiry
- Then the disconnect clears the Claude Code status, dots, title and session-driven root, no stale dot or previous-epoch state is applied after reconnect, the fresh snapshot after the completing resnapshot frame restores the session's current directory and hook-derived status, protocol status waits for a fresh emission, no outage record is replayed individually, and the same PTY session is preserved

#### Scenario: Out-of-date companion and older supervisor

- Given a server whose staged companion predates the Claude Code package, and separately a tab still attached to a supervisor of an older Made version (a same-version supervisor started from older artifacts is an accepted residual, relying on a version bump at every release; see the proposal decision index)
- When the user chooses `Claude Code SSH` on the first server, and types `claude` in the older tab's shell
- Then the first shows the existing SSH-06 update-required state without starting a shell or other program and writes nothing for Claude Code before an approved update, any approved update follows the SSH-06 staging rules, and the older tab keeps running unchanged with Claude Code started without a Made plugin entry and its diagnostics row reporting session-status hooks unavailable

#### Scenario: Staging consent declined

- Given a server with claude on its remote PATH and no staged companion
- When the user refuses the SSH-06 companion staging consent, which covers the Claude Code package
- Then no app-owned remote file is written, the attempt fails clearly and stays retryable as SSH-06 defines, and no Claude Code plugin, spool or hook exists on the server

#### Scenario: Remote hooks disabled keep protocol status only

- Given remote claude 2.1.295 or later started with `--bare` in a supported remote shell tab outside tmux
- When Claude Code sends the capability probe, emits `working`, `blocked` and `done`, sets its window title, and later exits
- Then Made answers the probe through that remote terminal, the tab shows `Claude Code`, the Claude Code mark and one dot that is busy, attention and idle in turn, the tooltip starts with the sanitized remote title, the explorer stays on the remote root chosen without Claude Code, the diagnostics row reports session-status hooks unavailable, no other tab changes, and after exit the tab returns to the server-name label and terminal glyph

#### Scenario: Claude missing or undetected on the server

- Given server A without claude, and server B where the bounded detection did not find claude but the user's interactive remote shell PATH starts a claude
- When the user opens launch choices and works in remote shells on both
- Then neither offers `Claude Code SSH`, A's diagnostics row says `Claude Code is not on PATH.`, B's shell-typed Claude Code still loads the staged plugin and follows hook records, no listing runs on B, and no terminal or explorer behavior breaks on either

#### Scenario: Remote listing failures change nothing

- Given a remote tab following a live owned session, and a server listing that exits non-zero, times out, prints malformed output or has no entry for the session's pid
- When hook records with new remote directories arrive
- Then the explorer follows the hook records exactly as without the listing and no listing failure moves the explorer or changes presentation

#### Scenario: Remote directories are validated on the server

- Given a remote session whose hook records or listing report a relative path, a missing remote directory, an unreadable remote directory, and a path that exists only on the host
- When the reports are ingested
- Then none moves the explorer, nothing is opened or resolved on the host, and nothing in the report is executed

#### Scenario: Remote privacy markers never retained or relayed

- Given remote Claude Code sessions whose prompts, permission notifications, protocol `msg` fields and tool data contain unique marker strings
- When every remote spool, relay frame, server-side log, client log, diagnostics row and copied diagnostics report is searched
- Then no marker string is found, and the remote user's Claude Code configuration and state files are byte-for-byte unchanged and were not read by Made

#### Scenario: Staged hook matches each server's target

- Given one Linux glibc aarch64 server and one macOS x86_64 server, both supported, with no python3 or other interpreter on PATH
- When Made stages the companion set on each and a remote Claude Code session starts there
- Then each staged Claude Code package carries the hook built for that server's operating system and architecture, verified by hash and executable mode under SSH-06, and the hook writes owned records that move the explorer and drive the dot

#### Scenario: Remote Claude Code diagnostics

- Given remote workspaces where claude is undetected, not running, started less than about 10 seconds ago with no live owned session, running with `--bare` for longer than the grace period, running with a live owned session and no session-level emission for longer than the grace period, and running with a live owned session and observed emissions
- When the user reveals About diagnostics and copies the report
- Then each row is driven by that tab's remote state with `Claude Code is not on PATH.`, `Claude Code is not running.`, `Claude Code is starting.`, `Claude Code session-status hooks are unavailable.`, `Claude Code status protocol is unavailable.` and the ready pill with `Claude Code session follow is ready.` respectively, no row mentions an interpreter, the rows stay out of routine chrome, and the copied report includes them

### Requirement: Remote OpenCode session and cwd follow (SSH-06)

Supported remote integration MUST expose the current optional session-map data/actions, activity indicators and focus correlation within the owning remote OpenCode instance. Root precedence MUST remain active OpenCode session directory, then usable connected sessionless view directory when tabs are enabled, then latest validated remote shell cwd, then remote launch directory. Shell cwd MUST continue to be recorded while OpenCode takes precedence. Integration disconnect/normal exit/unsupported state/tabs disabled/new epoch MUST clear view-directory state and resume remote shell fallback without using the host cwd. Worktree/session changes MUST not restart terminal processes or manage worktrees. Missing or incompatible OpenCode/plugin MUST be distinguished from loss of SSH transport; optional integration failure MUST NOT by itself disable a healthy remote filesystem or terminal. Safe shell hooks MUST preserve remote user startup files and existing hooks; unsupported reporting MUST be reported honestly, never replaced by prompt scraping or injected commands in a running shell.

Initial helper/plugin staging, including the Claude Code plugin package and hook of SSH-11, MUST require explicit per-server/version consent before app-owned remote files are written; consent rejection, unsupported artifacts, missing supported-target capability, missing checksum tool, or transfer/integrity failure MUST fail clearly without mutating personal configuration or silently offering terminal-only parity. Existing pre-provisioned matching private artifacts MAY be verified and reused. Made MUST NOT install OpenCode. Made MUST NOT install, probe, execute, configure, or fall back to tmux; a server with tmux absent and a server with a user-installed tmux MUST be treated identically, and that user installation MUST NOT become a dependency, fallback, or hidden prerequisite. The frozen major-1 `Capabilities.tmux` field MUST remain for wire compatibility, MUST be encoded false, and MUST be ignored by every consumer; its presence is not permission to probe or require tmux. Optional OpenCode absence/version/integration failure MUST still leave an otherwise supported terminal/explorer usable. Remote config composition MUST read only remote user config/inline settings and inject only the remote app-owned plugin and remote loopback bridge for that launch; host provider/config/environment secrets MUST NOT be uploaded. Direct OpenCode and manual `opencode` inside a supported remote shell MUST both receive this launch-local integration environment. User shell, OpenCode, SSH, and any pre-existing tmux configuration bytes MUST remain unchanged.

Initial staging and exact reuse MUST remain unchanged: a missing version target is staged once after per-server/version consent, and an existing target whose shipped artifacts verify successfully is reused without rewriting. When an existing same-version target contains a missing, corrupt or different artifact set, Made MUST detect the mismatch, MUST NOT overwrite, delete or repair anything before a separate explicit `Update companion` approval for the current attempt, and MUST report a sanitized update-required state naming the affected version and target instead of a generic `ssh connection failed` or an app-version bump recommendation. The existing per-server/version staging consent remains valid for initial staging and exact reuse but does not authorize changed bytes; refusal or cancellation of the update approval MUST perform no remote write or delete and MUST leave the prior state actionable and retryable.

An approved same-version update MUST run through the fixed staging script: write all 14 shipped artifacts (the companion, the five OpenCode plugin files, the grok-made `plugin.json`, `README.md`, `hooks/hooks.json` and native session-status hook executable built for that target, and the claude-code-made `.claude-plugin/plugin.json`, `README.md`, `hooks/hooks.json` and native session-event hook executable built for that target) into a private temporary sibling of the existing target, verify every size, hash and required permission/executable mode there, then replace the existing target path through the narrowest practical atomic or rollback-safe swap; only after the verified swap may Made remove the temporary and replaced app-owned artifact directories so no per-build copies accumulate. Any failure before the verified swap MUST leave the original target byte-for-byte untouched; after classifying a failed update, Made MUST preserve the actionable failing phase and reason and keep the original reusable or update-required as appropriate. This update path MUST NOT add an arbitrary remote delete, rename or exec API. The companion and Grok-install commands MUST continue resolving the same current version and target path after an update, `plugins/grok-made` MUST remain present there for the consented installer, `plugins/claude-code-made` MUST remain present there for the SSH-11 launch environment, and exact reuse of the updated set MUST be verified on the next attempt.

Updating artifacts MUST NOT terminate a running companion, its supervisor, its PTY or its terminals: an already-running process keeps its own OS process lifecycle and open files, while subsequent launches resolve the updated path; reattach semantics, protocol major 1 and existing tabs MUST remain unchanged. A staging or companion-open failure before attach MUST surface a bounded Explorer unavailable/failure state with the actionable update or provisioning reason, MUST NOT display a perpetual `Loading folder…`, MUST NOT claim the Explorer is usable when it is not, and MUST NOT spawn a companion after a failed stage; optional OpenCode/Grok/Claude Code integration failure is not this provisioning state and still leaves a healthy terminal/filesystem usable. Routine SSH chrome MUST NOT be added for this state.

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
- Then all 14 artifacts are written to a private temporary sibling and verified for size, hash and permissions, the existing target is replaced atomically or rollback-safely, temporary and replaced app-owned directories are removed without per-build accumulation, the pre-swap failure leaves the original untouched with its actionable phase preserved, the running process keeps its OS lifecycle, later companion, Grok-install and Claude Code launches resolve the same updated version/target path, and the next attempt verifies and reuses the exact updated set

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

### Requirement: Remote security and private state (SSH-08)

SSH workspaces MUST authenticate the server and MUST NOT disable host-key verification or silently accept a changed key. Made MUST NOT upload SSH private keys or host credential files, forward an agent, or deliver authentication secrets to a remote helper/plugin/workspace; any authentication secret exchange MUST be confined to the SSH authentication method explicitly agreed under Q2. Host-key trust changes and any remote helper/plugin provisioning MUST require explicit authorization under the clarified workflow. Made MUST NOT install OpenCode or Claude Code, persistently modify user SSH/shell/OpenCode/Grok/Claude Code configuration, execute untrusted path/name/report text as commands, or probe, execute, or configure tmux. The single authorized exception is the SSH-10 consented official Grok plugin install into the remote user's existing Grok home: it is the only permitted write outside app-owned remote storage, every other Grok-home byte including `config.toml` MUST remain unchanged, `GROK_HOME` MUST NOT be redirected or set for the remote user, and remote Grok ingestion MUST be bounded reads of only the two Grok-native state files named by SSH-10, never a general remote file API or arbitrary read operation. Remote Claude Code ingestion under SSH-11 MUST be bounded reads of only the app-owned per-tab spools and the `pid`, `cwd`, `kind` and `name` fields of the remote `claude agents --json` listing; the `Claude Code SSH` launch and that listing are the only remote Claude Code commands Made runs, and Made MUST NOT read, create, edit or delete any file of the remote user's Claude Code configuration or state, including everything under `~/.claude/**`. Remote app-owned resources MUST have private ownership/access and be distinguished from user workspace/config files. Workspace content MUST travel only through the selected authenticated SSH transport; integration listeners MUST remain loopback-only, with per-workspace credentials/routing enforced before any state disclosure. Remote filesystem enforcement MUST happen on the remote authority and reject traversal, outside-root/symlink escapes, non-regular preview reads and cross-server/tab access. Requests/results/queues MUST be bounded; malformed or oversized remote responses MUST fail without crashing/blocking the UI. Local keys, tokens, passwords, clipboard payloads and file content MUST NOT appear in diagnostics, copied logs or notices. SSH server content remains untrusted even after authentication.

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

#### Scenario: Claude Code parity touches only app-owned remote state

- Given a remote user with Claude Code settings, installed plugins, hooks, sessions and an existing `CLAUDE_CODE_PLUGIN_DIRS`, plus a staged companion set with the Claude Code package
- When remote Claude Code runs in remote shell and `Claude Code SSH` tabs, the listing runs, tabs close and the client reconnects
- Then Made wrote only inside app-owned remote storage, read no file under the remote user's Claude Code home, ran no `claude` subcommand other than `claude agents --json`, persisted no change to the remote user's environment or shell startup files, and relayed no prompt, `msg`, tool or transcript content

### Requirement: Local preservation and truthful parity evidence (SSH-09)

All existing local workflows MUST remain available and MUST NOT wait for SSH test/detection/connectivity or change local path/security semantics. Remote work MUST remain asynchronous and bounded so stalled servers cannot block input/tab switching/preview return/Explorer controls in other workspaces. Diagnostics MUST identify selected authority, cwd/root/source, shell reporting capability, transport versus OpenCode integration status, remote Grok Build tier status under SSH-10, remote Claude Code status under SSH-11, supported versions and actionable failure reasons without adding routine technical chrome. Acceptance MUST execute real local and remote workflows in a controlled environment, not only inspect source or mock a PTY. Evidence MUST record client/server OS/architecture/shell/OpenCode versions, network profile, workload, tab count, window size and backend, and distinguish automated/model/software-rendered evidence from native GUI evidence and untested combinations.

A scoped implementation archive of this change folder MAY proceed with environment combinations explicitly labeled `UNVERIFIED` and accepted by the requester, such as the unexecuted native reference slice and full client/server matrix; this records an honest evidence limit, does not satisfy the acceptance above, and MUST NOT remove or downgrade any product guarantee. The bounded Linux loopback CI smoke job is regression evidence for the protocol and harness surface only, not a substitute for the approved client/server matrix or native GUI evidence, and full release acceptance MUST remain blocked until the `UNVERIFIED` combinations execute and their results are recorded.

Initial remote targets MUST be Linux glibc x86_64/aarch64 (glibc 2.31 or newer) and macOS x86_64/arm64 (macOS 12 or newer), with OpenSSH server, POSIX `sh`, sha256sum or shasum, and writable private user app data. tmux MUST NOT be a target prerequisite. The companion MUST operate without a display server, GUI runtime, remote system clipboard, or tmux. Made MUST NOT install tmux or a Rust toolchain on the remote, or in a development container, to satisfy this change. bash, zsh and fish MUST support launch-local cwd reporting on these targets when installed; other shells retain honest launch-directory/latest-cwd fallback. Windows remotes and Linux musl targets are unsupported and MUST fail capability preflight explicitly. Client targets remain the existing macOS arm64/x64, Linux x64 X11/Wayland and Windows x64, using installed OpenSSH 8.4 or newer and working interactive prompt support. Remote OpenCode v2 is optional and must be detected on the remote user's executable path; no host executable is reused. Remote grok is likewise optional and detected only on the remote PATH under SSH-10, and remote claude is likewise optional and detected only on the remote PATH under SSH-11. Remote python3, or any other interpreter or language runtime, MUST NOT be a prerequisite for any SSH-10 Grok Build tier, for the consented hook install, for hook activity, or for SSH-11 Claude Code hook activity; its presence or absence MUST NOT change any tier, install outcome or diagnostics state, and Made MUST NOT install python3 or any other interpreter on the remote or in a development container.

Controlled acceptance MUST use at least two allocated client/server CPU cores, 4 GiB RAM, a 10,000-file fixture, three live tabs, RTT at most 50 ms, throughput at least 20 Mbit/s and no packet loss. With prerequisites/artifacts present and authentication completed, remote terminal plus first root listing SHOULD become usable within five seconds; reported cwd/session/view changes and visible file create/delete/rename/preview-change-or-stale MUST appear within two seconds; remote Git status SHOULD update within three seconds. Authentication, explicit consent and first artifact transfer MUST be measured separately and excluded only from the five-second warm-start budget. Nonterminal RPC MUST time out within ten seconds unless it is an explicitly displayed/cancellable auth/provision/upload operation; stalled work MUST NOT block the UI. Under a 250-ms RTT/1% loss profile and a 60-second outage, correctness, stale/unavailable feedback, bounded work and same-session reattach remain required, not the low-latency remote budgets. Existing local responsiveness targets remain unchanged; native hardware evidence and all unverified combinations MUST be labeled honestly.

#### Scenario: Local regression under a stalled remote server

- Given an existing local shell/OpenCode workspace and an unreachable/stalled remote server
- When the user starts a remote test, then uses local cwd/session follow, explorer/Git/preview, image/text paste, tab switching and layout controls
- Then local behavior and targets remain unchanged, no network call blocks the UI, and diagnostics attribute failures only to the correct remote authority

#### Scenario: Controlled end-to-end parity evidence

- Given disposable controlled SSH servers matching the approved Q4 matrix, real remote shell/Git/OpenCode fixtures and the local baseline
- When the SSH-01 through SSH-11 scenarios and inherited local scenarios are executed, including actual interrupted SSH, wrong host key, bad authentication, denied provisioning, cross-tab spoofing and traversal/symlink attacks
- Then scenario-to-test evidence and measured budgets are recorded, real versus simulated/native evidence is labeled, and unsupported/untested behavior is not certified as parity

### Requirement: Remote Grok Build session follow and tab parity (SSH-10)

Remote Grok support MUST mirror the local Grok Build contract for the covered flows. Remote grok detection MUST be the same bounded metadata-only PATH lookup performed on the server: it MUST ignore empty and relative PATH entries, MUST NOT spawn grok, MUST NOT check a version, MUST NOT install the plugin, and MUST NOT delay terminal or explorer startup. Choosing `Grok SSH` MUST start the detected remote grok absolute path with the single argument `dashboard` as SSH-01 defines, and typing `grok` in a remote shell MUST remain valid, MUST NOT be replaced, and MUST NOT be given the `dashboard` argument. While remote OpenCode is not connected in the tab, Claude Code is not present in the tab under SSH-11, and the tab's remote target is a living grok process, or the tab's remote shell has at least one open Grok session, the tab MUST use the exact local `Grok Build` label and the official 16px `grok.svg` mark in the leading icon slot, even before any session exists; its tooltip MUST be the session summary when sessions exist and MUST omit the workspace path. When that grok is no longer running and the tab has no open Grok sessions, the tab MUST return to the server-name label and terminal glyph, unless Claude Code is present, in which case SSH-11 owns them. When remote OpenCode is connected, the label, leading icon and dots MUST stay OpenCode's even when Grok sessions exist. When remote OpenCode is not connected and Claude Code is present under SSH-11, the label, leading icon and dots MUST be Claude Code's even when Grok sessions exist.

Made MUST treat the remote `$GROK_HOME/active_sessions.json` (`~/.grok/active_sessions.json` when `GROK_HOME` is unset on the server) as the inventory of sessions that remote Grok still has open, refreshed about once a second by the owning companion. A row belongs to this tab only when its process id is the tab's managed terminal process or a descendant of it, determined by live remote PID ancestry at read time; a stale, exited or foreign pid MUST NOT count. The inventory supplies session id, directory and attachment order and does not supply status. An unreadable or missing file, a truncated or malformed row, more than one matching interpretation of a malformed row, or a row outside this tab MUST NOT move the explorer or change tab presentation, and ingestion MUST stay bounded. This inventory tier requires no plugin, no consent and no remote write.

Hook ingestion requires a grok-made plugin equal to or newer than the shipped one installed in the remote Grok home: when such a plugin is present, whether the SSH-10 consented install put it there or the user installed it themselves (in which case Made performs no Grok-home write and needs no consent), Made MUST ingest the remote `$GROK_HOME/made-grok-status.jsonl` lines appended after the current attachment starts, applying the same event semantics as the local contract: session start and prompt submission move the explorer without waiting for the next inventory refresh, a session start shows idle, a prompt submission shows working, questions and permission requests show attention, finished stops and idles show idle, failures show attention, cancellations show idle, session end removes the dot, and parallel sessions keep distinct dots. Hook data is server-global, so Made MUST apply an event only to a tab whose process ancestry owns that session, and MUST NOT leak sessions, dots or explorer moves between tabs on the same server.

Remote explorer follow MUST behave identically to the local Grok contract: the newest attachment for this tab moves the explorer to that session's directory, a prompt in an older session keeps the explorer there instead of being stolen back, focusing an already-open session without a new attach or prompt MUST NOT move the explorer, and when no Grok session remains open for the tab and no OpenCode or Claude Code source applies the explorer MUST return to the remote shell-cwd fallback. Remote OpenCode keeps precedence: a connected remote OpenCode session, or its usable sessionless view directory under SSH-06, MUST keep the root and dots. A live owned remote Claude Code session directory, or its agent-view selection, under SSH-11 MUST also keep the root over Grok, and a usable Grok directory reported while Claude Code holds the root MUST be retained for when Claude Code exits. An open Grok session MUST win over the remote shell cwd only while OpenCode is not connected and the tab has no live owned remote Claude Code session. Every Grok-reported directory MUST be validated against the owning remote authority like any SSH-02 report and MUST NOT be canonicalized, opened or resolved against the host filesystem. Repeated A→B→C switches between Grok sessions, Claude Code sessions, OpenCode sessions and shell directories MUST land on the correct root each time.

Open/close lifecycle MUST be honest: a session appears when it first becomes owned, disappears when its process is gone or its session end is reported, and disconnect or exit clears that tab's stale Grok activity and dots before restoring the presentation above. On reconnect the client MUST discard previous-epoch Grok state and apply a fresh snapshot; events and snapshots from a previous attachment, stale revisions, or identities that do not match the owning tab MUST be discarded, and late or duplicated events MUST NOT double-apply. Corrupt, truncated or oversized ingestion MUST fail bounded and honest without breaking the terminal or filesystem.

Remote Grok activity has exactly two honest capability tiers. SSH-10 consent is Made's authorization for its own Grok-home write, running the official installer; it is not a precondition for reading hook state that an already-installed plugin supplies. When no grok-made equal to or newer than the shipped one is installed in the remote Grok home, which is the case when consent was never given or was refused so Made's install never ran, when the consented install failed, or when only an older copy is installed, Made MUST keep the inventory tier: owned session inventory, cwd follow, open/close lifecycle and running indication continue, while hook-driven activity/status dots and prompt-event timing are unavailable; sessions seen only in the inventory MUST show idle, and diagnostics MUST say visibly and honestly that session-status hooks are unavailable and why, without degrading the terminal or filesystem. Made runs the official `grok plugin install <staged-plugin> --trust` with the grok executable that detection found only after a separate explicit per-server/version consent and only after staging the shipped `plugins/grok-made` package into the private remote version directory and verifying its hashes under the SSH-06 staging rules; the staged package MUST contain the native session-status hook built for the server's own target, the same Linux glibc or macOS operating system and CPU architecture as the companion staged for that server, with the executable mode SSH-06 requires, so the installed hook runs there with no interpreter or runtime prerequisite. A pre-existing or user-self-installed grok-made equal to or newer than the shipped one MAY supply full hook activity without that consent because Made performs no Grok-home write in that case, and it MUST NOT be treated as permission to run the installer. That action MUST be bounded and idempotent per server and version: an installed grok-made version equal to or newer than the shipped one MUST skip the command, a missing or older install MUST run it once, and a newer installed copy MUST be left in place. Made MUST NOT run the installer silently, MUST NOT hand-edit the user's Grok `config.toml` or any other Grok-home file, MUST NOT redirect or set `GROK_HOME` for the remote user, and MUST NOT pass OpenCode's plugin environment to Grok. The installer persistently installs and enables the plugin in the remote user's existing Grok home so existing settings, sessions and authentication remain visible; because plugin hooks attach when Grok starts, they apply only to Grok sessions started after the install, and Made MUST disclose that limit. The presence or absence of python3 or any other interpreter on the server MUST NOT gate the hook tier, the consent offer or the install, and MUST NOT be reported as a failure reason; uninstall remains the user's own `grok plugin uninstall grok-made --confirm` and Made MUST NOT run it. A failed or refused install MUST still let the user start and use Grok with the inventory tier whenever no grok-made equal to or newer than the shipped one is otherwise installed, and a refused consent MUST always leave the terminal and inventory follow usable with no Made Grok-home write.

Revealed About diagnostics MUST show the same single Grok Build row treatment for a remote workspace, driven by that tab's remote detection, install and running state rather than the host's, using the realized Grok wording for shared states and a bounded honest reason for remote-only states such as inventory-only mode or a failed or absent install; the row MUST stay hidden until diagnostics are revealed and out of routine chrome, and copied diagnostics MUST include it.

#### Scenario: Remote Grok detection, direct launch and sessionless branding

- Given server A with grok on its remote PATH and no session open, and server B without grok
- When the user opens the remote launch choices and chooses `Grok SSH` on A
- Then only A offers the row, the launch starts A's grok executable with the single argument `dashboard` directly in a new tab bound to A and its selected remote directory with no shell first or separate picker, the current tab keeps running, no plugin configuration is injected, the tab is branded `Grok Build` with the official mark before any session exists, and a manual `grok` typed in A's remote shell is not given that argument

#### Scenario: Inventory-only follow without consent

- Given a remote tab that never granted the plugin consent, with unreadable-inventory, foreign-pid-row and malformed-row variants available
- When sessions start and end for this tab's process tree and the once-a-second inventory refresh runs
- Then owned sessions appear and move the explorer to their directories, foreign, stale and malformed rows change nothing, each owned session shows an idle dot, session end removes the dot and the explorer returns to the remote shell cwd, no plugin is installed or consent requested by the inventory, and the terminal and filesystem remain usable

#### Scenario: Consented install enables hook activity

- Given a supported server with grok present, no python3 or other interpreter installed or on its PATH, no grok-made installed, and an explicit per-server/version consent granted
- When Made stages the shipped `plugins/grok-made`, verifies hashes, runs the official `grok plugin install <staged-plugin> --trust` with the detected grok executable, and the user starts a new Grok session and submits a prompt
- Then the install runs exactly once for that version without editing `config.toml`, the new session moves the explorer without waiting for the inventory refresh, its dot shows working and then attention on a question, hooks apply only to sessions started after the install and that limit is disclosed, and the dots match the local Grok Build treatment

#### Scenario: Staged hook matches each server's target

- Given one Linux glibc aarch64 server and one macOS x86_64 server, both supported, with grok present and consent granted
- When Made stages the shipped package on each and runs the consented install
- Then each staged and installed grok-made carries the hook built for that server's operating system and architecture, verified by hash and executable mode under SSH-06, and a new Grok session on each server produces hook lines that drive working and attention dots

#### Scenario: Install idempotence, refusal and failure keep honest tiers

- Given one server whose installed grok-made equals the shipped version, one with a newer installed copy, one where consent is refused, and one where the install command fails
- When the consent flow is exercised on each
- Then equal and newer installs skip the command and stay in place, refusal performs no write, failed servers record an honest degraded reason, none of them hand-edits Grok configuration or redirects `GROK_HOME`, every variant still lets Grok start with inventory-tier follow, and no variant silently retries or claims hook readiness

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

- Given remote workspaces in the inventory-only, installing, ready, not-running and install-failed states
- When the user reveals About diagnostics and copies the report
- Then exactly one Grok Build row per workspace appears beside the OpenCode pill driven by that tab's remote state, inventory-only and failure states state the honest reason instead of claiming readiness, no state mentions python3, the row stays out of header/tab/explorer/terminal chrome, and the copied report includes the Grok and OpenCode sentences

### Requirement: Session-only Claude Code plugin

Made MUST ship a Claude Code plugin, including its native hook, in the installed layout of every local client target, resolved from Made's installation root, never downloaded at runtime, and needing nothing beyond what Claude Code itself provides; where the OS uses an executable permission, the shipped hook MUST keep it in the installed layout and in development runs. Made MUST load that plugin only for Claude Code started inside Made's local tabs, through Claude Code's session plugin-directory environment variable `CLAUDE_CODE_PLUGIN_DIRS`, by adding the shipped plugin's directory to that variable in the environment of every local shell tab and every directly launched Claude Code tab. When the variable already has a value in Made's environment, Made MUST keep every existing entry in its original order and add its own entry, MUST NOT add its entry a second time when it is already present, and MUST NOT overwrite, reorder or drop any user entry. Made MUST NOT add the variable to directly launched OpenCode or Grok tabs or to any process it does not start for a local tab, and MUST NOT add the shipped local plugin's directory to any SSH tab or remote process; Claude Code in an SSH tab loads only the plugin staged on that server, as SSH-11 in `specs/ssh-remote-workspaces.md` defines. The plugin MUST load in Claude Code without a plugin validation error or warning, and MUST NOT disable, replace or reorder the user's own plugins or hooks. Made MUST NOT create, edit or delete any file in the user's Claude Code configuration, including settings, plugin registry, marketplaces, hooks and statusline, MUST NOT run `claude plugin install` or any `claude` subcommand other than the read-only `claude agents --json` listing of `Claude Code explorer follow and precedence`, and MUST NOT pass Claude Code command-line arguments. Claude Code started outside Made MUST be unaffected. If the shipped plugin is missing or unreadable, Made MUST still start shells and Claude Code, MUST NOT add an entry that points to it, and MUST record that Claude Code session-status hooks are unavailable.

#### Scenario: Shell tab loads the plugin for one session

- Given a local shell tab and no `CLAUDE_CODE_PLUGIN_DIRS` in Made's environment
- When the user types `claude` in that shell
- Then Claude Code loads Made's shipped plugin for that session only, and after Claude Code exits the user's Claude Code configuration files are byte-for-byte unchanged

#### Scenario: Pre-existing plugin directories are preserved

- Given Made was started with `CLAUDE_CODE_PLUGIN_DIRS` already holding two user plugin directories
- When the user opens a shell tab, and separately chooses the Claude Code row
- Then in both environments the variable holds the two user entries in their original order plus Made's single entry, and Claude Code loads the user's plugins and Made's plugin together

#### Scenario: Shipped hook is executable

- Given Made installed from its package on macOS or Linux, and a development run
- When the shipped plugin's hook is inspected and Claude Code fires a hook in a Made tab
- Then the hook carries the executable permission and runs without a permission error

#### Scenario: Made's entry is not duplicated

- Given Made is started from a shell whose `CLAUDE_CODE_PLUGIN_DIRS` already contains Made's shipped plugin directory
- When the user opens a shell tab
- Then the variable contains Made's entry exactly once and every other entry unchanged

#### Scenario: Other launches do not receive the plugin

- Given OpenCode and grok are on PATH and an SSH server is registered
- When the user chooses the OpenCode row, the Grok Build row and a remote launch row
- Then none of those launches has the shipped local plugin's entry added to `CLAUDE_CODE_PLUGIN_DIRS`, the remote launch receives only what SSH-11 defines, and the Grok launch still injects no plugin configuration

#### Scenario: No persistent Claude Code configuration change

- Given the user has Claude Code settings, installed plugins, marketplaces and hooks
- When the user runs Claude Code in Made tabs, Made restarts, and the user runs Claude Code in an ordinary terminal outside Made
- Then Made never ran a `claude` subcommand other than `claude agents --json`, no file in the user's Claude Code configuration was created, edited or deleted, the user's own plugins and hooks still run in Made, and the Claude Code outside Made does not load Made's plugin

#### Scenario: Missing shipped plugin

- Given the shipped plugin is missing or unreadable in the installation
- When the user opens a shell tab and starts Claude Code there, and separately chooses the Claude Code row
- Then shells and Claude Code still start, no entry pointing to the missing plugin is added, and Made records that Claude Code session-status hooks are unavailable

### Requirement: Claude Code privacy and observation boundary

Made MUST observe Claude Code only through the structured status-protocol emissions of `Program status protocol in the terminal`, the metadata-only records of `Claude Code session events`, and the `pid`, `cwd`, `kind` and `name` fields of the `claude agents --json` listing as `Claude Code explorer follow and precedence` uses it. Made MUST read a tab's terminal window title only as the bounded tooltip title of `Claude Code tab presentation` and as the selector of `Claude Code agent-view session follow`, only while a Claude Code protocol status is live in that tab, and a change of that title then also triggers a listing run; without a live protocol status a title change triggers nothing and the title is not kept. Made MUST use the user's input to a tab only to detect whether it submits a line (Enter, Return, or pasted text containing a newline) as a listing trigger, and MUST NOT keep, log or otherwise inspect that input. A window title MUST NOT be used for status or identity, and MUST NOT be used as or trusted as a shell or session directory; its one effect on a directory is to select, by exact normalized name, one of the user's own background sessions listed by `claude agents --json`, whose listed directory is then validated before use. On the host and on any SSH server, Made MUST NOT read or scrape screen or scrollback contents, prompts, Claude Code transcripts or session files, including Claude Code's internal `~/.claude/sessions` and `~/.claude/jobs` directories, the IDE lockfile, status-line output, or Claude Code configuration to derive status, title, identity or directory. Made MUST NOT configure a Claude Code status line or HTTP hook, MUST NOT open a network listener for Claude Code, and MUST NOT send any Claude Code data off the device, except that for an SSH tab the owned-session metadata SSH-11 permits travels from the server to the client over that tab's authenticated SSH transport and nothing else does. Prompt text, the protocol `msg` field, notification message text, tool data and transcript content MUST NOT be stored, logged, displayed or copied by Made. Session titles, session ids and directories are untrusted: they MUST be sanitized by replacing control characters and newlines with spaces and removing bidirectional and format control characters before display, logging or copy, and directories MUST be validated before use and never executed or passed to a shell.

#### Scenario: No prompt or message content anywhere

- Given Claude Code sessions in Made tabs where prompts, permission notifications and protocol `msg` fields contain unique marker strings
- When every Made-owned spool, log file, diagnostics row and copied diagnostics report is searched
- Then no marker string is found

#### Scenario: Window titles and screen are not used

- Given Claude Code is present in a tab and the status protocol and hooks are unavailable
- When Claude Code changes the window title, including its working and idle glyph prefixes, and prints status text on screen
- Then Made derives no status, title, identity or directory from them and the tooltip stays `Claude Code`

#### Scenario: Permission message never shown

- Given Claude Code sends a session-level `state=blocked` emission with `kind=permission` and a `msg` whose decoded text contains a unique marker
- When the emission is received
- Then the dot becomes attention and the marker appears in no tooltip, log, spool, diagnostics row or copied report

#### Scenario: No network or Claude Code configuration access

- Given Claude Code sessions are running in Made tabs
- When Made's listening sockets, outbound connections and file accesses are inspected
- Then Made opened no listener or connection for Claude Code and read no Claude Code transcript, session, IDE lockfile or configuration file

#### Scenario: Remote metadata is the only Claude Code data crossing SSH

- Given remote Claude Code sessions in SSH tabs whose prompts, notifications, protocol `msg` fields and tool data contain unique marker strings
- When the SSH transport traffic Made generates for those tabs and every Made-owned remote and local file are inspected
- Then only owned-session metadata and listing-derived directories cross the transport, no marker string is found, and Made opened no listener or outbound connection for Claude Code outside that tab's SSH transport

### Requirement: Declared precedence (AIA-05)
Each integration MUST declare a precedence rank, and ranks MUST be unique within a list. The core MUST derive local explorer-root precedence and local tab-presentation precedence from these ranks, never from catalog order or a hard-coded chain. The declared ranks MUST reproduce today's order: root follow OpenCode > Claude Code > Grok > shell, and local tab presentation OpenCode active > Claude Code present > Grok open > folder. When a holder releases, the root MUST return to the highest-ranked remaining holder with a usable directory, else the shell. Remote explorer-root and remote tab-presentation precedence MUST follow the same order, as SSH-06, SSH-10 and SSH-11 of `specs/ssh-remote-workspaces.md` define it, with the remote shell cwd and remote launch directory in place of the shell directory and the server-name label and terminal glyph in place of the folder; the delimited remote paths of AIA-02 MAY implement it directly. Root-source identifiers MUST stay `shell`, `opencode`, `claude-code` and `grok`.

#### Scenario: Precedence by rank
- Given a local tab where OpenCode, Claude Code and Grok each hold a usable directory
- When holders release one at a time in the orders OpenCode, Claude Code, Grok and OpenCode, Grok, Claude Code
- Then the effective root source is always the highest-ranked remaining holder, then `shell`, with the same root, generation and switch results as before the change

#### Scenario: Rank is independent of catalog order
- Given the catalog order OpenCode, Grok Build, Claude Code
- When Claude Code and Grok both hold a directory and OpenCode holds none
- Then Claude Code owns the root and the tab presentation, although Grok Build is earlier in the menu

#### Scenario: Duplicate rank rejected
- Given an injected list in which two integrations declare the same rank
- When the list is validated by test
- Then the validation fails

#### Scenario: Remote order equals local order
- Given a remote tab where remote OpenCode, Claude Code and Grok each hold a usable remote directory
- When holders release one at a time in the orders OpenCode, Claude Code, Grok and OpenCode, Grok, Claude Code
- Then the remote root and tab presentation always belong to the highest-ranked remaining holder, then the remote shell cwd and server-name label, matching the local order
