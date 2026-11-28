# Grok Session Follow Specification

## Purpose

This specification owns Grok Build session follow for a Made terminal tab: the open-session inventory, the shipped status plugin, explorer follow, workspace-tab status, the integrated-TUI catalog menu, and the revealed Grok Build and OpenCode diagnostics wording. OpenCode session-follow behavior that this specification does not change remains in `specs/opencode-session-follow.md`. Presentation that this specification does not change remains in `specs/made-ui-ux.md`.

## Requirements

### Requirement: Grok open-session inventory

While Grok runs in a Made terminal tab, Made MUST treat `$GROK_HOME/active_sessions.json` (`~/.grok/active_sessions.json` when `GROK_HOME` is unset) as the inventory of sessions that Grok still has open. A row belongs to that tab only when its process id is the terminal process or a descendant of it. Made MUST refresh this inventory about once a second. The inventory supplies session id, directory, and attachment order. It does not supply status. An unreadable file, a row whose process is outside the tab, or more than one matching interpretation of a malformed row MUST NOT move the explorer.

#### Scenario: Newly attached session moves the explorer

- Given Grok is running in a Made terminal and the user attaches a session from the Grok dashboard
- When that session becomes the newest row for this terminal in the inventory
- Then the explorer for that tab switches to that session's directory

#### Scenario: Inventory does not steal the explorer back

- Given two Grok sessions are open in the same tab and the user has just submitted a prompt in the older one
- When the once-a-second inventory refresh runs and the other session still has the newer attachment time
- Then the explorer stays on the session that received the prompt

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

### Requirement: Grok tab dots

When OpenCode is not connected in a workspace tab, Claude Code is not present there as `specs/claude-code-session-follow.md` defines, and that tab has open Grok sessions, the tab MUST paint one status dot per open Grok session using the same size, placement, cap of eight, pulse, colors, tooltip summary, and accessible-name summary as OpenCode session dots. Working is busy, attention and error are attention, and idle is idle. A session present only in the inventory, before any hook event, MUST show idle. While OpenCode is not active and Claude Code is not present, a tab whose target is a running Grok TUI, or a shell tab with open Grok sessions, MUST use the Grok Build program label of `specs/made-ui-ux.md` `Brand and Workspace Tabs` (the session title or summary when the inventory exposes one, else `Grok Build`) and the official 16px `grok.svg` mark in the leading icon slot, even before a session exists. Its tooltip MUST be the session summary when sessions exist and MUST omit the workspace path. When that Grok is no longer running and the tab has no open Grok sessions, the label and leading icon MUST return to the shell label and terminal glyph, unless Claude Code is present, in which case `specs/claude-code-session-follow.md` owns them. If OpenCode is active, the label, leading icon, and dots MUST stay OpenCode's even when Grok sessions exist. If OpenCode is not active and Claude Code is present, the label, leading icon, and dots MUST be Claude Code's even when Grok sessions exist.

#### Scenario: Dots match session state

- Given a Grok tab whose sessions are working, waiting on a question, and idle, with no Claude Code present
- When the workspace tab is displayed
- Then it shows a pulsing busy dot, a yellow attention dot, and a neutral idle dot, the label is the Grok Build program label (`Grok Build` when the inventory exposes no title), the leading icon is the official `grok.svg` mark, and the tooltip counts those three categories without the workspace path

#### Scenario: Claude Code takes over and hands back the tab

- Given a shell tab showing `Grok Build` with two open Grok sessions
- When the user starts Claude Code in that tab, and later Claude Code exits while both Grok sessions stay open
- Then while Claude Code is present the label, leading icon and dots are Claude Code's, and after it exits the tab shows `Grok Build`, the official mark and the two Grok dots again

### Requirement: Grok yields to OpenCode and returns to the shell

A connected OpenCode session MUST keep the explorer root and the workspace-tab dots. Grok MUST NOT replace them. A live owned Claude Code session directory, as `specs/claude-code-session-follow.md` defines, MUST also keep the explorer root over Grok, and Grok MUST NOT replace it. While OpenCode is not connected and the tab has no live owned Claude Code session, an open Grok session's directory MUST win over the shell directory. When no Grok session remains open for that terminal and no OpenCode or Claude Code source applies, the explorer MUST return to the shell directory. Selecting an already-open Grok session, with no new attach and no new prompt, MUST NOT move the explorer.

#### Scenario: OpenCode remains first

- Given OpenCode is connected in a tab and Grok sessions are also open there
- When Grok events arrive
- Then the explorer root and the tab dots stay on OpenCode

#### Scenario: Claude Code outranks Grok for the root

- Given a live owned Claude Code session and an open Grok session in the same tab, with the explorer on the Claude Code session directory
- When a Grok session attaches or receives a prompt
- Then the explorer stays on the Claude Code session directory, and when Claude Code exits the explorer moves to the open Grok session's directory

#### Scenario: Grok exit restores the shell

- Given the explorer is following a Grok session and no OpenCode or Claude Code source applies
- When that terminal no longer has an open Grok session
- Then the explorer returns to the shell directory

#### Scenario: Focus alone does not move the explorer

- Given two Grok sessions are already open and the explorer follows one of them
- When the user only focuses the other session in the Grok dashboard
- Then the explorer stays where it was

### Requirement: Catalog-driven new-tab menu

The `+` control MUST keep its tab-strip placement with the tooltip and accessible name `Open workspace` and MUST open the Open workspace screen directly with no menu (`Open Workspace Screen` in the UI spec). The screen's launch bar MUST be driven by the integrated-TUI catalog rather than by a special case for one program. Detecting a catalog entry on the local host MUST be a metadata-only PATH lookup that never spawns the program, never checks a version, and never installs a plugin, and that lookup MUST NOT delay shell or explorer startup. Empty and relative PATH segments MUST be ignored. The launch bar MUST show Terminal first, using the existing terminal glyph and opening a plain workspace tab in the effective launch path, then one button for each integrated TUI detected on the selected host in catalog order; with no TUI detected it MUST show Terminal alone. Catalog order MUST be OpenCode, then Grok, then Claude Code, and a program that is not detected on the selected host MUST be omitted. Choosing a local TUI button MUST start that executable's absolute path directly as the child of a new tab whose launch folder and explorer root are the effective launch path chosen on the screen, with no shell first and no further selection surface, and the current tab MUST keep running. Made MUST NOT launch an integrated TUI unless the user chooses its button. The OpenCode button MUST keep the vendored 16px opencode.svg and MUST keep its existing launch, including its per-launch plugin configuration injection, follow, and dots. The Grok button label MUST be Grok Build, and its 16px leading slot MUST paint the official mark in `grok.svg` (the two paths from https://grok.com/images/favicon.svg, fill #FCFCFC, viewBox "56 56 400 400") rather than the terminal glyph. Choosing Grok MUST launch grok's absolute path with the single argument `dashboard`. That launch MUST NOT inject plugin configuration, including OpenCode's plugin environment and Made's Claude Code plugin entry, and MUST NOT write the user's Grok config. The Claude Code button's label, mark and launch are owned by `specs/claude-code-session-follow.md`. Codex and any other agent TUI MUST stay out of the catalog. Typing grok in a shell MUST remain valid, MUST NOT be replaced by the launch bar, and MUST NOT be given the `dashboard` argument.

#### Scenario: No detected TUI shows Terminal alone

- Given no integrated TUI is on PATH and Local is selected
- When the user activates `+`
- Then the Open workspace screen opens with no menu and its launch bar shows Terminal alone

#### Scenario: Catalog launch bar lists detected TUIs in order

- Given OpenCode, Grok and Claude Code are all on PATH and Local is selected
- When the user activates `+`
- Then the launch bar is Terminal, then OpenCode, then Grok Build, then Claude Code, and the `+` tooltip and accessible name are Open workspace

#### Scenario: Only Grok is installed

- Given Grok is on PATH and neither OpenCode nor Claude Code is, and Local is selected
- When the user activates `+`
- Then the launch bar is Terminal, then Grok Build, and OpenCode and Claude Code are omitted

#### Scenario: Grok button uses the official mark

- Given the launch bar includes Grok
- When the Grok button is shown
- Then its label is Grok Build and its 16px leading slot paints the official mark from `grok.svg`

#### Scenario: OpenCode button keeps its icon and launch

- Given OpenCode is on PATH and Local is selected with effective launch path `/work/api`
- When the user chooses the OpenCode button
- Then the button shows the vendored 16px opencode.svg, Made starts OpenCode's absolute path directly in a new tab whose launch folder and explorer root are `/work/api` with no shell first, and OpenCode's existing launch injection, follow, and dots stay in place

#### Scenario: Choosing Grok starts grok in the chosen folder

- Given a tab is running, Grok is on PATH, and Local is selected with effective launch path `/work/api`
- When the user chooses Grok Build
- Then Made starts grok's absolute path plus the argument `dashboard` directly as the child of a new tab whose launch folder and explorer root are `/work/api`, with no shell first and no further selection surface, the current tab keeps running, that launch does not inject plugin configuration, OpenCode's plugin environment or Made's Claude Code plugin entry, and does not write the user's Grok config, and typing grok in a shell is not given that argument

#### Scenario: Terminal button opens a shell workspace

- Given at least one integrated TUI is detected
- When the user chooses Terminal
- Then a plain shell workspace opens in the effective launch path and no integrated TUI is launched

#### Scenario: Detection does not launch or block startup

- Given an integrated TUI executable is on PATH
- When Made starts the shell and explorer
- Then the lookup does not spawn that program, does not check its version, ignores empty and relative PATH segments, and startup does not wait on the lookup

#### Scenario: PATH lookup does not install

- Given grok is on PATH
- When Made runs the metadata-only PATH lookup
- Then that lookup does not spawn grok and does not install the plugin

#### Scenario: Launch bar does not replace a shell launch

- Given Grok is on PATH
- When the user types grok in a shell tab and does not choose the Grok button
- Then Made does not launch Grok on its own, and that shell session remains a valid way to start Grok

#### Scenario: Other agents stay out

- Given Codex or another agent TUI is on PATH
- When the user opens the Open workspace screen
- Then no launch button is shown for it

### Requirement: Grok Build diagnostics row

When About diagnostics are revealed, Made MUST show exactly one Grok Build row for the workspace whose diagnostics are revealed, directly with the existing OpenCode status pill, using that same pill-plus-sentence treatment, and not in the sectioned connection, session, worktree, version, metrics, or log detail. The OpenCode row's visible pill label and reason sentence MUST follow the OpenCode diagnostics wording requirement. The Grok row MUST stay hidden until the user activates `Show diagnostics`. Its pill and sentence MUST be absent from the normal workspace chrome, including the header, workspace tabs, explorer, and terminal, and this MUST NOT change the Grok Build tab label or menu row. The inactive pill MUST use the existing inactive pill treatment, the degraded pill MUST use the existing degraded pill treatment, and the ready pill MUST use the success tone. The copied diagnostics report MUST include the same Grok sentence the revealed row shows and MUST include the OpenCode sentence shown on the OpenCode row.

The row MUST show exactly one of these states. While PATH detection is still running, the pill MUST be inactive and the sentence MUST be `Checking for Grok Build.` That in-progress detection MUST NOT be shown as not on PATH. When detection has finished and grok is not on PATH, or an explicit catalog omits Grok and detection is not running, the pill MUST be inactive and the sentence MUST be `Grok Build is not on PATH.` When grok was detected and the shipped-plugin install is still running, the pill MUST be inactive and the sentence MUST be `Installing the Grok Build plugin.` Still running is the period after grok was detected until the install succeeds, the installed version is already current, or the install fails. When the install succeeded, or the installed version is already current, and Grok is running in this workspace, the pill MUST be ready in the success tone and the sentence MUST be `Grok Build session follow is ready.` Already current means the installed version is equal to or newer than the shipped version. Grok is running in this workspace when either this workspace's terminal target is Grok and that process is still alive, or this workspace's shell has at least one open Grok session. OpenCode owning the tab label MUST NOT by itself count as Grok not running. Grok running only in another workspace MUST NOT make this row ready. When the install succeeded, or the installed version is already current, and Grok is not running in this workspace, the pill MUST be inactive and the sentence MUST be `Grok Build is not running.` The sentences `Grok Build session follow is ready.` and `Grok Build is not running.` MUST NOT replace checking, not on PATH, installing, hooks unavailable, or the explicit-catalog sentence. When the install failed, the pill MUST be degraded and the sentence MUST be `Grok Build session-status hooks are unavailable.` When an explicit catalog includes Grok and PATH detection did not run, the pill MUST be inactive and the sentence MUST be `Grok Build plugin was not installed for this launch.` That state MUST NOT claim the plugin is ready and MUST NOT claim an install is running.

#### Scenario: Grok diagnostics stay hidden until revealed

- Given a workspace
- When the user opens About and has not activated `Show diagnostics`
- Then the Grok row is not shown and the OpenCode status pill is not shown

#### Scenario: Grok diagnostics stay out of workspace chrome

- Given a workspace, whether or not About diagnostics are revealed
- When the user looks at the header, workspace tabs, explorer, and terminal
- Then those surfaces do not show the Grok diagnostics pill or sentence, and the Grok Build tab label and menu row are unchanged

#### Scenario: Checking for Grok Build

- Given PATH detection is still running
- When the user reveals About diagnostics
- Then exactly one Grok row appears directly with the OpenCode status pill, its pill is inactive, its sentence is `Checking for Grok Build.`, and it is not shown as not on PATH

#### Scenario: Grok Build is not on PATH

- Given PATH detection has finished and grok is not on PATH, or an explicit catalog omits Grok and detection is not running
- When the user reveals About diagnostics
- Then the Grok row's pill is inactive and its sentence is `Grok Build is not on PATH.`

#### Scenario: Installing the Grok Build plugin

- Given grok was detected and the shipped-plugin install is still running
- When the user reveals About diagnostics
- Then the Grok row's pill is inactive and its sentence is `Installing the Grok Build plugin.`

#### Scenario: Grok session follow is ready

- Given the shipped plugin install succeeded or the installed version is already current, and either this workspace's terminal target is Grok and that process is still alive or this workspace's shell has at least one open Grok session
- When the user reveals About diagnostics
- Then the Grok row's pill is ready in the success tone and its sentence is `Grok Build session follow is ready.`

#### Scenario: OpenCode label does not hide a running Grok

- Given the shipped plugin install succeeded or the installed version is already current, OpenCode owns this workspace's tab label, and either this workspace's terminal target is Grok and that process is still alive or this workspace's shell has at least one open Grok session
- When the user reveals About diagnostics
- Then the Grok row's pill is ready in the success tone and its sentence is `Grok Build session follow is ready.`

#### Scenario: Grok Build is not running

- Given the shipped plugin install succeeded or the installed version is already current, this workspace's terminal target is not a living Grok process, and this workspace's shell has no open Grok session
- When the user reveals About diagnostics
- Then the Grok row's pill is inactive and its sentence is `Grok Build is not running.`

#### Scenario: Another workspace's Grok does not ready this row

- Given the shipped plugin install succeeded or the installed version is already current, Grok is running in another workspace, and this workspace's terminal target is not a living Grok process and this workspace's shell has no open Grok session
- When the user reveals this workspace's About diagnostics
- Then this Grok row's pill is inactive and its sentence is `Grok Build is not running.`

#### Scenario: Grok row shows hooks unavailable

- Given the install failed
- When the user reveals About diagnostics
- Then the Grok row's pill is degraded and its sentence is `Grok Build session-status hooks are unavailable.`

#### Scenario: Grok plugin was not installed for this launch

- Given an explicit catalog includes Grok and PATH detection did not run
- When the user reveals About diagnostics
- Then the Grok row's pill is inactive, its sentence is `Grok Build plugin was not installed for this launch.`, and the row does not claim the plugin is ready or that an install is running

#### Scenario: Copied diagnostics include the Grok sentence

- Given About diagnostics are revealed in any Grok row state
- When the user copies the diagnostics report
- Then the copied report includes the same Grok sentence shown in the row and includes the OpenCode sentence shown on the OpenCode row

### Requirement: OpenCode diagnostics wording

The OpenCode diagnostics row MUST use the same visible wording pattern as the Grok Build row for the not-running, ready, and not-on-PATH cases. While OpenCode session follow is not connected and no other OpenCode reason applies, the OpenCode pill MUST stay inactive and its sentence MUST be `OpenCode is not running.` While OpenCode session follow is connected, including after a reconnect, the visible pill MUST say `ready` in the existing success tone and the sentence MUST be `OpenCode session follow is ready.` The machine-readable status code MAY stay `connected`, and that code is what connected status means in Diagnostics. When OpenCode is not on PATH, the sentence MUST be `OpenCode is not on PATH.` and the pill MUST stay the existing degraded or unsupported treatment that already applies to that reason. The focus-unavailable sentence, wherever it is shown (the session-map focus notice and the diagnostics row), MUST be `Focus is unavailable: OpenCode is not connected.` Every other OpenCode reason sentence MUST stay as it is, including heartbeat, protocol, and directory messages. Every Grok Build sentence required above MUST stay as stated. The OpenCode row MUST NOT show `Optional OpenCode integration is inactive.` This wording MUST NOT change OpenCode launch, follow, or dots, and MUST NOT change the Grok Build tab label or menu row.

#### Scenario: OpenCode is not running

- Given OpenCode session follow is not connected and no other OpenCode reason applies
- When the user reveals About diagnostics
- Then the OpenCode pill stays inactive and its sentence is `OpenCode is not running.`

#### Scenario: OpenCode session follow is ready

- Given OpenCode session follow is connected
- When the user reveals About diagnostics
- Then the visible pill says `ready` in the existing success tone, the sentence is `OpenCode session follow is ready.`, and the machine-readable status code may still be `connected`

#### Scenario: OpenCode session follow is ready after a reconnect

- Given OpenCode session follow has reconnected
- When the user reveals About diagnostics
- Then the visible pill says `ready` in the existing success tone, the sentence is `OpenCode session follow is ready.`, and the machine-readable status code may still be `connected`

#### Scenario: OpenCode is not on PATH

- Given OpenCode is not on PATH
- When the user reveals About diagnostics
- Then the sentence is `OpenCode is not on PATH.` and the pill stays the existing degraded or unsupported treatment that already applies to that reason

#### Scenario: Other OpenCode reason sentences stay

- Given OpenCode diagnostics are showing a heartbeat, protocol, or directory reason, or any other OpenCode reason that is not the not-running, ready, or not-on-PATH case
- When the user reveals About diagnostics
- Then that reason sentence stays as it is

#### Scenario: Grok sentences stay

- Given the OpenCode row is showing any of its wording cases
- When the user reveals About diagnostics
- Then each Grok sentence stays the one already required for that Grok state

#### Scenario: Focus unavailable names OpenCode

- Given OpenCode is not connected in the tab and the session map still lists a session
- When the user activates that session in the map and then reveals About diagnostics
- Then the focus notice and the diagnostics reason both read `Focus is unavailable: OpenCode is not connected.`
