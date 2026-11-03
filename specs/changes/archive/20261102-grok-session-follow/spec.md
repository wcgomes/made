# Change Spec: grok-session-follow

## ADDED Requirements

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

Made MUST ship the Grok plugin at `plugins/grok-made`. The user removes it with `grok plugin uninstall grok-made --confirm`. Made MUST NOT hand-edit the user's Grok `config.toml`. The plugin MUST append one JSON line per session event to `$GROK_HOME/made-grok-status.jsonl`, read one JSON value without waiting for stdin to close, and exit successfully without denying a tool or a prompt. Made MUST apply only lines appended after Made starts. Each line MUST carry the session id, directory, event, and status `idle`, `working`, `attention`, or `error`.

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

### Requirement: Shipped Grok plugin install

After PATH detection reports grok, Made MUST ensure the shipped `plugins/grok-made` package is installed by running `grok plugin install <absolute shipped path> --trust` with the grok executable that lookup found. The shipped package is the `plugins/grok-made` directory beside `plugins/opencode-made`, in the development repository or the install-root copy in a release, and `<absolute shipped path>` is that directory's absolute path. That command is not part of launching Grok and is not a hand edit of `config.toml`. Made MUST compare the `version` field of the shipped `plugin.json` with the `version` field of the installed grok-made `plugin.json` as dotted numeric versions. Equal versions MUST skip the install command. A missing grok-made install, or an installed version lower than the shipped version, MUST run that command. An installed version higher than the shipped version MUST be left in place. The version check and the install command MUST be separate from the metadata-only PATH lookup and MUST NOT block shell, explorer, or Grok startup. Supplying a catalog to the menu without running PATH detection MUST NOT install the plugin. If the install command fails, Made MUST still let the user start Grok, and Made MUST record for Grok session follow that session-status hooks are unavailable. Made MUST NOT hand-edit the user's Grok `config.toml` and MUST NOT pass OpenCode's plugin environment to Grok.

#### Scenario: Missing or older plugin is installed

- Given PATH detection reports grok and grok-made is missing or its installed `plugin.json` version is lower than the shipped version
- When Made ensures the shipped plugin
- Then it runs `grok plugin install` with the absolute path of the shipped `plugins/grok-made` and `--trust`, and it does not hand-edit `config.toml`

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

When OpenCode is not connected in a workspace tab and that tab has open Grok sessions, the tab MUST paint one status dot per open Grok session using the same size, placement, cap of eight, pulse, colors, tooltip summary, and accessible-name summary as OpenCode session dots. Working is busy, attention and error are attention, and idle is idle. A session present only in the inventory, before any hook event, MUST show idle. While OpenCode is not active, a tab whose target is a running Grok TUI, or a shell tab with open Grok sessions, MUST use the label `Grok Build` and the official 16px `grok.svg` mark in the leading icon slot, even before a session exists. Its tooltip MUST be the session summary when sessions exist and MUST omit the workspace path. When that Grok is no longer running and the tab has no open Grok sessions, the label and leading icon MUST return to the folder label and terminal glyph. If OpenCode is active, the label, leading icon, and dots MUST stay OpenCode's even when Grok sessions exist.

#### Scenario: Dots match session state

- Given a Grok tab whose sessions are working, waiting on a question, and idle
- When the workspace tab is displayed
- Then it shows a pulsing busy dot, a yellow attention dot, and a neutral idle dot, the label is `Grok Build`, the leading icon is the official `grok.svg` mark, and the tooltip counts those three categories without the workspace path

### Requirement: Grok yields to OpenCode and returns to the shell

A connected OpenCode session MUST keep the explorer root and the workspace-tab dots. Grok MUST NOT replace them. While OpenCode is not connected, an open Grok session's directory MUST win over the shell directory. When no Grok session remains open for that terminal, the explorer MUST return to the shell directory. Selecting an already-open Grok session, with no new attach and no new prompt, MUST NOT move the explorer.

#### Scenario: OpenCode remains first

- Given OpenCode is connected in a tab and Grok sessions are also open there
- When Grok events arrive
- Then the explorer root and the tab dots stay on OpenCode

#### Scenario: Grok exit restores the shell

- Given the explorer is following a Grok session
- When that terminal no longer has an open Grok session
- Then the explorer returns to the shell directory

#### Scenario: Focus alone does not move the explorer

- Given two Grok sessions are already open and the explorer follows one of them
- When the user only focuses the other session in the Grok dashboard
- Then the explorer stays where it was

### Requirement: Catalog-driven new-tab menu

The `+` control MUST keep the tooltip and accessible name Open folder, and its behavior MUST be driven by the integrated-TUI catalog rather than by a special case for one program. Detecting a catalog entry MUST be a metadata-only PATH lookup that never spawns the program, never checks a version, and never installs a plugin, and that lookup MUST NOT delay shell or explorer startup. Empty and relative PATH segments MUST be ignored. When no integrated TUI is detected, `+` MUST open the folder picker and MUST show no menu. When any integrated TUI is detected, `+` MUST open a menu whose first row is Terminal, using the existing terminal glyph and the existing folder-picker flow, then one row for each detected integrated TUI in catalog order. Catalog order MUST be OpenCode, then Grok, and a program that is not on PATH MUST be omitted. Choosing a TUI row MUST start that executable's absolute path directly as the child of a new home-directory tab, with no shell first and no folder picker, and the current tab MUST keep running. Made MUST NOT launch an integrated TUI unless the user chooses its row. The OpenCode row MUST keep the vendored 16px opencode.svg and MUST keep its existing launch, including its per-launch plugin configuration injection, follow, and dots. The Grok row label MUST be Grok Build, and its 16px leading slot MUST paint the official mark in `grok.svg` (the two paths from https://grok.com/images/favicon.svg, fill #FCFCFC, viewBox "56 56 400 400") rather than the terminal glyph. Choosing Grok MUST launch grok's absolute path with the single argument `dashboard`. That launch MUST NOT inject plugin configuration, including OpenCode's plugin environment, and MUST NOT write the user's Grok config. Claude Code and Codex MUST stay out of the catalog. Typing grok in a shell MUST remain valid, MUST NOT be replaced by the menu, and MUST NOT be given the `dashboard` argument.

#### Scenario: No detected TUI opens the folder picker

- Given no integrated TUI is on PATH
- When the user activates `+`
- Then the folder picker opens and no menu is shown

#### Scenario: Catalog menu lists detected TUIs in order

- Given OpenCode and Grok are both on PATH
- When the user activates `+`
- Then the menu is Terminal, then OpenCode, then Grok, and the `+` tooltip and accessible name stay Open folder

#### Scenario: Only Grok is installed

- Given Grok is on PATH and OpenCode is not
- When the user activates `+`
- Then the menu is Terminal, then Grok, and OpenCode is omitted

#### Scenario: Grok row uses the official mark

- Given the menu includes Grok
- When the Grok row is shown
- Then its label is Grok Build and its 16px leading slot paints the official mark from `grok.svg`

#### Scenario: OpenCode row keeps its icon and launch

- Given OpenCode is on PATH
- When the user chooses the OpenCode row
- Then the row shows the vendored 16px opencode.svg, Made starts OpenCode's absolute path directly in a new home-directory tab with no shell first, and OpenCode's existing launch injection, follow, and dots stay in place

#### Scenario: Choosing Grok starts grok at home

- Given a tab is running and Grok is on PATH
- When the user chooses Grok
- Then Made starts grok's absolute path plus the argument `dashboard` directly as the child of a new home-directory tab, with no shell first and no folder picker, the current tab keeps running, that launch does not inject plugin configuration or write the user's Grok config, and typing grok in a shell is not given that argument

#### Scenario: Terminal row still opens the folder picker

- Given at least one integrated TUI is detected
- When the user chooses Terminal
- Then the existing folder-picker flow runs and no integrated TUI is launched

#### Scenario: Detection does not launch or block startup

- Given an integrated TUI executable is on PATH
- When Made starts the shell and explorer
- Then the lookup does not spawn that program, does not check its version, ignores empty and relative PATH segments, and startup does not wait on the lookup

#### Scenario: PATH lookup does not install

- Given grok is on PATH
- When Made runs the metadata-only PATH lookup
- Then that lookup does not spawn grok and does not install the plugin

#### Scenario: Menu does not replace a shell launch

- Given Grok is on PATH
- When the user types grok in a shell tab and does not choose the Grok row
- Then Made does not launch Grok on its own, and that shell session remains a valid way to start Grok

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

The OpenCode diagnostics row MUST use the same visible wording pattern as the Grok Build row for the not-running, ready, and not-on-PATH cases. While OpenCode session follow is not connected and no other OpenCode reason applies, the OpenCode pill MUST stay inactive and its sentence MUST be `OpenCode is not running.` While OpenCode session follow is connected, including after a reconnect, the visible pill MUST say `ready` in the existing success tone and the sentence MUST be `OpenCode session follow is ready.` The machine-readable status code MAY stay `connected`, and that code is what connected status means in Diagnostics. When OpenCode is not on PATH, the sentence MUST be `OpenCode is not on PATH.` and the pill MUST stay the existing degraded or unsupported treatment that already applies to that reason. Every other OpenCode reason sentence MUST stay as it is, including focus, heartbeat, protocol, and directory messages. Every Grok Build sentence required above MUST stay as stated. The OpenCode row MUST NOT show `Optional OpenCode integration is inactive.` This wording MUST NOT change OpenCode launch, follow, or dots, and MUST NOT change the Grok Build tab label or menu row.

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

- Given OpenCode diagnostics are showing a focus, heartbeat, protocol, or directory reason, or any other OpenCode reason that is not the not-running, ready, or not-on-PATH case
- When the user reveals About diagnostics
- Then that reason sentence stays as it is

#### Scenario: Grok sentences stay

- Given the OpenCode row is showing any of its wording cases
- When the user reveals About diagnostics
- Then each Grok sentence stays the one already required for that Grok state

## MODIFIED Requirements

### Requirement: V1 Scope Boundary

Made MUST support OpenCode v2 and the Grok Build session follow defined by this change. The `+` menu MUST be where OpenCode and Grok are offered, each only when it is detected on PATH. Made MUST NOT integrate with any other agent TUI. Made MUST NOT require remote, SSH, or WSL-hosted OpenCode or Grok. Made MUST NOT provide an AI or chat UI that replaces either TUI. Made MUST NOT create, remove, merge, or orchestrate worktrees. Made MUST NOT require a separate OpenCode or Grok server process.

#### Scenario: Unsupported agent not offered

- Given the `+` menu
- When the user looks for agent integrations
- Then OpenCode and Grok are the only agent rows the menu can offer, and Claude Code, Codex, ACP, and any other agent integration are not presented

#### Scenario: No replacement chat UI

- Given the Made interface
- When the user looks for an AI or chat surface
- Then no AI or chat UI replaces the OpenCode or Grok TUI

#### Scenario: Worktree lifecycle remains outside Made

- Given a worktree
- When the user inspects Made's controls
- Then no create, remove, merge, or orchestration action is available

#### Scenario: Direct launch

- Given a user wants to use OpenCode or Grok with Made
- When they run that command directly in the Made terminal
- Then follow works without Made starting a separate server or attach process, and the `+` menu does not replace that direct launch
