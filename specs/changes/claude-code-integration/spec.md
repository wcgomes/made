# Change Spec: claude-code-integration

This delta adds the new domain `specs/claude-code-session-follow.md`, which receives every ADDED requirement below except `Remote Claude Code launch and status (SSH-11)`, which is added to `specs/ssh-remote-workspaces.md`. It modifies `specs/grok-session-follow.md` (requirements `Catalog-driven new-tab menu`, `Grok tab dots` and `Grok yields to OpenCode and returns to the shell`), `specs/opencode-session-follow.md` (requirements `Shell Directory Follow` and `V1 Scope Boundary`), `specs/made-ui-ux.md` (requirement `Brand and Workspace Tabs`) and `specs/ssh-remote-workspaces.md` (requirement `SSH server entry and launch choices (SSH-01)`). Each MODIFIED requirement below replaces the realized requirement of the same name in full, scenarios included. It modifies no requirement that the active change `grok-native-hook` modifies. The new domain is authoritative for the Claude Code diagnostics row, as `specs/grok-session-follow.md` is for the Grok Build row; `Diagnostics Presentation` in `specs/made-ui-ux.md` is not modified because it does not limit revealed diagnostics to the OpenCode pill.

Claude Code presence, used throughout, is defined by the requirement `Claude Code presence`.

## ADDED Requirements

### Requirement: Claude Code presence

Claude Code MUST be treated as present in a local tab while any of these holds: the tab's directly launched Claude Code child is still running; the tab has at least one live owned Claude Code session as defined by `Claude Code session ownership and liveness`; or the tab received a session-level `app=claude-code` status emission that has not since been ended by a session-level clear emission, by the tab's terminal child exiting, or by the tab's launch-local shell integration reporting that the shell is back at its prompt. Claude Code is present in a workspace when it is present in that workspace's tab; presence in another workspace MUST NOT count. Presence alone never supplies a directory.

#### Scenario: Each source makes Claude Code present

- Given three local tabs: one launched from the Claude Code row before any status or hook record, one shell tab with a live owned Claude Code session and hooks only, and one shell tab whose Claude Code has hooks disabled and sends a session-level `state=working`
- When presence is evaluated for each tab
- Then Claude Code is present in all three tabs and in no other tab

#### Scenario: Protocol-only presence ends

- Given a shell tab where Claude Code is present only through a session-level status emission
- When a session-level clear arrives, and in other runs the terminal child exits or the shell integration reports the prompt again
- Then Claude Code is no longer present in that tab in each run

#### Scenario: Presence in another workspace does not count

- Given Claude Code is present in workspace B only
- When presence is evaluated for workspace A
- Then Claude Code is not present in workspace A

### Requirement: Claude Code launch

Claude Code MUST be an integrated-TUI catalog entry after Grok. Detecting it MUST be the catalog's metadata-only PATH lookup for the executable `claude`, which never spawns it, never checks its version, never loads or installs a plugin, ignores empty and relative PATH segments, and does not delay shell or explorer startup. When `claude` is detected, the `+` menu MUST show a row labeled `Claude Code` after every other detected catalog row, whose 16px leading slot paints the Claude Code mark defined by `Claude Code tab presentation`. Choosing that row MUST start the detected `claude` absolute path with no arguments directly as the child of a new home-directory tab, with no shell first and no folder picker, and the current tab MUST keep running. That launch MUST carry the session-only plugin loading of `Session-only Claude Code plugin`, MUST NOT carry OpenCode's plugin environment, and MUST NOT add any command-line argument. Made MUST NOT launch Claude Code unless the user chooses its row. Typing `claude` in a local shell tab MUST remain valid, MUST NOT be replaced by the menu, and MUST receive the same session-only plugin loading.

#### Scenario: Catalog lists Claude Code last

- Given OpenCode, grok and claude are all on PATH
- When the user activates `+`
- Then the menu is Terminal, OpenCode, Grok Build, Claude Code, and the `+` tooltip and accessible name stay Open folder

#### Scenario: Only Claude Code is installed

- Given claude is on PATH and neither OpenCode nor grok is
- When the user activates `+`
- Then the menu is Terminal, then Claude Code, and the Claude Code row paints the Claude Code mark in its 16px leading slot

#### Scenario: Choosing Claude Code starts claude at home

- Given a tab is running and claude is on PATH
- When the user chooses the Claude Code row
- Then Made starts claude's absolute path with no arguments directly as the child of a new home-directory tab, with no shell first and no folder picker, the current tab keeps running, and the launched Claude Code loads Made's session-only plugin and no OpenCode plugin environment

#### Scenario: Detection does not run claude

- Given claude is on PATH
- When Made starts the shell and explorer and runs PATH detection
- Then the lookup does not spawn claude, does not check its version, does not load or install a plugin, ignores empty and relative PATH segments, and startup does not wait on it

#### Scenario: Shell-typed claude stays valid

- Given a local shell tab and claude on PATH
- When the user types `claude` in that shell and does not choose the Claude Code row
- Then Made does not launch Claude Code on its own, the typed claude starts normally with Made's session-only plugin loaded, and the shell remains a valid way to start Claude Code

### Requirement: Program status protocol in the terminal

Every Made terminal tab, local or SSH, MUST implement the receiving side of the Program Status Protocol (OSC 7501) as Claude Code uses it. When any program in the tab sends the protocol's capability probe, Made MUST answer it by writing exactly the protocol's capability reply to that tab's terminal input, at most once per terminal within one second, so that a terminal line that echoes the reply back as output cannot start an endless exchange, and MUST write nothing else to the terminal in response to any protocol sequence. The probe reply MUST be written before any reply to a query that follows the probe in the output stream. Made MUST consume every protocol sequence, whether terminated by BEL or ST, without rendering it as text and without changing screen contents, scrollback, cursor position or the window title. An emission is a list of `key=value` fields; the fields Claude Code sends are `state`, `app`, `id`, `kind`, `title` and `msg`. Made MUST act only on session-level emissions, which are those without an `id` field; an emission with an `id` describes a sub-task row and MUST NOT change any Made state, including a clear for that `id`. A session-level emission whose `app` is `claude-code` sets the tab's Claude Code protocol status from its `state`: `working` is busy, `blocked` is attention, `error` is attention, `done` is idle and `idle` is idle, and `clear` ends the protocol status. A session-level `state=clear` without an `app` field MUST also end the protocol status, but only while a Claude Code protocol status is live in that tab, and MUST otherwise be ignored. The `kind` and `title` fields MUST be ignored. The `msg` field can contain tool commands and MUST NOT be kept, stored, logged, displayed or copied. An emission with an unknown `state` value, a missing or empty `app` other than that bare clear, a malformed field, or a total length over 4096 bytes MUST NOT change any status, presentation or explorer root. An unknown field name MUST be ignored and MUST NOT by itself invalidate an otherwise valid emission. Session-level emissions for any other `app` MUST NOT change any Made state or presentation. Protocol status MUST apply only to the tab whose terminal received it and never supplies a directory or a title. Made MUST NOT derive status from window-title text or glyphs, including the glyph prefixes Claude Code adds to its title when it receives no probe reply.

#### Scenario: Probe is answered

- Given a local tab and an SSH tab, each running a program that sends the OSC 7501 capability probe
- When the probe reaches Made's terminal
- Then Made writes exactly the protocol's capability reply to that tab's terminal input, writes nothing else, and the probe is not rendered

#### Scenario: Echoed probe reply is not answered again

- Given a tab whose foreground program leaves terminal echo on, so the capability reply comes back as terminal output
- When the echoed reply reaches Made's terminal within one second of the reply Made wrote
- Then Made writes no further reply, no matter how many times the echo repeats, and a probe received after that second is answered again

#### Scenario: Probe reply precedes device attributes reply

- Given a program writes the capability probe followed by a device-attributes query in one output chunk
- When the chunk reaches Made's terminal
- Then the bytes written to that tab's terminal input contain the capability reply before the device-attributes reply

#### Scenario: Probe from another program gets only the reply

- Given OpenCode, Grok or another non-Claude program sends the capability probe
- When Made answers it
- Then that program receives only the standard capability reply, and no tab label, icon, dot, tooltip, explorer root or diagnostics row changes

#### Scenario: States map to dot categories

- Given Claude Code is running in a tab and emits `app=claude-code` emissions with state `working`, then `blocked`, then `working`, then `done`, then `error`, then `idle`
- When each emission is received
- Then the tab's Claude Code dot is busy, attention, busy, idle, attention and idle in turn, and no emission text is rendered

#### Scenario: Clear emission removes protocol status

- Given a tab whose Claude Code protocol status is busy
- When a session-level `state=clear` for `claude-code` arrives, and in another run the user exits Claude Code, which sends `working` and then bare `state=clear` emissions without an `app` field
- Then in both runs the protocol status is removed, and the dot falls back to the hook-derived status when a live owned session exists, or the protocol-only presence ends

#### Scenario: Bare clear without live status is ignored

- Given a tab with no live Claude Code protocol status, such as a plain shell or a Grok tab
- When a session-level `state=clear` without an `app` field arrives
- Then no Made state, label, dot or diagnostics row changes

#### Scenario: Sub-task rows are ignored

- Given Claude Code is running with session-level protocol status busy
- When emissions with an `id` field arrive for sub-task rows with states `working` and `done` and base64 titles, and later `state=clear` for that `id`
- Then the tab's Claude Code dot stays busy, no extra dot or row appears, and no sub-task title is displayed

#### Scenario: Unknown state and unknown field

- Given Claude Code is running with protocol status busy
- When an emission arrives with an unknown `state` value, and separately a valid `state=done` emission arrives with an extra unknown field
- Then the unknown-state emission changes nothing, and the emission with the extra field sets the dot to idle with the extra field ignored

#### Scenario: Other app emissions are ignored

- Given a tab where a program emits status emissions with an `app` other than `claude-code`
- When those emissions arrive
- Then no Made status, title, label, icon, dot, tooltip, explorer root or diagnostics row changes and nothing is rendered

#### Scenario: Malformed or oversized emissions are ignored

- Given Claude Code is running with protocol status idle
- When emissions arrive with a missing `app` and a non-clear state, a malformed field, and a total length over 4096 bytes
- Then none of them changes status, title, presentation or the explorer root, none is rendered, and the terminal stays usable

#### Scenario: Protocol status stays in its tab

- Given Claude Code is running in tab A and tab B is a plain shell
- When tab A receives `state=blocked`
- Then only tab A's dot becomes attention, and tab B's label, dots and explorer root are unchanged

### Requirement: Session-only Claude Code plugin

Made MUST ship a Claude Code plugin, including its native hook, in the installed layout of every local client target, resolved from Made's installation root, never downloaded at runtime, and needing nothing beyond what Claude Code itself provides; where the OS uses an executable permission, the shipped hook MUST keep it in the installed layout and in development runs. Made MUST load that plugin only for Claude Code started inside Made's local tabs, through Claude Code's session plugin-directory environment variable `CLAUDE_CODE_PLUGIN_DIRS`, by adding the shipped plugin's directory to that variable in the environment of every local shell tab and every directly launched Claude Code tab. When the variable already has a value in Made's environment, Made MUST keep every existing entry in its original order and add its own entry, MUST NOT add its entry a second time when it is already present, and MUST NOT overwrite, reorder or drop any user entry. Made MUST NOT add the variable to directly launched OpenCode or Grok tabs, to SSH tabs or remote processes, or to any process it does not start for a local tab. The plugin MUST load in Claude Code without a plugin validation error or warning, and MUST NOT disable, replace or reorder the user's own plugins or hooks. Made MUST NOT create, edit or delete any file in the user's Claude Code configuration, including settings, plugin registry, marketplaces, hooks and statusline, MUST NOT run `claude plugin install` or any other `claude` subcommand, and MUST NOT pass Claude Code command-line arguments. Claude Code started outside Made MUST be unaffected. If the shipped plugin is missing or unreadable, Made MUST still start shells and Claude Code, MUST NOT add an entry that points to it, and MUST record that Claude Code session-status hooks are unavailable.

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
- Then none of those launches has Made's entry added to `CLAUDE_CODE_PLUGIN_DIRS`, and the Grok launch still injects no plugin configuration

#### Scenario: No persistent Claude Code configuration change

- Given the user has Claude Code settings, installed plugins, marketplaces and hooks
- When the user runs Claude Code in Made tabs, Made restarts, and the user runs Claude Code in an ordinary terminal outside Made
- Then Made never ran a `claude` subcommand, no file in the user's Claude Code configuration was created, edited or deleted, the user's own plugins and hooks still run in Made, and the Claude Code outside Made does not load Made's plugin

#### Scenario: Missing shipped plugin

- Given the shipped plugin is missing or unreadable in the installation
- When the user opens a shell tab and starts Claude Code there, and separately chooses the Claude Code row
- Then shells and Claude Code still start, no entry pointing to the missing plugin is added, and Made records that Claude Code session-status hooks are unavailable

### Requirement: Claude Code session events

The shipped plugin's hook MUST run asynchronously for these Claude Code events: session start, session end, prompt submission, turn stop, turn stop failure, permission request, notification, and working-directory change. The hook MUST be a native executable built for the local client target it ships with, MUST run on macOS arm64, macOS x64 and Linux glibc x64, and SHOULD run on Windows x64; where Claude Code on Windows cannot run the hook, Windows MUST fail open with session-status hooks reported unavailable. Running the hook MUST NOT require an interpreter, language runtime or separately installed package. The hook MUST NOT delay, block, deny or alter any prompt, tool, permission or session, MUST write nothing to standard output, and MUST exit successfully for every input and every failure. For each event it MUST append at most one record to the spool of the Made tab whose environment started that Claude Code, and MUST write nothing when it cannot identify that spool. A record MUST carry only the time, the Claude Code session id, the working directory that event reports, the event kind, the notification type, the derived status, the session title when the event carries one, and the process ancestry needed by `Claude Code session ownership and liveness`; it MUST NOT carry prompt text, tool names, tool inputs or outputs, notification message text, transcript paths or contents, model output, environment values, or any other payload field. The derived status MUST be: session start is idle; prompt submission is working; permission request is attention; a permission-prompt notification is attention; an idle-prompt notification is idle; turn stop is idle; turn stop failure is error; working-directory change keeps the session's current status; session end ends the session. A session title carried by an event, such as the first prompt submission after the user renames the session, is untrusted and MUST be sanitized by replacing control characters and newlines with spaces and removing bidirectional and format control characters and truncated to at most 120 characters before it is recorded. Each spool MUST be in Made's private per-user application directory and readable and writable only by the current user (directory mode 0700 and file mode 0600 on POSIX), MUST NOT follow a symlink, MUST stay at or below 1 MiB, and MUST NOT receive a record larger than 4 KiB. A full, missing or unwritable spool MUST NOT block or fail Claude Code, and reaching the size bound MUST NOT stop later events from being applied. Made MUST apply only records appended after the tab started, MUST ignore malformed, truncated or oversized records without moving the explorer or changing presentation, MUST delete a tab's spool when the tab closes, MUST never apply a spool owned by another Made instance or an earlier run, and MUST remove a leftover spool only when the Made instance that owns it is no longer running, leaving every concurrently running instance's spools intact.

#### Scenario: Prompt submission is recorded without the prompt

- Given Claude Code with Made's plugin is running in a local tab
- When the user submits a prompt containing a unique marker string
- Then the tab's spool gains one record with status working and the session id and directory, and the marker string appears in no spool, log, diagnostics row or copied diagnostics report

#### Scenario: Permission request and notifications

- Given a Claude Code session is working in a local tab and the status protocol is unavailable
- When Claude Code requests a permission, later raises a permission-prompt notification, and later an idle-prompt notification
- Then the session's dot becomes attention on the request without waiting for the notification, stays attention on the permission-prompt notification, and becomes idle on the idle-prompt notification

#### Scenario: Turn stop and stop failure

- Given a Claude Code session is working in a local tab and the status protocol is unavailable
- When the turn stops, and in another turn the stop fails
- Then the dot is idle after the stop and attention after the failure

#### Scenario: Hook never blocks Claude Code

- Given the tab's spool is unwritable, its size bound is reached, or the hook receives malformed input
- When Claude Code fires any registered event
- Then Claude Code continues without delay, denial or visible error, the hook writes nothing to standard output and exits successfully, and later events are applied once the spool is writable again

#### Scenario: Spool is private and bounded

- Given Made is running on a POSIX system with Claude Code sessions in two tabs
- When the spools are inspected after many events
- Then each spool is in Made's private per-user directory with directory mode 0700 and file mode 0600, no spool exceeds 1 MiB, no record exceeds 4 KiB, and no symlinked spool path was written

#### Scenario: Stale, malformed and earlier records are ignored

- Given a tab's spool contains records written before the tab started, a truncated record and a malformed record, spools remain from an earlier Made run that has exited, and a second Made instance is running with its own spools
- When Made ingests the spool and starts up
- Then none of those records moves the explorer or changes presentation, the exited run's spools are removed without being applied, and the second instance's spools are neither applied nor removed

#### Scenario: Closing the tab removes its spool

- Given a local tab with a Claude Code spool
- When the user closes the tab
- Then that tab's spool is deleted and other tabs' spools are unchanged

### Requirement: Claude Code session ownership and liveness

A Claude Code session MUST belong to a tab only when the Claude Code process that ran its hook is that tab's terminal process or a descendant of it within 32 parent hops, determined from live process ancestry when Made ingests the record. Records whose Claude Code process is outside the tab, has already exited at first ingestion, or cannot be resolved MUST be ignored. Each live owned Claude Code process MUST have at most one current session: a session start with a new session id from the same process, after a clear, resume, fork or compact, MUST replace that process's previous session without leaving a second dot. A session MUST stop being live when its session end is recorded or when its Claude Code process exits, and Made MUST check owned processes for exit about once a second. Separate Claude Code processes in one tab MUST remain separate sessions.

#### Scenario: Claude Code in another tab is not claimed

- Given tab A is a plain shell and the user runs `claude` in tab B
- When tab B's Claude Code starts, receives prompts and changes directory
- Then tab A's label, dots and explorer root never change, and only tab B follows its own session

#### Scenario: Claude Code outside Made is not claimed

- Given a Claude Code process outside every Made tab's process tree whose environment was copied from a Made shell tab, including one started inside a tmux server that runs outside the tab's process tree
- When that Claude Code fires hook events into the copied tab's spool
- Then no tab claims the session, and no label, dot or explorer root changes

#### Scenario: Clear starts a new session id

- Given a live owned Claude Code session in a tab
- When the user runs `/clear` and Claude Code starts a new session id from the same process
- Then the new session replaces the previous one, the tab shows one Claude Code dot rather than two, and the explorer stays on the session directory

#### Scenario: Exit without session end

- Given a live owned Claude Code session whose process is killed before any session end is recorded
- When Made's liveness check runs
- Then within about one second the session stops being live and its dot is removed

### Requirement: Claude Code explorer follow and precedence

The directory of a live owned Claude Code session MUST drive its tab's explorer root. That directory is the working directory carried by the most recent owned record of that session, whatever its event, applied when the record is ingested, within about 250 ms of the record being written. Explorer root precedence per tab MUST be OpenCode, then Claude Code, then Grok, then the shell directory: a connected OpenCode session, or its usable connected view directory as `specs/opencode-session-follow.md` defines, keeps the root while Claude Code is live; a live owned Claude Code session directory wins over open Grok sessions and the shell directory. With more than one live owned Claude Code session in a tab, the directory of the session that most recently started, submitted a prompt or changed directory MUST win. When no live owned Claude Code session remains, the explorer MUST return to the next applicable source in that order. Reported directories are untrusted: they MUST be validated as local absolute paths, never executed or passed to a shell, and a relative, missing or unreadable directory MUST NOT move the explorer. Claude Code presence known only from the status protocol, without a live owned session, MUST NOT move or hold the explorer root.

#### Scenario: Session start moves the explorer

- Given a local shell tab whose explorer follows the shell directory
- When the user starts Claude Code there and its session starts in a different directory
- Then the explorer switches to that directory within about 250 ms of the session-start record being written

#### Scenario: Working-directory change moves the explorer

- Given the explorer follows a live owned Claude Code session
- When a later record of that session, of any event, reports a different working directory
- Then the explorer switches to the new directory

#### Scenario: Worktree session follows later records

- Given the user starts `claude -w` in a shell tab, whose session start reports the original directory and whose later records report the new worktree directory
- When the first later record arrives
- Then the explorer switches from the original directory to the worktree directory

#### Scenario: OpenCode in the same tab keeps the root

- Given OpenCode is connected in a tab where a live owned Claude Code session also exists
- When Claude Code events and status emissions arrive
- Then the explorer root, label, leading icon and dots stay OpenCode's, and when OpenCode disconnects while the Claude Code session is still live the root moves to the Claude Code session directory

#### Scenario: Claude Code outranks Grok

- Given a shell tab with an open Grok session whose directory is the explorer root
- When the user starts Claude Code in that tab and its session starts in another directory
- Then the explorer switches to the Claude Code session directory and Grok events do not move it back while that session is live

#### Scenario: Claude Code exit restores the next source

- Given the explorer follows a live owned Claude Code session, once in a tab that also has an open Grok session and once in a plain shell tab
- When Claude Code exits
- Then the first tab's explorer returns to the open Grok session's directory and the second tab's explorer returns to the shell directory

#### Scenario: Invalid directory does not move the explorer

- Given the explorer follows a live owned Claude Code session
- When a record reports a relative, missing or unreadable directory
- Then the explorer stays where it was and nothing in the report is executed

#### Scenario: Protocol-only presence does not hold the root

- Given Claude Code runs with hooks unavailable and sends status emissions, in a tab with an open Grok session
- When the emissions arrive
- Then the explorer follows the Grok session as before and Claude Code does not move or hold the root

### Requirement: Claude Code tab presentation

While OpenCode is not active in a tab and Claude Code is present there, the tab MUST use the label `Claude Code` and paint the Claude Code mark in its 16px leading icon slot, even before any session or status exists, and Grok Build presentation MUST yield to it. The Claude Code mark MUST be the official Claude Code mark, shipped as an app asset and painted at 16px in its official colors with the same treatment as the official Grok mark, without redrawing or tinting, and it MUST be the same asset the `+` menu row paints. The tab MUST paint one status dot per live owned Claude Code session, or exactly one dot when Claude Code is present with a protocol status and no live owned session, using the same size, placement, cap of eight, pulse, colors, tooltip summary and accessible-name summary as OpenCode session dots. A dot's status MUST be the tab's protocol status when one is set, applied to the tab's most recent live owned session, otherwise that session's hook-derived status, otherwise idle; busy is busy, attention and error are attention, and idle is idle. A finished turn MUST show idle. The tooltip MUST omit the workspace path and MUST be the session title, followed by ` — ` and the session summary when dots exist, where the session title is the first of these that is non-empty: the terminal's window title, read only while a Claude Code protocol status is live in that tab; the most recent session title carried by an owned record of the tab's most recent live owned session; `Claude Code`. The window title is untrusted and MUST be sanitized by replacing control characters and newlines with spaces and removing bidirectional and format control characters and truncated to at most 120 characters. The accessible name MUST be `Claude Code` followed by ` — ` and the session summary when dots exist. When Claude Code is no longer present and no Grok Build presentation applies, the label, tooltip and leading icon MUST return to the folder label, full-path tooltip and terminal glyph, and its dots MUST be removed. If OpenCode is active, the label, leading icon and dots MUST stay OpenCode's even while Claude Code is present.

#### Scenario: Direct launch is branded before any status

- Given the user chose the Claude Code row
- When the new tab is shown before any status emission or hook record arrives
- Then its label is `Claude Code`, its leading slot paints the Claude Code mark, it shows no dot, and its tooltip is `Claude Code` without the workspace path

#### Scenario: Dots follow protocol status with a title

- Given a live owned Claude Code session, a protocol emission with state `working`, and Claude Code setting its window title to `Refactor parser`
- When the tab is displayed
- Then it shows one pulsing busy dot, the tooltip is `Refactor parser — Sessions: 1 running`, and the accessible name is `Claude Code — Sessions: 1 running`

#### Scenario: Hook-only dots

- Given a live owned Claude Code session started with `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1`
- When the user submits a prompt and Claude Code later requests a permission
- Then the dot is busy and then attention, and the tooltip begins with `Claude Code`

#### Scenario: Window title only while protocol status is live

- Given Claude Code runs with the status protocol unavailable and sets its window title to `◐ Refactor parser`
- When the tab is displayed
- Then the tooltip begins with `Claude Code` rather than the window title, and no status is derived from the title or its glyph

#### Scenario: Renamed session title from hooks

- Given a live owned Claude Code session with the status protocol unavailable
- When the user renames the session to `foo` and submits the next prompt
- Then the tooltip begins with `foo`

#### Scenario: Empty window title falls back

- Given a live Claude Code protocol status and an owned session that has carried no session title
- When Claude Code sets an empty window title
- Then the tooltip begins with `Claude Code`

#### Scenario: Untrusted title is sanitized

- Given a window title read while protocol status is live, containing control characters, newlines, bidi controls and more than 120 characters
- When the tab tooltip and copied diagnostics show it
- Then controls and newlines are replaced, bidi and format controls are removed, and at most 120 characters are shown

#### Scenario: Claude Code exits and the tab returns to its folder

- Given a shell tab branded `Claude Code` with no open Grok session
- When Claude Code exits and the shell is back at its prompt
- Then the tab returns to the folder label, full-path tooltip and terminal glyph, and its Claude Code dots are removed

#### Scenario: Claude Code outranks Grok presentation

- Given a shell tab with two open Grok sessions showing `Grok Build` and Grok dots
- When the user starts Claude Code in that tab and later exits it
- Then while Claude Code is present the tab shows `Claude Code`, the Claude Code mark and only Claude Code dots, and after it exits the tab returns to `Grok Build`, the official Grok mark and the Grok dots

### Requirement: Claude Code channel fallback

The status protocol and the session hooks MUST each work without the other, and losing either MUST NOT break the terminal, shell, explorer, diagnostics or other tabs. With both channels, protocol status drives the dots, the window title read under `Claude Code tab presentation` supplies the tooltip title, and hook records drive session identity and the explorer. Without the status protocol, for example with a Claude Code version that does not emit it, inside tmux, in a background session, with `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1`, when no probe reply reaches Claude Code, or whenever Claude Code does not enable the protocol for another reason, hook-derived status drives the dots, the session title comes from hook records or is `Claude Code`, and explorer follow is unchanged. Without hooks, for example with `--bare`, safe mode, `disableAllHooks`, a managed-hooks-only policy, a missing shipped plugin, an unwritable spool, or Windows where the hook cannot run, protocol status and the window title drive the presentation, Claude Code does not move the explorer, and the explorer follows the next applicable source. With neither channel, the tab behaves as a plain terminal, except that a directly launched Claude Code tab keeps the `Claude Code` label and mark while its child runs. Made MUST NOT retry, relaunch, add arguments, change Claude Code configuration, drop, reorder or replace the user's `CLAUDE_CODE_PLUGIN_DIRS` entries, or prompt the user in order to restore a channel.

#### Scenario: No probe reply or tmux

- Given Claude Code receives no status-protocol channel to Made, once because it runs inside tmux started from a Made shell tab and once because the reply does not reach it in a tab where its hooks are owned
- When the user works in Claude Code
- Then in the owned-hooks case the dots come from hook-derived status and the explorer follows the session, and in the tmux case, where the Claude Code process runs outside the tab's process tree, the tab stays a plain terminal with no misattributed session, dot or explorer move

#### Scenario: Bare mode keeps protocol status only

- Given claude 2.1.295 or later started with `--bare` in a local shell tab
- When Claude Code emits `working` and then `done`
- Then the tab shows `Claude Code`, a busy then idle dot, the explorer stays on the shell directory, and the diagnostics row reports hooks unavailable

#### Scenario: Terminal title disabled keeps hooks only

- Given Claude Code started with `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1` in a local shell tab
- When its session starts, receives a prompt and stops
- Then the tab shows `Claude Code` with an idle, busy, then idle dot from hook-derived status, the explorer follows the session directory, and the diagnostics row reports the status protocol unavailable

#### Scenario: Status protocol not enabled while hooks work

- Given Made was started with a user entry in `CLAUDE_CODE_PLUGIN_DIRS`, so a shell tab's Claude Code receives that entry and Made's, and that Claude Code does not enable the status protocol
- When the user works in Claude Code in that tab
- Then hook-derived status drives the dot, the explorer follows the session, the diagnostics row reports `Claude Code status protocol is unavailable.`, and the user's entry stays in the variable in its original position

#### Scenario: Neither channel

- Given Claude Code runs with hooks disabled and the status protocol unavailable, once typed in a shell tab and once launched from the Claude Code row
- When the user works in Claude Code
- Then the shell-typed tab stays a plain terminal with its folder label, the directly launched tab keeps the `Claude Code` label and mark with no dot while its child runs, and neither tab's terminal, explorer or diagnostics breaks

### Requirement: Claude Code diagnostics row

When About diagnostics are revealed, Made MUST show exactly one Claude Code row for the workspace whose diagnostics are revealed, directly after the Grok Build row and with the existing OpenCode status pill, using that same pill-plus-sentence treatment, and not in the sectioned connection, session, worktree, version, metrics or log detail. The row MUST stay hidden until the user activates `Show diagnostics`, MUST be absent from the header, workspace tabs, explorer and terminal, and MUST NOT change the Claude Code tab label or menu row. The inactive, degraded and ready pills MUST use the existing inactive, degraded and success treatments. In this requirement, the status protocol is observed for a Claude Code when its tab has received at least one session-level `app=claude-code` emission since that Claude Code started; a capability probe alone MUST NOT count as observed. The copied diagnostics report MUST include the same Claude Code sentence the revealed row shows, and MUST NOT include prompt text, the protocol `msg` field or an unsanitized title.

The row MUST show exactly one state, the first of these that applies. While PATH detection is still running, the pill MUST be inactive and the sentence MUST be `Checking for Claude Code.`, and that state MUST NOT be shown as not on PATH. When detection has finished and claude is not on PATH, or an explicit catalog omits Claude Code and detection is not running, the pill MUST be inactive and the sentence MUST be `Claude Code is not on PATH.` When the shipped plugin is missing or unreadable, the pill MUST be degraded and the sentence MUST be `Claude Code session-status hooks are unavailable.` When Claude Code is not present in this workspace, the pill MUST be inactive and the sentence MUST be `Claude Code is not running.`; OpenCode owning the tab label MUST NOT by itself count as Claude Code not present, and Claude Code present only in another workspace MUST NOT change this row. When Claude Code became present in this workspace less than about 10 seconds ago and it has no live owned session or the status protocol is not yet observed, the pill MUST be inactive and the sentence MUST be `Claude Code is starting.` When Claude Code is present and has had no live owned session for at least that grace period, the pill MUST be degraded and the sentence MUST be `Claude Code session-status hooks are unavailable.` When a live owned session exists and the status protocol has not been observed for at least that grace period, the pill MUST be degraded and the sentence MUST be `Claude Code status protocol is unavailable.` Otherwise, with a live owned session and the status protocol observed, the pill MUST be ready in the success tone and the sentence MUST be `Claude Code session follow is ready.` An explicit catalog that includes Claude Code without running detection MUST use the plugin, running, starting, unavailable and ready states, because plugin loading does not depend on detection.

#### Scenario: Claude Code diagnostics stay hidden until revealed

- Given a workspace
- When the user opens About and has not activated `Show diagnostics`
- Then the Claude Code row is not shown, and with diagnostics revealed or not, the header, workspace tabs, explorer and terminal show no Claude Code pill or sentence

#### Scenario: Checking and not on PATH

- Given PATH detection is still running, and later has finished without finding claude
- When the user reveals About diagnostics each time
- Then the row first shows an inactive pill with `Checking for Claude Code.` and is not shown as not on PATH, and then shows an inactive pill with `Claude Code is not on PATH.`

#### Scenario: Not running and another workspace

- Given claude is on PATH, Claude Code is not present in this workspace and is present in another workspace
- When the user reveals this workspace's About diagnostics
- Then the row shows an inactive pill with `Claude Code is not running.`

#### Scenario: Starting

- Given the user chose the Claude Code row less than about 10 seconds ago, and either no hook record has arrived or only the capability probe, with no session-level `app=claude-code` emission, has arrived
- When the user reveals About diagnostics
- Then the row shows an inactive pill with `Claude Code is starting.`

#### Scenario: Ready

- Given a live owned Claude Code session in this workspace whose tab has received a session-level `app=claude-code` emission since that Claude Code started, including while OpenCode owns the tab label
- When the user reveals About diagnostics
- Then the row shows a ready pill in the success tone with `Claude Code session follow is ready.`

#### Scenario: Hooks unavailable

- Given Claude Code started with `--bare` in this workspace more than about 10 seconds ago, and separately an installation whose shipped plugin is missing
- When the user reveals About diagnostics
- Then each row shows a degraded pill with `Claude Code session-status hooks are unavailable.`

#### Scenario: Probe without emissions is not observed

- Given a live owned Claude Code session whose Claude Code sent the capability probe more than about 10 seconds ago and has sent no session-level `app=claude-code` emission
- When the user reveals About diagnostics
- Then the row shows a degraded pill with `Claude Code status protocol is unavailable.`

#### Scenario: Status protocol unavailable

- Given a live owned Claude Code session started with `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1` more than about 10 seconds ago
- When the user reveals About diagnostics
- Then the row shows a degraded pill with `Claude Code status protocol is unavailable.`

#### Scenario: Copied diagnostics include the Claude Code sentence

- Given About diagnostics are revealed in any Claude Code row state
- When the user copies the diagnostics report
- Then the report includes the same Claude Code sentence shown in the row, the Grok Build and OpenCode sentences shown on their rows, and no prompt text, `msg` content or unsanitized title

### Requirement: Claude Code privacy and observation boundary

Made MUST observe Claude Code only through the structured status-protocol emissions of `Program status protocol in the terminal` and the metadata-only records of `Claude Code session events`. Made MUST read the terminal window title only as the bounded tooltip title of `Claude Code tab presentation`, only while a Claude Code protocol status is live in that tab, and MUST NOT use a window title for status, identity or any shell or session directory. Made MUST NOT read or scrape screen or scrollback contents, prompts, Claude Code transcripts or session files, the IDE lockfile, status-line output, or Claude Code configuration to derive status, title, identity or directory. Made MUST NOT configure a Claude Code status line or HTTP hook, MUST NOT open a network listener for Claude Code, and MUST NOT send any Claude Code data off the device. Prompt text, the protocol `msg` field, notification message text, tool data and transcript content MUST NOT be stored, logged, displayed or copied by Made. Session titles, session ids and directories are untrusted: they MUST be sanitized by replacing control characters and newlines with spaces and removing bidirectional and format control characters before display, logging or copy, and directories MUST be validated before use and never executed or passed to a shell.

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

### Requirement: Existing integrations unchanged

OpenCode and Grok behavior defined by `specs/opencode-session-follow.md` and `specs/grok-session-follow.md` MUST stay unchanged except where this delta states Claude Code precedence: OpenCode launch, launch injection, follow, dots and diagnostics, and Grok launch with its `dashboard` argument, plugin install, inventory, hooks, dots and diagnostics. Directly launched OpenCode and Grok tabs MUST NOT receive Made's Claude Code plugin entry. The only Made output caused by status-protocol sequences from OpenCode, Grok or any other program MUST be the capability reply to a probe, and their other protocol emissions MUST NOT change Made state.

#### Scenario: OpenCode tab unchanged

- Given OpenCode launched from the menu or typed in a shell tab, with no Claude Code present
- When OpenCode connects, switches sessions and exits
- Then launch injection, explorer follow, label, icon, dots and diagnostics behave as before this change

#### Scenario: Grok tab unchanged

- Given Grok launched from the menu or typed in a shell tab, with no Claude Code present
- When Grok sessions start, receive prompts, wait on questions and end
- Then the `dashboard` launch, plugin install, inventory, hook dots, explorer follow, label, mark and diagnostics behave as before this change, and the Grok launch environment has no Made Claude Code plugin entry

#### Scenario: Other programs' protocol emissions

- Given OpenCode or Grok sends a status-protocol probe and emissions with its own app value
- When Made receives them
- Then Made sends only the standard capability reply and changes no state or presentation

### Requirement: Remote Claude Code launch and status (SSH-11)

Remote Claude Code support MUST be limited to launch and status-protocol status. Unlike SSH-10, which mirrors the local Grok Build contract, SSH-11 does not mirror the local Claude Code contract: the session-only plugin, hook records, spool, ownership and liveness, explorer follow, the hook-derived session title and the local presentation precedence over Grok are excluded for remote tabs. Remote claude detection MUST be the same bounded metadata-only remote PATH lookup as SSH-01: it MUST ignore empty and relative PATH entries, MUST NOT spawn claude, MUST NOT check a version, MUST NOT install or load a plugin, and MUST NOT delay terminal or explorer startup. Choosing `Claude Code SSH` MUST start the detected remote claude absolute path with no arguments directly as a new tab bound to that server and its selected remote directory, with no shell first and no separate folder picker, while the current tab keeps running. Made MUST NOT inject plugin configuration into that launch, MUST NOT set `CLAUDE_CODE_PLUGIN_DIRS` on the server, MUST NOT stage, install or run a Claude Code plugin or hook on the server, and MUST NOT write any file on the server for Claude Code. Typing `claude` in a supported remote shell MUST remain valid and MUST NOT be replaced by the menu. Status-protocol sequences from remote Claude Code reach Made through the remote terminal: Made MUST answer the capability probe through that same remote terminal and MUST apply `app=claude-code` emissions only to that remote tab, with the same mapping, bounds, sanitization and ignore rules as `Program status protocol in the terminal`. Claude Code is present in a remote tab while its directly launched remote claude is running, or while a session-level `app=claude-code` emission has not since been ended by a session-level clear emission, by the remote terminal child exiting, or, where remote shell directory reporting is available, by the remote shell reporting that it is back at its prompt. While remote OpenCode is not connected in the tab and SSH-10's Grok Build presentation conditions do not hold, a remote tab with Claude Code present MUST use the `Claude Code` label, the Claude Code mark, at most one dot from protocol status, and the local Claude Code tooltip and accessible-name rules; when Claude Code is no longer present it MUST return to the server-name label and terminal glyph. Remote Claude Code MUST NOT move the explorer and MUST NOT change the root precedence of SSH-06 and SSH-10. Disconnect MUST clear that tab's remote Claude Code status and title, and after reconnect the tab MUST wait for a fresh emission rather than replay earlier status. Revealed About diagnostics MUST show the single Claude Code row for a remote workspace driven by that tab's remote detection and presence rather than the host's: `Checking for Claude Code.` and `Claude Code is not on PATH.` from remote detection, `Claude Code is not running.` when not present, an inactive pill with `Claude Code is starting.` while Claude Code became present less than about 10 seconds ago and the status protocol is not yet observed as the local diagnostics requirement defines it, a degraded pill with `Claude Code status protocol is unavailable.` when the directly launched remote claude is running and the status protocol has not been observed for at least that grace period, and otherwise, while present, a degraded pill with `Claude Code session-status hooks are unavailable over SSH.`; the row MUST stay hidden until diagnostics are revealed, out of routine chrome, and included in copied diagnostics.

#### Scenario: Claude Code SSH row and direct launch

- Given server A has claude on its remote PATH and server B does not, both supported and reachable
- When the user opens each server's remote launch choices, chooses `Claude Code SSH` on A, and separately types `claude` in A's remote shell
- Then only A offers `Claude Code SSH`, choosing it starts A's detected claude with no arguments directly in a new tab bound to A and its selected remote directory with no shell first or separate picker while the current tab keeps running, no plugin configuration or `CLAUDE_CODE_PLUGIN_DIRS` is set and no file is written on A, the typed `claude` remains a valid way to start Claude Code, and B's choices omit Claude Code without affecting its terminal

#### Scenario: Remote status through the remote terminal

- Given a `Claude Code SSH` tab on a supported server running claude 2.1.295 or later outside tmux
- When remote Claude Code sends the probe, then `working`, `blocked` and `done` emissions, and sets its window title
- Then Made answers the probe through that remote terminal, the tab shows `Claude Code`, the Claude Code mark and one dot that is busy, attention and idle in turn, the tooltip starts with the sanitized title, the explorer stays on the remote root chosen by SSH-06 and SSH-10, and no other tab changes

#### Scenario: Remote presentation yields to OpenCode and Grok

- Given one remote tab with remote OpenCode connected and another remote tab with open Grok sessions under SSH-10, each also receiving `app=claude-code` emissions
- When the tabs are displayed
- Then the first keeps OpenCode's label, icon, dots and root, the second keeps SSH-10's `Grok Build` presentation, and neither shows Claude Code presentation

#### Scenario: Remote disconnect and exit

- Given a remote tab showing Claude Code presentation with a busy dot
- When the SSH transport drops and reconnects, and later remote Claude Code exits
- Then the disconnect clears the Claude Code status and title, the reconnected tab shows no stale dot until a fresh emission arrives, and after exit the tab returns to the server-name label and terminal glyph

#### Scenario: Remote Claude Code diagnostics

- Given remote workspaces where claude is undetected, not running, directly launched less than about 10 seconds ago with no session-level emission yet, running with session-level emissions, and directly launched with only the probe and no emission for longer than about 10 seconds
- When the user reveals About diagnostics and copies the report
- Then each row is driven by that tab's remote state with `Claude Code is not on PATH.`, `Claude Code is not running.`, `Claude Code is starting.`, `Claude Code session-status hooks are unavailable over SSH.` and `Claude Code status protocol is unavailable.` respectively, none claims ready, the rows stay out of routine chrome, and the copied report includes them

## MODIFIED Requirements

### Requirement: Catalog-driven new-tab menu

The `+` control MUST keep the tooltip and accessible name Open folder, and its behavior MUST be driven by the integrated-TUI catalog rather than by a special case for one program. Detecting a catalog entry MUST be a metadata-only PATH lookup that never spawns the program, never checks a version, and never installs a plugin, and that lookup MUST NOT delay shell or explorer startup. Empty and relative PATH segments MUST be ignored. When no integrated TUI is detected, `+` MUST open the folder picker and MUST show no menu. When any integrated TUI is detected, `+` MUST open a menu whose first row is Terminal, using the existing terminal glyph and the existing folder-picker flow, then one row for each detected integrated TUI in catalog order. Catalog order MUST be OpenCode, then Grok, then Claude Code, and a program that is not on PATH MUST be omitted. Choosing a TUI row MUST start that executable's absolute path directly as the child of a new home-directory tab, with no shell first and no folder picker, and the current tab MUST keep running. Made MUST NOT launch an integrated TUI unless the user chooses its row. The OpenCode row MUST keep the vendored 16px opencode.svg and MUST keep its existing launch, including its per-launch plugin configuration injection, follow, and dots. The Grok row label MUST be Grok Build, and its 16px leading slot MUST paint the official mark in `grok.svg` (the two paths from https://grok.com/images/favicon.svg, fill #FCFCFC, viewBox "56 56 400 400") rather than the terminal glyph. Choosing Grok MUST launch grok's absolute path with the single argument `dashboard`. That launch MUST NOT inject plugin configuration, including OpenCode's plugin environment and Made's Claude Code plugin entry, and MUST NOT write the user's Grok config. The Claude Code row's label, mark and launch are owned by `specs/claude-code-session-follow.md`. Codex and any other agent TUI MUST stay out of the catalog. Typing grok in a shell MUST remain valid, MUST NOT be replaced by the menu, and MUST NOT be given the `dashboard` argument.

#### Scenario: No detected TUI opens the folder picker

- Given no integrated TUI is on PATH
- When the user activates `+`
- Then the folder picker opens and no menu is shown

#### Scenario: Catalog menu lists detected TUIs in order

- Given OpenCode, Grok and Claude Code are all on PATH
- When the user activates `+`
- Then the menu is Terminal, then OpenCode, then Grok, then Claude Code, and the `+` tooltip and accessible name stay Open folder

#### Scenario: Only Grok is installed

- Given Grok is on PATH and neither OpenCode nor Claude Code is
- When the user activates `+`
- Then the menu is Terminal, then Grok, and OpenCode and Claude Code are omitted

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
- Then Made starts grok's absolute path plus the argument `dashboard` directly as the child of a new home-directory tab, with no shell first and no folder picker, the current tab keeps running, that launch does not inject plugin configuration, OpenCode's plugin environment or Made's Claude Code plugin entry, and does not write the user's Grok config, and typing grok in a shell is not given that argument

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

#### Scenario: Other agents stay out

- Given Codex or another agent TUI is on PATH
- When the user activates `+`
- Then no row is shown for it

### Requirement: Grok tab dots

When OpenCode is not connected in a workspace tab, Claude Code is not present there as `specs/claude-code-session-follow.md` defines, and that tab has open Grok sessions, the tab MUST paint one status dot per open Grok session using the same size, placement, cap of eight, pulse, colors, tooltip summary, and accessible-name summary as OpenCode session dots. Working is busy, attention and error are attention, and idle is idle. A session present only in the inventory, before any hook event, MUST show idle. While OpenCode is not active and Claude Code is not present, a tab whose target is a running Grok TUI, or a shell tab with open Grok sessions, MUST use the label `Grok Build` and the official 16px `grok.svg` mark in the leading icon slot, even before a session exists. Its tooltip MUST be the session summary when sessions exist and MUST omit the workspace path. When that Grok is no longer running and the tab has no open Grok sessions, the label and leading icon MUST return to the folder label and terminal glyph, unless Claude Code is present, in which case `specs/claude-code-session-follow.md` owns them. If OpenCode is active, the label, leading icon, and dots MUST stay OpenCode's even when Grok sessions exist. If OpenCode is not active and Claude Code is present, the label, leading icon, and dots MUST be Claude Code's even when Grok sessions exist.

#### Scenario: Dots match session state

- Given a Grok tab whose sessions are working, waiting on a question, and idle, with no Claude Code present
- When the workspace tab is displayed
- Then it shows a pulsing busy dot, a yellow attention dot, and a neutral idle dot, the label is `Grok Build`, the leading icon is the official `grok.svg` mark, and the tooltip counts those three categories without the workspace path

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

### Requirement: Shell Directory Follow

For supported launch-local shell integration, Made MUST follow real directory changes in bash, zsh, fish and PowerShell, including paths with spaces and non-ASCII characters, without requiring user configuration edits. Directory reports MUST be validated as local absolute paths, bounded, and treated as untrusted data, never commands. To discover cwd, Made MUST NOT scrape prompts or titles and MUST NOT inject commands into an already-running shell, and a window title MUST NOT be used as a cwd source. This clause is scoped to cwd discovery; the only window-title read that feeds Claude Code tab presentation or identity is the bounded Claude Code tooltip title allowed by `Claude Code privacy and observation boundary` in `specs/claude-code-session-follow.md`, which is never used for cwd. Hooks MUST be scoped to that shell launch, preserve existing startup/profile behavior and prompt hooks, and leave user shell files byte-for-byte unchanged. If a shell, launch mode or policy cannot safely support reporting, Made MUST keep the terminal usable at the latest known cwd or initial folder and explain the capability in Diagnostics without claiming live follow. cmd MUST have this explicit fallback unless a safe reporting mechanism is demonstrated; receiving OSC 7 alone MUST NOT be described as guaranteed cmd or arbitrary-shell integration.

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

### Requirement: V1 Scope Boundary

Made MUST support OpenCode v2, the Grok Build session follow defined by `specs/grok-session-follow.md`, and the Claude Code integration defined by `specs/claude-code-session-follow.md`. The `+` menu MUST be where OpenCode, Grok and Claude Code are offered, each only when it is detected on PATH. Made MUST NOT integrate with any other agent TUI. Made MUST NOT require remote, SSH, or WSL-hosted OpenCode, Grok or Claude Code. Made MUST NOT provide an AI or chat UI that replaces any of these TUIs. Made MUST NOT create, remove, merge, or orchestrate worktrees. Made MUST NOT require a separate OpenCode, Grok or Claude Code server process.

#### Scenario: Unsupported agent not offered

- Given the `+` menu
- When the user looks for agent integrations
- Then OpenCode, Grok and Claude Code are the only agent rows the menu can offer, and Codex, ACP, and any other agent integration are not presented

#### Scenario: No replacement chat UI

- Given the Made interface
- When the user looks for an AI or chat surface
- Then no AI or chat UI replaces the OpenCode, Grok or Claude Code TUI

#### Scenario: Worktree lifecycle remains outside Made

- Given a worktree
- When the user inspects Made's controls
- Then no create, remove, merge, or orchestration action is available, and worktree changes appear only after OpenCode reports them

#### Scenario: Direct launch

- Given a user wants to use OpenCode, Grok or Claude Code with Made
- When they run that command directly in the Made terminal
- Then follow works without Made starting a separate server or attach process, and the `+` menu does not replace that direct launch

### Requirement: Brand and Workspace Tabs

Made MUST present editor-like workspace tabs starting at the window gutter/card left edge, with folder labels, full-path tooltips, and a reachable close affordance; an OpenCode-active tab MUST instead use the `OpenCode` label and path-free tooltip defined below. An open Grok Build tab is not required to use that folder label or full-path tooltip; its label and tooltip are owned by `specs/grok-session-follow.md`. A tab where Claude Code is present is likewise not required to use them; its label and tooltip are owned by `specs/claude-code-session-follow.md`. Every workspace tab MUST begin with a 16px leading icon slot at the 10px left inset. An OpenCode-active tab MUST paint the vendored 16px `opencode.svg` in its shipped colors in that slot, while every other workspace tab—including a plain shell tab with no OpenCode integration—MUST paint the existing app-drawn terminal glyph centered in the same slot, colored like the tab label: `TEXT_PRIMARY` when the tab is active or hovered and the inactive tab text otherwise. A Grok Build tab is not required to paint that glyph; its leading icon is owned by `specs/grok-session-follow.md`. A Claude Code tab is likewise not required to paint it; its leading icon is owned by `specs/claude-code-session-follow.md`. The label MUST start after the icon slot, and that slot MUST always be reserved in the label-width budget so the icon alone never changes tab width. The header row MUST span the card left/right edges with no extra horizontal padding. The last right-side icon’s 26/24 target, selected/hover fill, and focus ring MUST all end at the card right edge with no gutter overhang; the 14px glyph MUST remain centered inside its target with normal optical padding on both sides. The two-tone monospace `MADE` wordmark with its accent `M` and its `About` trigger identity MUST sit to the right of the tab strip, immediately before the right icon group, with a reserved 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark (the visible flexible space after the last visible control is larger when few tabs are shown), the same exact 24px wide / 18px compact gap from the wordmark to the first right icon, and no divider. The wordmark MUST NOT look like a tab and MUST NOT paint a fill, hover, or selection treatment. It MUST be the persistent About trigger defined by the Minimal Workspace Layout requirement, which opens the About card and its on-demand diagnostics detail, with the `About` tooltip and accessibility name, a pointer cursor, a target of at least 24×24, a visible keyboard focus ring, and Enter/Space activation. The top workspace strip and workspace tabs MUST both be 26/24 high; the session-map strip/tabs keep their previous 36/34 and 28/26 metrics unless they directly depend on the reduced top height. Workspace tabs MUST be modestly wider than the previous layout so longer labels stay usable: the base tab width MUST clamp to 84–198px in compact layout and 96–238px in normal layout. A fixed 12px text-growth allowance MUST be included beyond the reserved slots, keeping an OpenCode tab's usable label area about 12px wider than the previous layout at the same label length. The 16px leading icon slot with its 6px gap, a fixed 14px right-edge allowance, and the close slot MUST be reserved in the label layout; that allowance keeps the label clear of the corner session-dot/close treatment. The session-dot row MAY add up to 31px beyond the base width when needed, giving final maxima of 229px compact and 269px normal with 8 dots; that added width MUST keep the label area unchanged, MUST keep the label from overlapping the dots, and MUST NOT animate. The OpenCode icon MUST NOT add width when it appears or disappears; any width growth comes only from the session-dot row. The selected workspace tab MUST use the opaque `BG_TAB_SELECTED #1B231F` dark green-gray fill with a `TEXT_PRIMARY` 13.0px label, the faint 1px inside stroke, and no underline, dot/hollow marker, accent bar, or width animation. Selection MUST also carry a subtle non-color cue: the selected label is set at 13.0px versus 12.5px for inactive tabs, vertically centered, with tab width and layout unchanged and no animation; the selected fill is also a luminance step above the idle/hover fills. Inactive workspace tabs MUST use the translucent black `BG_TAB_IDLE` fill, no stroke, and secondary text at reduced opacity. Hovering an inactive workspace tab MUST replace the idle fill with the slightly darker opaque `BG_TAB_HOVER #151C18` fill, paint no stroke, and restore the label to full-opacity `TEXT_PRIMARY`; a selected tab keeps `BG_TAB_SELECTED` and the faint inside stroke on hover. The tab close affordance MUST keep its 24×24 hit area and keyboard focusability reserved at all times. That target MUST remain centered at the tab's right edge, and its `×` glyph MUST be hidden at rest—including on the selected tab—and MUST appear only while the tab is pointer-hovered, the close affordance is pointer-hovered, or the tab or close action has keyboard focus. The handoff between the session dots and the `×` MUST be a smooth crossfade of about 0.15s with quadratic ease-out, not an instant swap: while the close glyph is becoming visible, all session status dots MUST fade out with `(1 - reveal)` opacity and the `×` MUST fade in with `reveal` opacity, so mid-transition both may be partially visible; when fully hovered or focused only the `×` remains at the right corner, and leaving MUST reverse the crossfade smoothly. At rest, the session dots MUST be fully visible when the workspace has open sessions and the `×` MUST be hidden. The close affordance and the crossfade MUST paint no background fill. Hovering only the close icon MUST turn the visible glyph `ERROR` and MUST also apply the whole tab's hover fill and label treatment without changing tab width, label truncation, or layout. Direct close hover MUST still show the close tooltip `Close workspace` instead of the tab's tooltip. The glyph MUST use the fixed documented size with no scale or width animation. When OpenCode is active in that workspace, the tab MUST paint the vendored `opencode.svg` icon in its leading icon slot before its label at 16px in its shipped colors with no tint and a 6px gap to the label. OpenCode is active when its integration is connected in that workspace—either a directly launched OpenCode TUI or a shell tab where the user ran `opencode` manually—or when the tab was launched directly as the OpenCode TUI and its PTY child is still running. The icon MUST be absent only when the integration is disconnected and no directly launched OpenCode TUI's PTY child remains running; it MUST be absent on shell tabs with neither a connected OpenCode integration nor a running directly launched TUI child, while a shell tab where the user ran `opencode` manually remains OpenCode-active while that integration is connected; a tab without the OpenCode icon MUST paint the app-drawn terminal glyph in its leading icon slot, except a Grok Build or Claude Code tab, whose leading icon is owned by `specs/grok-session-follow.md` or `specs/claude-code-session-follow.md` respectively. When OpenCode is active in that workspace, the tab label MUST be exactly `OpenCode` instead of the workspace's folder name, and it MUST revert to the folder label when the workspace stops being OpenCode-active, except an open Grok Build or Claude Code tab, whose label is owned by `specs/grok-session-follow.md` or `specs/claude-code-session-follow.md` respectively; non-OpenCode tabs MUST keep folder labels, except an open Grok Build or Claude Code tab, whose label is owned by `specs/grok-session-follow.md` or `specs/claude-code-session-follow.md` respectively. An OpenCode-active tab's tooltip MUST omit the workspace path: when the workspace has open sessions it MUST show only the session summary, and with no sessions it MUST show no parent tooltip. Hovering its close affordance MUST still show `Close workspace`, and non-OpenCode tabs MUST keep the full-path tooltip, except an open Grok Build or Claude Code tab, whose tooltip is owned by `specs/grok-session-follow.md` or `specs/claude-code-session-follow.md` respectively. The OpenCode tab's accessible name MUST be `OpenCode` plus the summary when present, for example `OpenCode — Sessions: 2 running, 1 needs attention, 1 idle`. When OpenCode is active in that workspace and the workspace has open sessions, the tab MUST paint one 4px diameter (2px radius) status dot per open session in `SessionMap` row order, with 3px gaps, right-aligned so the last dot is centered at `tab.right() - 10`, and vertically centered. At most 8 dots MAY be shown; sessions beyond 8 MUST remain in the tooltip and accessibility count summary but MUST NOT get a visual marker. Each dot MUST reflect its own session state: busy uses `ACCENT` and MUST gently pulse via opacity only, about a 1.8s period with opacity between roughly 0.35 and 1.0, without movement or size change; attention uses static `WARNING`; idle or stopped uses static neutral `TEXT_TERTIARY`. Within one session, attention takes precedence over busy; attention is per session and MUST NOT suppress another session's busy dot. The dots MUST NOT convey status by color alone: the tab tooltip and accessible name MUST summarize all sessions, including sessions beyond the 8 visible dots, for example `Sessions: 2 running, 1 needs attention, 1 idle`, omitting zero categories. The session-dot row is a state-driven operational indicator separate from selection: it MUST NOT act as, or add, a selection underline, selection dot or hollow marker, accent bar, or width animation, and MUST NOT alter the selected tab's documented fill, stroke, or 13.0px label. Workspace tabs MUST remain visually distinct from optional OpenCode session tabs and MUST NOT animate their width. When tabs do not fit, a labeled, keyboard-reachable overflow list MUST expose every workspace tab; labels MUST truncate with an ellipsis and retain full-value tooltips.

#### Scenario: MADE wordmark identity

- Given the running UI
- When the header brand is inspected or activated by pointer or keyboard
- Then workspace tabs start at the window gutter/card left edge, the last right-side icon’s 26/24 target, selected/hover fill, and focus ring end at the card right edge with the 14px glyph centered inside the target with normal optical padding, the top strip and tabs are 26/24 high with one 8/6 pane gap above and below, and `MADE` is a cached two-tone monospace wordmark with accent `M`, primary `ADE`, the documented tracking and mint accent, the `About` tooltip and accessibility name, a pointer cursor, a target of at least 24×24, a visible keyboard focus ring, Enter/Space activation that opens the About card, a reserved 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark (visible space after the last visible control may be larger), an exact 24px wide / 18px compact wordmark-to-first-icon gap, no divider, no tab fill/hover/selection, and no `BG_HOVER`, `BG_SELECTED`, `BG_TAB_HOVER`, or `BG_TAB_SELECTED` fill

#### Scenario: Workspace tab presentation

- Given two or more open workspaces, including an OpenCode-active workspace and a shell workspace with no OpenCode integration
- When the tab strip is displayed
- Then the shell tab shows the app-drawn terminal glyph centered in its 16px leading icon slot at the 10px left inset, colored like its label, except a Grok Build or Claude Code tab, whose leading icon is owned by `specs/grok-session-follow.md` or `specs/claude-code-session-follow.md` respectively, plus its folder label starting after that slot and full-path tooltip, except an open Grok Build or Claude Code tab, whose label and tooltip are owned by `specs/grok-session-follow.md` or `specs/claude-code-session-follow.md` respectively, plus its close affordance, while the OpenCode-active tab shows the vendored 16px OpenCode icon in its shipped colors in the same leading icon slot with a 6px gap before its label, and the exact `OpenCode` label starting after the icon slot, a path-free tooltip (only the session summary, or no parent tooltip when it has no sessions), and its close affordance, while that shell tab shows no OpenCode icon; both tabs always reserve the icon slot in the label-width budget; tabs keep base widths of 84–198px compact and 96–238px normal with the fixed 12px text-growth allowance plus the reserved icon, fixed 14px right-edge allowance, and close slots accounted for, so an OpenCode tab's usable label area is about 12px wider at the same label length; a session-dot row MAY add up to 31px beyond the base to final maxima of 229px compact and 269px normal for 8 dots, keeping the label area unchanged and clear of the dots without animation, and the icon alone does not change tab width; the selected tab uses the opaque dark green-gray `BG_TAB_SELECTED` fill, a `TEXT_PRIMARY` 13.0px label, the faint 1px inside stroke, and no underline, dot/hollow marker, accent bar, or width animation; inactive tabs use the translucent black `BG_TAB_IDLE` fill, no stroke, secondary text at 12.5px and reduced opacity; and session tabs are not presented as workspace tabs

#### Scenario: Workspace tab OpenCode icon presence

- Given a workspace launched directly as the OpenCode TUI whose integration disconnects while its PTY child still runs, then that child exiting; and a shell workspace where the user runs `opencode` manually, then that integration disconnecting
- When the workspace tab strip is displayed after each change
- Then the icon remains while the directly launched TUI's PTY child still runs even if its integration is disconnected, and appears when the shell's OpenCode integration is connected; it disappears only when the integration is disconnected and no directly launched TUI's PTY child remains running, including on shell tabs with no OpenCode integration; it is the vendored 16px OpenCode icon in its shipped colors with a 6px gap to the label; while the icon is present the tab label is exactly `OpenCode`, starts after the leading icon slot, and its tooltip omits the workspace path, and when the icon disappears the tab paints the app-drawn terminal glyph in the same leading slot, except a Grok Build or Claude Code tab, whose leading icon is owned by `specs/grok-session-follow.md` or `specs/claude-code-session-follow.md` respectively, and the label reverts to the folder name with the full-path tooltip, except an open Grok Build or Claude Code tab, whose label and tooltip are owned by `specs/grok-session-follow.md` or `specs/claude-code-session-follow.md` respectively; and the icon's appearance adds no width beyond its reserved slot

#### Scenario: Workspace tab OpenCode activity

- Given a workspace whose OpenCode sessions are, in row order, busy, attention, idle, and busy, then a workspace with nine busy sessions, then a workspace with no sessions
- When the workspace tab is displayed
- Then with sessions present the tab paints one 4px diameter (2px radius) dot per session in that row order with 3px gaps, right-aligned so the last dot is centered at `tab.right() - 10` and vertically centered: the busy dots use `ACCENT` and gently pulse via opacity only over about a 1.8s period between roughly 0.35 and 1.0 with no movement or size change, the attention dot uses static `WARNING`, and the idle dot uses static `TEXT_TERTIARY`; attention takes precedence within its own session but does not suppress the other busy dots; the tooltip is `Sessions: 2 running, 1 needs attention, 1 idle` and the accessible name is `OpenCode — Sessions: 2 running, 1 needs attention, 1 idle`; with nine sessions only 8 dots are painted, the ninth session gets no visual marker, the tooltip is `Sessions: 9 running`, the accessible name is `OpenCode — Sessions: 9 running`, and at most 31px of dot-row width is added; with no sessions no dots are painted, no summary is added, no parent tooltip is shown, and the accessible name is `OpenCode`; status is not conveyed by color alone, the tab width does not animate, and the session-dot row neither replaces nor adds a selection underline, selection dot or hollow marker, accent bar, or width animation

#### Scenario: Workspace tab hover and close

- Given selected and inactive workspace tabs with their close affordances
- When an inactive tab or only its close icon is hovered
- Then an inactive tab switches to the opaque `BG_TAB_HOVER` fill, paints no stroke, and shows a full-opacity primary label; a selected tab keeps `BG_TAB_SELECTED` and the faint inside stroke; the hidden-at-rest close glyph appears while the tab is hovered or the close affordance is hovered through a smooth crossfade of about 0.15s with quadratic ease-out in which all session status dots at the right corner fade out with `(1 - reveal)` opacity while the `×` fades in with `reveal` opacity, so both may be partially visible mid-transition before only the `×` remains, paints no background, and, when hovered directly, turns only the glyph `ERROR` while the parent tab also shows its hover fill and primary label, the close tooltip replaces the tab tooltip, including the OpenCode session summary, the tab width/label layout stays unchanged, and leaving reverses the crossfade smoothly

#### Scenario: Workspace tab close visibility

- Given selected and inactive workspace tabs with their 24×24 close affordances, first with OpenCode session dots and then without any
- When the tabs are at rest, then pointer-hovered, then keyboard-focused on the tab or close
- Then the `×` glyph is absent at rest on both inactive and selected tabs, and, when the workspace has open sessions, all session dots are painted right-aligned at the right corner; on tab hover, close hover, or keyboard focus the `×` fades in over about 0.15s with quadratic ease-out while all session status dots fade out with `(1 - reveal)` opacity over the same transition, so both may be partially visible mid-transition and only the `×` is painted once fully hovered or focused; leaving the tab or close reverses the crossfade smoothly; the `×` keeps its 24×24 keyboard-reachable target and focus ring centered at the right edge, and no background is painted; the glyph uses the `ERROR` tone only on direct close hover, never scales, and the crossfade changes neither tab width nor label layout

#### Scenario: Workspace tab overflow and close

- Given enough workspace tabs to exceed the available width at 1280×800 or 720×480
- When the header lays out
- Then a labeled keyboard-reachable overflow list exposes every tab, the active tab remains visible, no tab or close affordance is clipped, and closing still targets only its workspace

### Requirement: SSH server entry and launch choices (SSH-01)

Made MUST expose a `Servidores SSH` icon-only header action immediately beside the Explorer hide/show action, with matching tooltip/accessibility name, visible keyboard focus and at least a 24×24 target. It MUST offer server registration, an explicit connection test and remote OpenCode, grok and claude detection with success/failure feedback. The `+` flow MUST retain existing local choices and offer `Terminal SSH`, and, for a selected registered server, `OpenCode SSH`, `Grok SSH` and `Claude Code SSH` when that program has been detected on the server; `OpenCode SSH` MUST be offered only when OpenCode has been detected on that server, `Grok SSH` only when grok has been detected there, and `Claude Code SSH` only when claude has been detected there. Detection MUST NOT start the OpenCode TUI, start grok, start claude, or install any program's plugin, and it MUST use the same bounded metadata-only remote PATH rules as the existing probe, ignoring empty and relative PATH entries. Failure or absence MUST NOT prevent a supported remote terminal/explorer workspace or unrelated local workspaces from operating. A previously detected executable that disappears before launch MUST cause contextual failure, not local execution or substitution of a different server. Choosing `Grok SSH` MUST start the detected remote grok executable with the single argument `dashboard` directly as a new tab bound to that server and its selected remote directory, with no shell first and no separate folder picker, while the current tab keeps running and no plugin configuration is injected into that launch; typing `grok` manually in a supported remote shell MUST remain valid, MUST NOT be replaced by the menu, and MUST NOT be given the `dashboard` argument. Choosing `Claude Code SSH` MUST start the detected remote claude executable as SSH-11 defines.

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
