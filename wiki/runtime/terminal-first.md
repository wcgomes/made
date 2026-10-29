# Terminal-First Runtime

A Made workspace starts as a real shell and explorer in a folder. OpenCode is optional and starts only when the user runs `opencode` in that terminal or explicitly chooses it from the `+` menu. This page records the runtime behavior implemented in [`app.rs`](../../crates/made-app/src/app.rs), [`workbench.rs`](../../crates/made-app/src/workbench.rs), [`session_follow.rs`](../../crates/made-app/src/session_follow.rs), and [`made-platform`](../../crates/made-platform/src/lib.rs). Normative behavior is owned by [`spec.md`](../../specs/changes/opencode-session-follow-v1/spec.md) and [`design.md`](../../specs/changes/opencode-session-follow-v1/design.md).

## Launch and default workspace

| Launch input | Result |
|---|---|
| `--workspace <path>` or positional folder | Explicit folder wins over `--picker`; opens a normal workspace. |
| `--picker` | Picker surface only; no shell or explorer until a folder is confirmed. |
| No workspace argument | Platform-resolved home opens as a normal workspace. |
| Missing/unreadable/unresolvable home | Visible non-blocking error, terminal-only fallback in a usable directory, no blocking picker, no crash. |
| `--smoke-test` without a folder | Test-only picker surface; never starts a real home shell. |

An explicit workspace does not require OpenCode installation, detection, a version wait, plugin readiness, a probe, or automatic launch. The folder picker is also the add-workspace surface and the state after the final tab closes.

When `opencode` is on `PATH`, the `+` new-tab control opens a menu with `Terminal` first and `OpenCode` second. `Terminal` runs the existing folder-picker flow. `OpenCode` starts `opencode` directly as the PTY child of a new home-directory tab: no shell first, no picker, and the current tab keeps running. The launched process is the absolute path that the same lookup finds, never the bare name. Detection is a metadata-only `PATH` lookup on a background thread started at launch; it never spawns the program, never verifies its version, and never delays shell or explorer startup. With no detected TUI, `+` keeps opening the folder picker directly.

The home/`--picker`/explicit-path policy is an accepted product decision; see [ADR 0005](../decisions/0005-home-default-startup.md). Implementation lives in [`workspace.rs`](../../crates/made-app/src/workspace.rs) and [`cli.rs`](../../crates/made-app/src/cli.rs).

## Shell cwd reporting

`made-platform::shell_integration` prepares a launch-local plan for supported shells. It sources the user's real startup files and appends an app-owned OSC 7 emitter without writing the user's files. The launch plan can carry environment and arguments for paths; it does not interpolate commands into the running shell.

| Shell | Current capability |
|---|---|
| bash | Verified with a real installed shell, including spaces/Unicode, profile preservation, and unchanged rc hash. |
| zsh | Source/unit coverage; real installed-shell run pending. |
| fish | Source/unit coverage; real installed-shell run pending. |
| PowerShell/pwsh | Source/unit coverage; real installed-shell run pending. |
| `cmd` / unknown / restricted launch | Explicit unsupported capability and reason; shell remains usable at the latest known cwd or launch folder. |

`made-terminal` validates OSC 7 as a bounded local absolute path, decodes URI escapes/Unicode, and rejects remote, relative, malformed, or oversized data. Cwd is never derived from terminal titles, prompts, or injected discovery commands. The latest validated shell cwd is retained independently of OpenCode.

## Root precedence

1. A connected OpenCode active session directory wins while an active session exists; an unavailable active directory is shown as unavailable, not replaced by the view directory or shell cwd.
2. Otherwise, a connected OpenCode view directory selected in the Home/new-session view wins while tabs are enabled, no session is active, and the directory is usable.
3. Otherwise, the latest validated shell cwd wins.
4. Otherwise, the launch/picker folder or platform-resolved home.
5. A stale OpenCode root is never retained ahead of a newer shell cwd.

While OpenCode is authoritative, shell cwd reports continue to be recorded. A connected view directory applies only while tabs are enabled and no session is active; an absent view directory leaves the latest shell cwd or launch folder in effect, while an invalid, missing, or unreadable one falls back to the latest shell cwd, then the launch folder, and is reported in Diagnostics. The view directory is cleared when OpenCode disconnects or exits, when integration is unsupported, when session tabs become unavailable, or on a new connection epoch. On OpenCode disconnect or normal exit, the root returns to the latest shell cwd; if no shell cwd exists, it returns to the launch folder. Shell-owned roots that are deleted or become unreadable are revalidated by a low-frequency background worker, marked unavailable once, and recover when the path returns. Agent-owned roots are unaffected by bad shell reports.

The root model distinguishes effective path, display name, source, git/worktree metadata, and availability. Root changes increment a generation, clear open preview, and invalidate stale tree/preview results. See [`session_follow.rs`](../../crates/made-app/src/session_follow.rs) and [`root_switch.rs`](../../crates/made-explorer/src/root_switch.rs).

## Workspace tabs and process lifecycle

`AppModel` holds an ordered list of live workspaces and one active id. Each `Workbench` owns its own:

- PTY/process session and shell bootstrap;
- terminal state and bounded output/scrollback;
- explorer root, generation, tree, watcher, and preview;
- bridge listener, token, pane id, and optional OpenCode connection;
- session map, focus state, explorer width/visibility, and left-surface mode.

Switching tabs marks the previous workbench invisible and the target visible, resets terminal focus/composition state, and never stops either process. Hidden workspaces continue bounded polling/output consumption and bridge liveness, but they do not paint grids. Preview input is isolated from its hidden terminal.

Closing a tab terminates/reaps only that workbench's process group, stops its listener/watchers, cancels pending loads, and releases its token/bootstrap state. Late callbacks are rejected by workspace id and generation. Closing the last tab returns to the picker and starts no shell or OpenCode. Window close shuts down all workbenches. There is no tab persistence across restart.

Process cleanup lives in `made-platform`:

- Unix captures the shell's process group, re-queries `tcgetpgrp` at cleanup to catch a foreground job after shell exit, skips PID 1/Made's group, and sends SIGHUP then SIGKILL.
- Windows workspace-scoped Job Object cleanup is implemented and cross-compiled; native runtime evidence is still pending. Immediate-child-only behavior is not claimed as full descendant cleanup.

## Bridge isolation

Each tab owns a separate loopback bridge and credential set. Events carry the owning endpoint and connection epoch. A snapshot, focus result, disconnect, or goodbye from another workspace is ignored; a cross-workspace hello is rejected before replacement or state disclosure. A stale epoch cannot overwrite a healthy workspace. See [Security](../security.md) and [OpenCode bridge](../integration/opencode.md).

## Optional OpenCode behavior

- Normal shell use is `inactive`, not degraded; detection is a background metadata lookup only, with no version wait, probe, or automatic launch.
- A valid plugin hello and snapshot move the tab to `connected`.
- A lost connection, heartbeat timeout, or plugin error moves it to `degraded` and resumes shell cwd.
- A protocol or OpenCode major mismatch is `unsupported`; the terminal remains usable.
- Sessions are observed only. Made does not create/remove/merge/prune worktrees or replace the OpenCode TUI.
- The optional session map is hidden by default and scoped to the selected workspace.

## Practical implications

- Do not make shell startup depend on OpenCode readiness or detection.
- Keep shell cwd valid while OpenCode is authoritative.
- Do not retain an agent root after disconnect.
- Do not share bridge state, tokens, preview state, or focus across tabs.
- When adding platform behavior, keep OS code in `made-platform` and preserve the per-tab shutdown boundary.

Related: [Architecture](../architecture.md), [Testing strategy](../testing/strategy.md), [ADR 0002](../decisions/0002-terminal-first-workspace-tabs.md).
