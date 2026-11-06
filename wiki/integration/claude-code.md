# Claude Code

Claude Code is the third integrated TUI. Made learns about it through two independent, fail-open channels and joins them per tab: status from the OSC 7501 Program Status Protocol, and identity plus directory from a session-only plugin. Neither is required; with both missing the tab is a plain shell tab. The normative contract is [`specs/changes/claude-code-integration/spec.md`](../../specs/changes/claude-code-integration/spec.md) (it becomes `specs/claude-code-session-follow.md` at archive); rationale and alternatives are in [ADR 0008](../decisions/0008-claude-code-osc7501-session-plugin.md) and [`design.md`](../../specs/changes/claude-code-integration/design.md). Windows and macOS hook behavior is `UNVERIFIED`.

## Catalog and launch

- Catalog order is OpenCode, Grok, Claude Code; undetected programs are omitted. The row is labeled Claude Code and paints the official mark from [`claude-code.svg`](../../crates/made-app/assets/claude-code.svg) (source URL recorded in the file).
- Choosing it starts the absolute `claude` path with no arguments as the child of a new home-directory tab, with no shell first and no picker. Made passes no Claude Code arguments, edits no Claude Code file and runs no other `claude` subcommand.
- Detection is the shared metadata-only `PATH` lookup. Plugin loading does not depend on it, so a shell-typed `claude` is covered too.

## Channel 1: OSC 7501 status

`made-terminal` parses OSC 7501 and queues the reply through the same path as the DA1 answer; the app maps the events.

- **Probe reply.** Made answers the capability probe `ESC ] 7501 ; ? BEL` for any program with the byte-identical standard reply. Other programs only ever see that reply.
- **Reply ordering.** The probe reply is queued before the reply to any query that follows the probe in the output stream. DA1 is the observed case: Claude Code sends the probe with XTVERSION and DA1 and appears to treat the DA1 answer as the end of the batch. `TerminalCore::feed` runs the parser over the whole chunk before the scanner, so a naive implementation inverts the order when probe and DA1 share a chunk; the implementation splits the chunk at the probe terminator.
- **Echo guard.** The reply equals the probe, so a tty with `ECHO` on and `-echoctl` echoes it back and would be answered forever (reproduced at about 16k replies per second). At most one probe per terminal is answered per second; a probe inside the window yields no reply and no event, and a later probe, such as a Claude Code relaunch, is answered again.
- **Session-level emissions only.** Only emissions without `id` and with `app=claude-code` count. Emissions with `id` are sub-task rows and are ignored, including their clears. `kind`, `title` and `progress` are not used.
- **Mapping.** `working` is busy; `blocked` and `error` are attention; `done` and `idle` are idle. A bare `state=clear` ends a live Claude Code status and is ignored otherwise.
- **`msg` is never retained.** It can carry tool commands and is skipped during parsing: not decoded into retained state, stored, logged, displayed or copied.
- **Bounds.** Sequences over 4096 bytes are ignored.

## Channel 2: session-only plugin and hook

- **Plugin.** [`plugins/claude-code-made`](../../plugins/claude-code-made/README.md) is source only; `.github/scripts/stage-claude-plugin.sh` stages the shipped package, with the executable hook, into `<install_root>/plugins/claude-code-made`. The manifest name is `made`, because Claude Code's validator rejects `claude-*` names; the directory name is not validated.
- **Loading.** Made composes the shipped directory into `CLAUDE_CODE_PLUGIN_DIRS` at PTY spawn: existing entries are kept in order, the shipped one is appended, and an entry already equal to it is not duplicated. The value is never overwritten. The separator is `:` on POSIX and `;` on Windows (Windows unverified). A shipped path that is unreadable or contains the separator, quotes, backtick, `$` or a newline (or a backslash with `:`) is skipped, leaving the user's value untouched.
- **Where.** Local shell tabs and directly launched Claude Code tabs only; never OpenCode, Grok or SSH launches. Nothing is persisted in Claude Code configuration, and there is no `claude plugin install` and nothing to uninstall.
- **Spool binding.** Made also sets `MADE_CLAUDE_SPOOL=<absolute path>` in the same environments. The hook writes only when that path is absolute and its parent is a current-user `0700` directory.
- **Hook.** Native `made-claude-status` (`.exe` on Windows) from [`crates/made-claude-hook`](../../crates/made-claude-hook/src/lib.rs); no interpreter. It registers async command hooks for `SessionStart`, `SessionEnd`, `UserPromptSubmit`, `Stop`, `StopFailure`, `PermissionRequest`, `Notification` and `CwdChanged`, reads one event within a ~2 s deadline, always exits 0 and writes nothing to stdout.
- **Metadata only.** A record carries time, session id, cwd, event, notification type, derived status, an optional sanitized session title and the process ancestry. The hook deserializes into a fixed struct, so `prompt`, `tool_input`, `message` and `transcript_path` never reach a write call.
- **Status fallback.** `SessionStart` and `Stop` are idle, `UserPromptSubmit` working, `PermissionRequest` and a permission notification attention, `StopFailure` error, `SessionEnd` ended. The permission notification arrives about 6 s late, which is why hooks are only the fallback behind the protocol.
- **Session facts.** The session id comes from the payload and changes on clear, resume, fork and compact, replacing the previous session of the same process. The session directory is the cwd of the latest owned record of that session, whatever its event, because with `claude -w` only later events carry the worktree. The title appears only on the prompt after an explicit `/rename`.

## Spool, ownership and cleanup

- **Location.** `<tmp>/claude/<pid>-<id>/<pane>.jsonl` under the per-user app temp, directory `0700`, file `0600`, created without following symlinks. The `<pid>-<id>` directory names the owning Made instance and holds `owner.lock`.
- **Bound.** One line is at most 4 KiB. At 1 MiB the hook drops a record while the file is unconsumed; Made truncates the file after it has consumed it fully, under the advisory lock the hook also takes.
- **Ingestion.** Made reads incrementally off the UI thread about every 250 ms and ignores malformed, oversized or pre-tab-start lines.
- **Ownership.** A record is claimed only when live process ancestry ties it to the tab: the nearest live pid in the hook's recorded chain that descends from the tab's root pid within 32 hops. The Claude Code process, not the hook's `/bin/sh -c` parent, is the liveness anchor (hook, shell, then `claude`: two hops). One current session per anchor pid. A Claude Code started elsewhere that inherited the variables, such as under tmux, is not claimed.
- **Cleanup.** Tab close deletes its spool. Startup removes only instance directories whose owner is gone (dead pid, or a live pid with a different instance id), so concurrent Made instances never touch each other's spools. Liveness is checked about once a second; pid reuse inside that window is an accepted residual.

## Presence, precedence and presentation

- **Precedence.** Explorer root: OpenCode > Claude Code > Grok > shell. Tab presentation: OpenCode active > Claude Code present > Grok open > folder. Locally Claude Code outranks Grok because a running Claude Code is in the foreground of its tab. Remotely the SSH-10 order is kept, so remote Claude Code yields to Grok; this is a recorded follow-up to harmonize once `grok-native-hook` has archived.
- **Dots.** Protocol status wins over hook-derived status, which wins over idle. A finished turn shows idle; there is no new unread treatment.
- **Tooltip title.** The terminal's own window title, sanitized and capped at 120 characters, read only while a `claude-code` protocol status is live in that tab because the structured status proves which program owns the title. Otherwise the hook session title, otherwise `Claude Code`. A window title never feeds cwd, status or identity; see [security](../security.md) and [Layout and controls](../ui/layout-and-controls.md).

## Diagnostics

The revealed About diagnostics show one Claude Code row beside the Grok row. The first applicable state wins:

- Checking for Claude Code; not on PATH; not running.
- Starting, for about 10 s after Claude Code becomes present while a channel has not yet appeared.
- Hooks unavailable: the plugin is missing or no live owned session arrived after the grace period.
- Status protocol unavailable: hooks work but no session-level `app=claude-code` emission arrived. A probe alone does not count as observed, because a probe can be followed by no emissions when the reply is late or misordered or Claude Code does not enable reporting.
- Ready, with both channels working.

## Fail-open matrix

| Protocol | Hooks | Behavior |
|---|---|---|
| yes | yes | Protocol status, hook identity, directory and title |
| yes | no | Dots only; explorer and identity stay on shell cwd; hooks reported unavailable |
| no | yes | Hook-derived status; directory follows; protocol reported unavailable |
| no | no | Plain shell tab |

Reporting is off for Claude Code older than 2.1.295, inside tmux, in `--bg` sessions and with `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1`; hooks are off with `--bare`, `--safe-mode`, `disableAllHooks` and `allowManagedHooksOnly`. Made never drops or reorders the user's `CLAUDE_CODE_PLUGIN_DIRS` entries.

## SSH

For a registered SSH server the `+` flow offers `Claude Code SSH` when that server's own bounded remote `PATH` lookup finds `claude` (one more line in the client probe; the companion `Capabilities` struct is unchanged). The companion gains only an additive `LaunchTarget::Claude` that starts the absolute remote `claude` with no arguments and no shell first. Status arrives as OSC 7501 through the remote PTY; there are no remote hooks, plugin, staging or remote write, so the remote diagnostics row never reports ready and states that hooks are unavailable over SSH. Remote session follow and remote explorer follow are deferred. See [Grok SSH workspaces](grok.md#ssh-workspaces) for the shared launch flow.

## Evidence

Phase 0 on claude 2.1.319 (86 runs) traced the intermittent failure to enable reporting to reply ordering, not plugin count; see [ADR 0008](../decisions/0008-claude-code-osc7501-session-plugin.md) for the numbers. Not covered by tests that need no live `claude`: macOS and Windows hook launcher behavior and the Windows separator, left `UNVERIFIED`.

Related: [Grok](grok.md), [OpenCode bridge](opencode.md), [Terminal-first runtime](../runtime/terminal-first.md).
