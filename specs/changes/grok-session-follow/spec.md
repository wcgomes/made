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

Made MUST ship the Grok plugin at `plugins/grok-made`. The user installs it with `grok plugin install` and removes it with `grok plugin uninstall grok-made --confirm`. Made MUST NOT write the user's Grok configuration. The plugin MUST append one JSON line per session event to `$GROK_HOME/made-grok-status.jsonl`, read one JSON value without waiting for stdin to close, and exit successfully without denying a tool or a prompt. Made MUST apply only lines appended after Made starts. Each line MUST carry the session id, directory, event, and status `idle`, `working`, `attention`, or `error`.

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

The `+` control MUST keep the tooltip and accessible name Open folder, and its behavior MUST be driven by the integrated-TUI catalog rather than by a special case for one program. Detecting a catalog entry MUST be a metadata-only PATH lookup that never spawns the program and never checks a version, and that lookup MUST NOT delay shell or explorer startup. Empty and relative PATH segments MUST be ignored. When no integrated TUI is detected, `+` MUST open the folder picker and MUST show no menu. When any integrated TUI is detected, `+` MUST open a menu whose first row is Terminal, using the existing terminal glyph and the existing folder-picker flow, then one row for each detected integrated TUI in catalog order. Catalog order MUST be OpenCode, then Grok, and a program that is not on PATH MUST be omitted. Choosing a TUI row MUST start that executable's absolute path directly as the child of a new home-directory tab, with no shell first and no folder picker, and the current tab MUST keep running. Made MUST NOT launch an integrated TUI unless the user chooses its row. The OpenCode row MUST keep the vendored 16px opencode.svg and MUST keep its existing launch, including its per-launch plugin configuration injection, follow, and dots. The Grok row label MUST be Grok Build, and its 16px leading slot MUST paint the official mark in `grok.svg` (the two paths from https://grok.com/images/favicon.svg, fill #FCFCFC, viewBox "56 56 400 400") rather than the terminal glyph. Choosing Grok MUST launch grok's absolute path with the single argument `dashboard` and MUST NOT inject plugin configuration into that launch or write the user's Grok configuration. Claude Code and Codex MUST stay out of the catalog. Typing grok in a shell MUST remain valid, MUST NOT be replaced by the menu, and MUST NOT be given the `dashboard` argument.

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
- Then Made starts grok's absolute path plus the argument `dashboard` directly as the child of a new home-directory tab, with no shell first and no folder picker, the current tab keeps running, Made does not inject plugin configuration or write the user's Grok configuration, and typing grok in a shell is not given that argument

#### Scenario: Terminal row still opens the folder picker

- Given at least one integrated TUI is detected
- When the user chooses Terminal
- Then the existing folder-picker flow runs and no integrated TUI is launched

#### Scenario: Detection does not launch or block startup

- Given an integrated TUI executable is on PATH
- When Made starts the shell and explorer
- Then the lookup does not spawn that program, does not check its version, ignores empty and relative PATH segments, and startup does not wait on the lookup

#### Scenario: Menu does not replace a shell launch

- Given Grok is on PATH
- When the user types grok in a shell tab and does not choose the Grok row
- Then Made does not launch Grok on its own, and that shell session remains a valid way to start Grok

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
