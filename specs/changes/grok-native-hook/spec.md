# Change Spec: grok-native-hook

This delta modifies `specs/grok-session-follow.md` (requirements `Grok status hook` and `Shipped Grok plugin install`) and `specs/ssh-remote-workspaces.md` (requirements `Remote OpenCode session and cwd follow (SSH-06)`, `Local preservation and truthful parity evidence (SSH-09)` and `Remote Grok Build session follow and tab parity (SSH-10)`). Each MODIFIED requirement below replaces the realized requirement of the same name in full, scenarios included.

## MODIFIED Requirements

### Requirement: Grok status hook

Made MUST ship the Grok plugin at `plugins/grok-made`. The user removes it with `grok plugin uninstall grok-made --confirm`. Made MUST NOT hand-edit the user's Grok `config.toml`. The plugin's session-status hook MUST be a native executable built for the operating system and CPU architecture of the machine where that plugin copy is installed, and running it MUST NOT require Python or any other interpreter, language runtime, or separately installed package on that machine. The supported hook platforms are the local client targets macOS arm64, macOS x64, Linux glibc x64 and Windows x64, and the remote targets of SSH-09, Linux glibc x86_64/aarch64 and macOS x86_64/arm64. The plugin MUST register the hook for exactly these Grok events and matchers, each with a 5-second timeout: `SessionStart`, `UserPromptSubmit`, `PreToolUse` and `PostToolUse` matching `ask_user_question|AskUserQuestion|AskQuestion`, `Stop`, `StopFailure`, `StopCancelled`, `Notification` matching `idle_prompt|permission_prompt`, and `SessionEnd`.

On each run the hook MUST read one JSON value from standard input without waiting for standard input to close, waiting at most about two seconds in total; leading whitespace MUST be ignored, and invalid UTF-8 bytes MUST be decoded as replacement characters rather than abort reading. If no complete value arrives within that bound, input ends first, or the first value is not a JSON object, the hook MUST write no line. In the rules below, a field's text is its value when that value is a non-empty string and is otherwise empty, and the first of several named fields with non-empty text wins. The event name is the text of `hookEventName` or `hook_event_name`, compared case-insensitively with `-` treated as `_`. The notification type is the text of `notificationType` or `notification_type`, compared case-insensitively. The tool is an asking tool when the text of `toolName` or `tool_name`, compared case-insensitively, is `ask_user_question`, `askuserquestion` or `askquestion`. A stop is still active when `stopHookActive` or `stop_hook_active` is present with a value other than `false`, `null`, a numeric zero such as `0` or `0.0`, an empty string, an empty array or an empty object. The status MUST be the first match of these rules, in order: event `session_start` or `sessionstart` is `idle`; event `user_prompt_submit` or `userpromptsubmit` is `working`; event `pre_tool_use` or `pretooluse` with an asking tool is `attention`; event `post_tool_use` or `posttooluse` with an asking tool is `working`; notification type `permission_prompt` is `attention`; notification type `idle_prompt` is `idle`; event `stop_failure` or `stopfailure` is `error`; event `stop_cancelled`, `stopcancelled`, `session_end` or `sessionend` is `idle`; event `stop` is `working` when the stop is still active and `idle` otherwise. Input matching no rule MUST write no line.

For a matched status the hook MUST append exactly one line, a UTF-8 JSON object followed by a newline, to `$GROK_HOME/made-grok-status.jsonl`, where `GROK_HOME` is used when set to a non-empty value and `~/.grok` under the running user's home directory is used otherwise, creating the missing parent directory, and MUST append each line with a single write so concurrent hook runs never interleave partial lines when that log is on a local filesystem; a log on a network filesystem has no such guarantee. The object MUST carry exactly these fields: `at`, the UTC time the line is written at second precision formatted `YYYY-MM-DDTHH:MM:SSZ`; `sessionId`, the text of `sessionId` or `session_id`; `pid`, the integer process id of the hook process's parent, which is the Grok process that ran the hook, and any launch indirection introduced by this change MUST NOT change which process that is; `cwd`, the text of `cwd`; `event`, the event name text exactly as received; `notification`, the notification type text exactly as received; and `status`, one of `idle`, `working`, `attention` or `error`. Absent text fields MUST be written as empty strings. The hook MUST write nothing to standard output and MUST exit with status 0 for every input and every failure, including malformed input, an unwritable log and any internal error, so it never denies, blocks or alters a tool or a prompt. Made MUST apply only lines appended after Made starts.

#### Scenario: Session start and prompt move the explorer immediately

- Given the plugin is installed and Grok is running in a Made tab
- When a session starts or the user submits a prompt in a session
- Then the explorer switches to that session's directory without waiting for the next inventory refresh, a start shows idle, and a prompt shows working

#### Scenario: Question and permission show attention

- Given a Grok session is working in a Made tab
- When Grok asks `ask_user_question` or raises a permission notification
- Then that session's dot becomes attention, and after the user answers `ask_user_question` the dot returns to working

#### Scenario: Turn end and session end

- Given a Grok session is working or waiting
- When the turn stops, fails, is cancelled, goes idle, or the session ends
- Then a stop that still has hook work in progress stays working, a finished stop or idle notification is idle, a failure is attention, a cancellation is idle, and session end removes that session's dot

#### Scenario: Parallel sessions stay distinct

- Given three Grok sessions are open in the same tab
- When each receives its own prompt, question, or stop
- Then each dot follows only that session's events

#### Scenario: Every registered event maps to its status

- Given the installed native hook
- When the hook receives each registered event in both camelCase and snake_case field spellings, including `Stop` with and without an active stop hook, `PreToolUse` and `PostToolUse` for an asking tool, `Notification` for `permission_prompt` and `idle_prompt`, and a notification type that matches no rule
- Then each mapped input appends one line whose `status` equals the status the rules above give, and the unmatched input appends no line

#### Scenario: Line carries exactly the seven fields

- Given `GROK_HOME` is unset and a Grok process runs the hook for a prompt event carrying `sessionId` `s1`, `cwd` `/work/a` and `hookEventName` `UserPromptSubmit`
- When the hook runs
- Then `~/.grok/made-grok-status.jsonl` gains one line whose object has exactly `at` in UTC `YYYY-MM-DDTHH:MM:SSZ` form, `sessionId` `s1`, `pid` equal to that Grok process's id, `cwd` `/work/a`, `event` `UserPromptSubmit`, `notification` empty and `status` `working`, and with `GROK_HOME` set to a non-empty directory the same line goes to that directory's `made-grok-status.jsonl`, creating the directory if missing

#### Scenario: Standard input left open

- Given Grok writes one complete JSON event object to the hook's standard input and keeps standard input open
- When the hook runs
- Then it appends the line and exits with status 0 without waiting for standard input to close, and well within the 5-second timeout

#### Scenario: Malformed, empty or late input writes nothing

- Given hook runs whose standard input is closed empty, stays open with no data, carries truncated JSON, carries invalid JSON, or carries a JSON array or string instead of an object
- When each run ends
- Then no line is appended, the hook writes nothing to standard output, it exits with status 0, and runs whose input never completes finish after about two seconds

#### Scenario: Failures never deny

- Given the log directory cannot be created or the log file cannot be written
- When the hook runs for a mapped event
- Then it writes nothing to standard output, exits with status 0, and the tool or prompt proceeds unchanged

#### Scenario: Concurrent hook runs append whole lines

- Given several Grok sessions whose hooks run at the same moment against the same `made-grok-status.jsonl` on a local filesystem
- When each hook appends its line
- Then the log gains one complete, parseable line per run and no line is interleaved with another

#### Scenario: No Python on the machine

- Given a supported local or remote machine with no `python`, `python3` or other interpreter installed or on PATH, and the plugin installed there
- When Grok sessions start, receive prompts and questions, stop and end
- Then the hook appends the same lines the Python hook appended on a machine with Python, except that a valid multibyte UTF-8 character split across two reads stays intact instead of becoming replacement characters, and session follow, dots and diagnostics behave as with Python present

### Requirement: Shipped Grok plugin install

After PATH detection reports grok, Made MUST ensure the shipped `plugins/grok-made` package is installed by running `grok plugin install <absolute shipped path> --trust` with the grok executable that lookup found. In a release, the shipped package is the install-root `plugins/grok-made` copy beside `plugins/opencode-made`. In a development run, the shipped package is the build-staged copy of the repository's `plugins/grok-made` that contains the built hook; the repository's `plugins/grok-made` holds source only and MUST NOT itself be installed. `<absolute shipped path>` is the shipped package directory's absolute path, and a missing shipped package MUST be treated as an install failure. The shipped package MUST contain the session-status hook executable built for the operating system and CPU architecture Made runs on, so the copy that `grok plugin install` installs runs on that machine without any interpreter, runtime or other prerequisite, and Grok MUST be able to run that installed hook through the plugin's hook registration on every supported local platform. The shipped package MUST NOT contain a Python hook, any other interpreted hook, or any test file. The shipped `plugin.json` `version` is governed by the bundled-manifest version rule of the release-distribution contract (REL-02) and is not otherwise constrained here. That command is not part of launching Grok and is not a hand edit of `config.toml`. Made MUST compare the `version` field of the shipped `plugin.json` with the `version` field of the installed grok-made `plugin.json` as dotted numeric versions. Equal versions MUST skip the install command. A missing grok-made install, or an installed version lower than the shipped version, MUST run that command. An installed version higher than the shipped version MUST be left in place. The version check and the install command MUST be separate from the metadata-only PATH lookup and MUST NOT block shell, explorer, or Grok startup. Supplying a catalog to the menu without running PATH detection MUST NOT install the plugin. If the install command fails, Made MUST still let the user start Grok, and Made MUST record for Grok session follow that session-status hooks are unavailable. Made MUST NOT hand-edit the user's Grok `config.toml` and MUST NOT pass OpenCode's plugin environment to Grok.

#### Scenario: Missing or older plugin is installed

- Given PATH detection reports grok and grok-made is missing or its installed `plugin.json` version is lower than the shipped version
- When Made ensures the shipped plugin
- Then it runs `grok plugin install` with the absolute path of the shipped `plugins/grok-made` and `--trust`, and it does not hand-edit `config.toml`

#### Scenario: Shipped package matches the host

- Given Made installed on macOS arm64, macOS x64, Linux x64 or Windows x64 with no Python installed
- When Made installs the shipped plugin and a Grok session started afterwards submits a prompt
- Then the installed hook is the executable built for that host, it runs, and its line moves the explorer and shows working

#### Scenario: Development run installs only the staged copy

- Given a development run whose build staged a copy of `plugins/grok-made` with the built hook, and a variant where that staged copy is missing
- When Made ensures the shipped plugin
- Then the staged copy's absolute path is passed to `grok plugin install`, the repository's source `plugins/grok-made` is never passed, and in the missing variant Made runs no install command and records that session-status hooks are unavailable while Grok still starts

#### Scenario: Same version skips install

- Given PATH detection reports grok and the installed grok-made `plugin.json` version equals the shipped version
- When Made ensures the shipped plugin
- Then it does not run `grok plugin install`

#### Scenario: Newer installed copy stays

- Given PATH detection reports grok and the installed grok-made `plugin.json` version is higher than the shipped version
- When Made ensures the shipped plugin
- Then Made does not run the install command and leaves the installed copy in place

#### Scenario: Ensuring the plugin does not block startup

- Given PATH detection reports grok
- When Made starts the shell and explorer, or the user starts Grok
- Then those do not wait on the version check or on `grok plugin install`

#### Scenario: Catalog without detection does not install

- Given the menu is given a Grok catalog entry and PATH detection did not run
- When the menu is shown
- Then Made does not run `grok plugin install`

#### Scenario: Install failure leaves hooks unavailable

- Given PATH detection reported grok and `grok plugin install` fails
- When the user starts Grok from the menu or by typing grok in a shell
- Then Grok still starts, Made records for Grok session follow that session-status hooks are unavailable, Made does not hand-edit `config.toml`, Made does not pass OpenCode's plugin environment to Grok, and the shell-typed grok is not given the `dashboard` argument

### Requirement: Remote OpenCode session and cwd follow (SSH-06)

Supported remote integration MUST expose the current optional session-map data/actions, activity indicators and focus correlation within the owning remote OpenCode instance. Root precedence MUST remain active OpenCode session directory, then usable connected sessionless view directory when tabs are enabled, then latest validated remote shell cwd, then remote launch directory. Shell cwd MUST continue to be recorded while OpenCode takes precedence. Integration disconnect/normal exit/unsupported state/tabs disabled/new epoch MUST clear view-directory state and resume remote shell fallback without using the host cwd. Worktree/session changes MUST not restart terminal processes or manage worktrees. Missing or incompatible OpenCode/plugin MUST be distinguished from loss of SSH transport; optional integration failure MUST NOT by itself disable a healthy remote filesystem or terminal. Safe shell hooks MUST preserve remote user startup files and existing hooks; unsupported reporting MUST be reported honestly, never replaced by prompt scraping or injected commands in a running shell.

Initial helper/plugin staging MUST require explicit per-server/version consent before app-owned remote files are written; consent rejection, unsupported artifacts, missing supported-target capability, missing checksum tool, or transfer/integrity failure MUST fail clearly without mutating personal configuration or silently offering terminal-only parity. Existing pre-provisioned matching private artifacts MAY be verified and reused. Made MUST NOT install OpenCode. Made MUST NOT install, probe, execute, configure, or fall back to tmux; a server with tmux absent and a server with a user-installed tmux MUST be treated identically, and that user installation MUST NOT become a dependency, fallback, or hidden prerequisite. The frozen major-1 `Capabilities.tmux` field MUST remain for wire compatibility, MUST be encoded false, and MUST be ignored by every consumer; its presence is not permission to probe or require tmux. Optional OpenCode absence/version/integration failure MUST still leave an otherwise supported terminal/explorer usable. Remote config composition MUST read only remote user config/inline settings and inject only the remote app-owned plugin and remote loopback bridge for that launch; host provider/config/environment secrets MUST NOT be uploaded. Direct OpenCode and manual `opencode` inside a supported remote shell MUST both receive this launch-local integration environment. User shell, OpenCode, SSH, and any pre-existing tmux configuration bytes MUST remain unchanged.

Initial staging and exact reuse MUST remain unchanged: a missing version target is staged once after per-server/version consent, and an existing target whose shipped artifacts verify successfully is reused without rewriting. When an existing same-version target contains a missing, corrupt or different artifact set, Made MUST detect the mismatch, MUST NOT overwrite, delete or repair anything before a separate explicit `Update companion` approval for the current attempt, and MUST report a sanitized update-required state naming the affected version and target instead of a generic `ssh connection failed` or an app-version bump recommendation. The existing per-server/version staging consent remains valid for initial staging and exact reuse but does not authorize changed bytes; refusal or cancellation of the update approval MUST perform no remote write or delete and MUST leave the prior state actionable and retryable.

An approved same-version update MUST run through the fixed staging script: write all 10 shipped artifacts (the companion, the five OpenCode plugin files, and the grok-made `plugin.json`, `README.md`, `hooks/hooks.json` and native session-status hook executable built for that target) into a private temporary sibling of the existing target, verify every size, hash and required permission/executable mode there, then replace the existing target path through the narrowest practical atomic or rollback-safe swap; only after the verified swap may Made remove the temporary and replaced app-owned artifact directories so no per-build copies accumulate. Any failure before the verified swap MUST leave the original target byte-for-byte untouched; after classifying a failed update, Made MUST preserve the actionable failing phase and reason and keep the original reusable or update-required as appropriate. This update path MUST NOT add an arbitrary remote delete, rename or exec API. The companion and Grok-install commands MUST continue resolving the same current version and target path after an update, `plugins/grok-made` MUST remain present there for the consented installer, and exact reuse of the updated set MUST be verified on the next attempt.

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
- Then all 10 artifacts are written to a private temporary sibling and verified for size, hash and permissions, the existing target is replaced atomically or rollback-safely, temporary and replaced app-owned directories are removed without per-build accumulation, the pre-swap failure leaves the original untouched with its actionable phase preserved, the running process keeps its OS lifecycle, later companion and Grok-install launches resolve the same updated version/target path, and the next attempt verifies and reuses the exact updated set

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

### Requirement: Local preservation and truthful parity evidence (SSH-09)

All existing local workflows MUST remain available and MUST NOT wait for SSH test/detection/connectivity or change local path/security semantics. Remote work MUST remain asynchronous and bounded so stalled servers cannot block input/tab switching/preview return/Explorer controls in other workspaces. Diagnostics MUST identify selected authority, cwd/root/source, shell reporting capability, transport versus OpenCode integration status, remote Grok Build tier status under SSH-10, supported versions and actionable failure reasons without adding routine technical chrome. Acceptance MUST execute real local and remote workflows in a controlled environment, not only inspect source or mock a PTY. Evidence MUST record client/server OS/architecture/shell/OpenCode versions, network profile, workload, tab count, window size and backend, and distinguish automated/model/software-rendered evidence from native GUI evidence and untested combinations.

A scoped implementation archive of this change folder MAY proceed with environment combinations explicitly labeled `UNVERIFIED` and accepted by the requester, such as the unexecuted native reference slice and full client/server matrix; this records an honest evidence limit, does not satisfy the acceptance above, and MUST NOT remove or downgrade any product guarantee. The bounded Linux loopback CI smoke job is regression evidence for the protocol and harness surface only, not a substitute for the approved client/server matrix or native GUI evidence, and full release acceptance MUST remain blocked until the `UNVERIFIED` combinations execute and their results are recorded.

Initial remote targets MUST be Linux glibc x86_64/aarch64 (glibc 2.31 or newer) and macOS x86_64/arm64 (macOS 12 or newer), with OpenSSH server, POSIX `sh`, sha256sum or shasum, and writable private user app data. tmux MUST NOT be a target prerequisite. The companion MUST operate without a display server, GUI runtime, remote system clipboard, or tmux. Made MUST NOT install tmux or a Rust toolchain on the remote, or in a development container, to satisfy this change. bash, zsh and fish MUST support launch-local cwd reporting on these targets when installed; other shells retain honest launch-directory/latest-cwd fallback. Windows remotes and Linux musl targets are unsupported and MUST fail capability preflight explicitly. Client targets remain the existing macOS arm64/x64, Linux x64 X11/Wayland and Windows x64, using installed OpenSSH 8.4 or newer and working interactive prompt support. Remote OpenCode v2 is optional and must be detected on the remote user's executable path; no host executable is reused. Remote grok is likewise optional and detected only on the remote PATH under SSH-10. Remote python3, or any other interpreter or language runtime, MUST NOT be a prerequisite for any SSH-10 Grok Build tier, for the consented hook install, or for hook activity; its presence or absence MUST NOT change any tier, install outcome or diagnostics state, and Made MUST NOT install python3 or any other interpreter on the remote or in a development container.

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
