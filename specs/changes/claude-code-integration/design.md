# Design: claude-code-integration

Non-normative. Guarantees live in `spec.md`; this file holds the channel trade-off, the interacting flow, names and security reasoning. Spike facts were observed on claude 2.1.319 with the emissions replayed through the real `made_terminal::TerminalCore`. Items marked "phase 0" are facts the early verification tasks settle before later work depends on them.

## Approach

Made learns about Claude Code through two independent channels and joins them per tab:

| Channel | Carries | Source of truth for | Missing when |
|---|---|---|---|
| OSC 7501 Program Status Protocol, parsed by Made's terminal | session-level `state` for `app=claude-code` | dot status; gates reading the window title as tooltip title | Claude Code < 2.1.295, inside tmux, `--bg` sessions, `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1`, probe reply not delivered, probe reply late or misordered |
| Session-only plugin via `CLAUDE_CODE_PLUGIN_DIRS`, native async command hook, per-tab spool | session id, cwd, event, notification type, derived status, optional session title, process ancestry | session identity, liveness, explorer directory, fallback status, renamed title | `--bare`, `--safe-mode`, `disableAllHooks`, `allowManagedHooksOnly`, missing shipped plugin, non-executable hook, unwritable spool, Claude Code started outside a Made tab |

Neither channel alone is enough: OSC 7501 has no directory, no title and no reliable session identity, and hooks have no immediate "blocked" signal (the permission notification arrives about 6 s late; `PermissionRequest` helps but not for every blocked state). Each is fail-open, so the four combinations (both, protocol only, hooks only, neither) are the whole state space; `Claude Code channel fallback` in the spec enumerates them.

Alternatives considered and rejected:

- **HTTP hooks posting to the OpenCode bridge:** needs a listener reachable by Claude Code, per-request authentication and an `http` hook; adds a network surface for no information the command hook cannot already write locally.
- **Statusline command:** a single user-owned slot in Claude Code settings; using it means editing user config or displacing the user's statusline.
- **IDE lockfile / MCP IDE channel:** Made would impersonate an IDE, implement an MCP server and receive editor-level context.
- **Reading transcripts or session files:** exposes prompt and model content and couples to an undocumented on-disk format.
- **`claude plugin install` (persistent install):** persists into the user's configuration and affects Claude Code outside Made.
- **`--plugin-dir` argument on the direct launch:** covers only the menu launch, not a shell-typed `claude`, and the spec forbids adding arguments.
- **Title glyph parsing (`✳` idle, `◐`/`◑` working without a probe reply):** unstructured and version-fragile; not a channel.
- **`claude agents --json` inventory:** works (pid, session id, name, idle/busy/waiting in 0.16 to 0.4 s, 1.4 s outlier) but spawns `claude` on a poll, which the metadata-only detection rule avoids; kept out of the contract.

## Significant decisions

### OSC 7501 wire format

- Claude Code sends the probe `ESC ] 7501 ; ? BEL` once, 0.27 to 0.7 s after start, alongside `ESC [ > 0 q` (XTVERSION) and `ESC [ c` (DA1). Made replies `ESC ] 7501 ; ? BEL`; the first emission `state=idle` follows within 20 to 50 ms. `TerminalCore` currently answers DA1/DECRQM but not XTVERSION or 7501, and `OscScanner` (`crates/made-terminal/src/emulation.rs`) parses only OSC 7, so the reply must be queued by Made through the same path as the DA1 answer. XTVERSION stays unanswered (no change). The reply is byte-identical to the probe, so a tty with `ECHO` on and `-echoctl` echoes it back as output and would be answered forever (reproduced at about 16k replies/s). `TerminalCore` therefore answers at most one probe per terminal within a 1 s window (timestamp of the last reply, `feed_at(bytes, now)` injects the clock for tests); a probe inside the window produces no reply and no event, and a later probe (for example a Claude Code relaunch) is answered again. Inside an OSC, `ESC ]` starts a new OSC as in vte, so a probe directly after an unterminated OSC is still answered.
- Reply ordering (phase 0 finding; non-normative, the guarantee is the ordering sentence in spec.md `Program status protocol in the terminal`): Claude Code sends the probe together with XTVERSION and DA1 and appears to treat the DA1 answer as the end of the query batch. The design therefore queues the probe reply before any reply to a query that follows the probe in the output stream; DA1 is the observed case and DECRQM only an example of another answered query. `TerminalCore::feed` runs the alacritty parser over the whole chunk and then `OscScanner` over the same chunk, so a DA1 reply is queued during `parser.advance` before the scanner could queue the probe reply; a naive implementation therefore inverts the order whenever probe and DA1 share a chunk. The implementation must keep stream order, for example by splitting the chunk at the probe terminator and queueing the probe reply between the two parser advances. Measured: reply in stream order 100% enabled; DA1 first 9/13; DA1 answered earlier in a separate loop iteration (old responder) about 35%.
- Emission: `ESC ] 7501 ;` + `:`-joined `key=value` + BEL (ST also accepted). Fields seen: `state`, `app`, `id`, `kind` (`permission`, `auth`), `title` (base64, only on `id` rows), `msg` (base64 human text, may contain tool commands such as `approve Bash: touch ...`). States: `idle`, `working`, `done`, `blocked`, `clear`, `error` expected. `progress` not seen.
- Session-level entries are those without `id`. `id` rows are sub-task/subagent rows and `state=clear:id=X` removes a row; both are ignored. On `/exit` Claude sends `working`, three bare `state=clear` (no `app`), then an empty OSC 0 title; a bare clear is accepted only while a `claude-code` status is live in the tab.
- Bounds and handling: sequence ≤ 4096 bytes; `msg` is skipped during parsing without being base64-decoded into a retained buffer; `title` and `kind` skipped. The parser emits a small event (`Probe`, `Status{state}`, `Clear{bare}`) per terminal; the app maps it.

### Tooltip title

The window title (OSC 0/2) already tracked by `TerminalCore::title()` is read for the tooltip only while that tab's Claude Code protocol status is live; the structured status proves the program that owns the title. Sanitized with `crates/made-app/src/sanitize.rs` and capped at 120 characters; an empty title falls through to the hook `session_title` (sent by Claude only on the next `UserPromptSubmit` after `/rename`), then `Claude Code`. Titles never feed cwd, status or identity. The spec MODIFIES `Shell Directory Follow` only to scope its title clause explicitly to cwd discovery and point to this bounded allowance.

### Plugin loading

- Shipped package `plugins/claude-code-made/` beside `plugins/opencode-made` and `plugins/grok-made`, resolved from `<install_root>/plugins/` like the others. Manifest `.claude-plugin/plugin.json` with `"name": "made"` (validator rejects `claude-*` and warns on `claude`; phase 0 confirms the directory name is not validated), `hooks/hooks.json` registering `async: true` command hooks for `SessionStart`, `SessionEnd`, `UserPromptSubmit`, `Stop`, `StopFailure`, `PermissionRequest`, `Notification` and `CwdChanged`, and `bin/made-claude-status[.exe]`, which must be executable (verified requirement).
- Staging mirrors `.github/scripts/stage-grok-plugin.sh`: repository source holds manifest and `hooks.json`; `.github/scripts/stage-claude-plugin.sh` copies them plus the built hook (mode 755) into `<install_root>/plugins/claude-code-made` for development runs and desktop packages. Windows `hooks.json` generation follows the Grok precedent if `bin/<name>` does not resolve `<name>.exe`.
- Env composition: at PTY spawn for local shell tabs and direct Claude Code tabs only, `CLAUDE_CODE_PLUGIN_DIRS = <existing entries> + <shipped dir>`, skipping the append when an entry already equals the shipped dir. `:` verified on POSIX; Windows separator is a phase 0 task (candidate `;`). Empty existing value means no leading separator.
- Per-tab spool binding: Made also sets `MADE_CLAUDE_SPOOL=<absolute spool path>` in the same environments. The hook writes only when that path is absolute and its parent is a current-user 0700 directory.
- Historical spike risk (resolved, see Phase 0 results): reply ordering, not plugin count, decides whether Claude Code enables reporting. Entries are still never dropped, reordered or replaced.

### Hook spool

- Location: `<private app temp>/claude/<owner-pid>-<instance-id>/<workspace-instance-id>.jsonl` under the existing per-user app temp (`made-<uid>`, 0700); the per-instance directory names the owning Made process and a random instance id, so startup cleanup removes only directories whose owner pid is gone (or alive but with a different instance id after pid reuse) and leaves a concurrently running instance untouched; file 0600, created with `O_NOFOLLOW`/`create_new` semantics on Unix; Windows relies on the per-user temp location like the rest of Made.
- Record (one JSON line ≤ 4 KiB, single write): `at`, `sessionId`, `cwd`, `event`, `notification`, `status` (`idle`/`working`/`attention`/`error`/`ended`/empty for no change), `title` (sanitized `session_title` when present), `pids` (ancestor chain, nearest first, ≤ 32). The hook deserializes into a fixed struct holding only those inputs, so `prompt`, `tool_input`, `message`, `transcript_path` never reach a write call. `CLAUDE_CODE_SESSION_ID` in the hook environment equals the payload `session_id` and may be used as a fallback.
- Hook process: same structure as `made-grok-status` (bounded stdin read with a ~2 s deadline, first JSON value only, always exit 0, nothing on stdout).
- Status mapping: `SessionStart` idle; `UserPromptSubmit` working; `PermissionRequest` attention; `Notification` `permission_prompt` attention (about 6 s), `idle_prompt` idle (about 60 s); `Stop` idle; `StopFailure` error; `CwdChanged` no status change; `SessionEnd` ended (reason recorded only as part of `event`, not stored separately). `CwdChanged` and `StopFailure` did not fire in tests; registering them is harmless and the directory rule does not depend on them.
- Directory rule: session directory = `cwd` of the latest owned record of that session, any event. With `claude -w`, `SessionStart.cwd` is the original directory and later events carry the worktree.
- Bound: Made reads incrementally from its last offset; when the file exceeds 1 MiB and is fully consumed, Made truncates it under an advisory lock the hook also takes. If the hook finds the file at the bound and unconsumed, it drops the record.
- Ingestion: about every 250 ms off the UI thread, which bounds the spec's "within about 250 ms" directory follow; lines older than the tab start, malformed, or over 4 KiB are ignored. Cleanup: tab close deletes its spool; startup removes only spool directories whose owner instance is gone.
- Windows: the spec makes Windows hook support a SHOULD. If 0.4 shows Claude Code on Windows cannot run the hook, the plugin still ships and loads, no records arrive, and diagnostics reports hooks unavailable after the grace period; nothing else is needed for fail-open.

### Phase 0 results

- 0.1 (OSC 7501 isolation): a standalone Python PTY responder (same bytes as the planned Rust spike; no Rust example was created, so none exists to delete) ran claude 2.1.319 with the probe reply written after the DA1 reply (54 runs): no plugin 0/3; one entry async hooks 1/3, hookless 2/3, sync hooks 1/3, entry with trailing colon 3/3; two entries hooks+hooks 1/3, reversed order 1/3, hookless+hookless 1/3, hooks+hookless 1/3, hookless+hooks 1/3, sync+hookless 2/3, nonexistent second 1/3, same dir twice 0/3, reply latency 0 2/3; reply latency 200 ms, 1 s, 3 s with one entry 0/9 and 1 s with two entries 0/3. Then an in-order responder (probe reply before DA1 reply as they appear in the stream), 19 runs, all enabled: two entries 6/6, no plugin 4/4, one entry 3/3, two entries with DA1 unanswered 3/3, two entries with 1 s latency 3/3; DA1-first control (13 runs): two entries 5/6, none 2/4, one entry 2/3 (9/13). Totals: 86 runs (54 + 19 + 13); enabled 17/54 with the DA1-first spike responder, 19/19 in order, 9/13 in the DA1-first control. Classification: Made-side requirement (reply ordering); not a Claude Code plugin-count limitation.
- 0.4 (macOS, Windows): `UNVERIFIED`. This machine is WSL Linux only, and the shipped binary text gives no documentary evidence for the hook launcher or the Windows separator. Linux result stands: hook process, `/bin/sh -c`, claude (two hops). CI covers only what needs no live claude: the hook binary and packaged-layout tests on macOS and Windows, and the composition helper with the separator as a parameter (`:` on POSIX, `;` candidate on Windows); the live `MADE_CLAUDE_LIVE` suite stays `UNVERIFIED` there until a runner has claude. If Windows cannot run the hook, Windows ships the plugin and fails open with hooks unavailable.
- 0.5 (plugin directory name): a throwaway copy of the plugin in a directory named `claude-code-made` with manifest name `made` passes `claude plugin validate` (exit 0, "Validation passed"). The default profile's only warnings were a missing `author` (and missing `description` when the manifest had none); with `description`, `author` and `license` in plugin.json plus a README the default profile reports nothing. The directory name and manifest name were not flagged, so no replacement name is needed. `--profile directory` (marketplace submission) still asks for a 40-word README, an icon and a marketplace install line; not applicable because the plugin is session-only. Consequence for task 2.8: put `description`, `author` and `license` in plugin.json.
- Hooks sanity: `session_title` only on the UserPromptSubmit after an explicit `/rename`, `CLAUDE_CODE_PLUGIN_DIRS` hooks with `async: true` fire from SessionStart, `:` separates entries (earlier spike).

### Ownership and liveness

Verified on Linux: hook process → `/bin/sh -c` → claude, so Claude Code is two parent hops from the hook. The hook records its ancestor chain at write time. Made accepts a record for a tab when the tab's terminal process pid is in that chain within 32 hops and the chain is still consistent with live ancestry at ingestion. The liveness anchor is the Claude Code process: the hook's grandparent when the parent is the hook shell, else the parent (phase 0 confirms macOS and Windows). For a directly launched Claude Code tab the anchor is the terminal child. Liveness is checked about once a second with the process-ancestry helpers used by `crates/made-app/src/grok_follow.rs`. One current session per anchor pid; a new `sessionId` from the same anchor replaces the old one. Pid reuse inside the one-second window is an accepted residual, as for Grok.

### Precedence and presentation

Root precedence per tab: OpenCode session/view directory > most recent live owned Claude Code session directory > Grok > shell cwd. `RootSource` in `crates/made-app/src/session_follow.rs` (`Shell`, `OpenCode`, `Grok`) gains `ClaudeCode` between OpenCode and Grok. Presentation precedence: OpenCode active > Claude Code present > Grok open > folder. Dot status: session-level protocol status (applied to the tab's most recent live owned session, or one protocol-only entry) > hook-derived status > idle.

### Diagnostics

Grace period about 10 s from presence start. "Status protocol observed" means at least one session-level `app=claude-code` emission since that Claude Code started; the probe alone does not count, because a probe can be followed by no emissions if the reply was late or misordered. A probe without emissions surfaces as `Claude Code status protocol is unavailable.` with hooks working. Remote rows reuse the same state machine with hooks permanently unavailable, so the remote ready state does not exist in this change.

## Flow

```
Claude Code start (menu row or shell-typed; env has CLAUDE_CODE_PLUGIN_DIRS + MADE_CLAUDE_SPOOL)
 ├─ ESC]7501;?BEL ──▶ OscScanner ──▶ queue reply ESC]7501;?BEL to PTY input; tab.protocol_seen
 ├─ ESC]7501;state=…:app=claude-code BEL (no id) ──▶ tab.protocol_status
 ├─ ESC]7501;…:id=… BEL ──▶ ignored          ESC]7501;state=clear BEL ──▶ clear if live
 ├─ OSC 0/2 title ──▶ TerminalCore::title() ──▶ tooltip only while protocol_status live
 └─ hooks (async) ──▶ made-claude-status ──▶ metadata JSONL to tab spool
made-app per tab: poll spool ─▶ validate ─▶ ownership (pid chain ∋ tab root, anchor alive)
                 ─▶ ClaudeSessions{anchor → (sessionId, latest cwd, hook_status, title, last_activity)}
liveness tick (~1 s) ─▶ drop sessions whose anchor exited
derive: present? ─▶ label/mark/dots/tooltip (OpenCode > Claude > Grok > folder)
        live session cwd ─▶ SessionFollowCoordinator (OpenCode > Claude > Grok > shell)
        channels seen + grace ─▶ diagnostics row
```

## Architectural boundaries

- `made-terminal`: OSC 7501 scan, bounds, probe reply queued through the existing answer path; exposes protocol events per terminal; knows nothing of Claude Code beyond the `app` filter value.
- `made-platform`: shipped plugin path resolution and env composition helper (pure over the inherited value).
- New crate `crates/made-claude-hook` (binary `made-claude-status`): parse/map logic in `lib.rs`, process glue in `main.rs`, depending on `serde_json` and the pid-chain platform calls.
- `made-app`: new `claude_follow` module (spool reader, ownership, liveness, session table), wiring into `SessionFollowCoordinator`, `tui.rs` catalog/target, workspace-tab presentation in `ui.rs`, diagnostics row in `integration_status.rs`/`diagnostics.rs`.
- SSH: the client-sent `PROBE` in `made-platform/src/ssh.rs` adds one `command -v claude` line after grok, parsed in `made-app/src/ssh_servers.rs`; remote tab presentation and diagnostics in `made-app`. The companion `Capabilities` struct (`deny_unknown_fields`, `grok` used for companion-side Grok follow) is not changed: detection stays in the client `PROBE`. The only `made-remote` change is an additive `LaunchTarget::Claude` that resolves the absolute `claude` on the remote sanitized PATH and starts it with no arguments, no shell first, no plugin, environment, staging or remote write; the companion has no other Claude Code work.

## Security considerations

- **Env propagation:** both variables are inherited by everything started from a Made shell, including tmux servers. Those Claude Code instances load Made's plugin and may write metadata into the tab's spool; ownership by live ancestry rejects them, the hook writes metadata only and the spool is private. A stale spool path fails closed.
- **Spool spoofing:** a same-user process can forge records; inside the existing same-user exclusion, still subject to live ancestry and directory validation.
- **Window title:** untrusted; read only while a structured `claude-code` status is live, sanitized, capped, never used for cwd, status or identity. This is the policy change ADR 0008 and `wiki/security.md` record.
- **`msg`:** may contain tool commands; skipped without retention. Tested by marker search across spool, logs, tooltip and copied diagnostics.
- **Prompt and tool input:** structurally excluded by the hook's fixed input struct; tested by marker search.

## Migration and rollback

Nothing is persisted outside Made's private temp. Rolling back removes the env composition; a Claude Code still running keeps the plugin for its session only, and its hook fails closed once the spool directory is gone.
