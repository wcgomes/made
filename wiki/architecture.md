# Architecture

Made is a native Rust desktop application built on eframe/egui with one window and independent workspace tabs. Each tab is a live shell plus explorer; OpenCode v2 is an optional enhancement started by the user. This page describes the implemented structure and ownership boundaries. Product requirements stay in [`specs/changes/archive/20261029-opencode-session-follow-v1/spec.md`](../specs/changes/archive/20261029-opencode-session-follow-v1/spec.md); UI requirements stay in [`specs/made-ui-ux.md`](../specs/made-ui-ux.md) plus the active delta [`specs/changes/explorer-git-view/spec.md`](../specs/changes/explorer-git-view/spec.md).

## Crate map

| Crate | Targets | Responsibility | Workspace dependencies |
|---|---|---|---|
| [`made-app`](../crates/made-app/src/lib.rs) | library + binary | Composition root: window, workspace collection, terminals, explorer/preview composition, picker, diagnostics, metrics, CLI. | all domain crates |
| [`made-terminal`](../crates/made-terminal/src/lib.rs) | library | `TerminalCore` boundary, VT screen state, PTY adapter, input encoding, render model. | `made-platform` |
| [`made-platform`](../crates/made-platform/src/lib.rs) | library | Only OS-specific crate: PTY/ConPTY, shell launch and cwd hooks, process lifecycle, app paths, clipboard, IME, native backdrop, home resolution. | none |
| [`made-explorer`](../crates/made-explorer/src/lib.rs) | library | Lazy tree, watcher, canonical path guard, read-only preview, highlighting, root generations. | none |
| [`made-bridge`](../crates/made-bridge/src/lib.rs) | library | Loopback WebSocket server, protocol state machine, session snapshots, config composition and immutability guard, token helpers. | none |

The OpenCode plugin is TypeScript outside the Cargo workspace at [`plugins/opencode-made`](../plugins/opencode-made/README.md). The Phase 0 gate harness was removed after the gate; its outcomes are preserved in the [Phase 0 gate record](records/20261025-phase-0-gate.md), with integration details in [OpenCode bridge](integration/opencode.md).

## Dependency direction

```text
made-app (binary + library, composition root)
├── made-terminal ──> made-platform
├── made-bridge
├── made-explorer
└── made-platform (leaf, OS capabilities)
```

Rules enforced across the workspace:

1. `made-app` is the only composition root; no crate depends on it.
2. Domain crates do not form cycles. `made-terminal` may depend on `made-platform`; `made-bridge` and `made-explorer` are independent siblings.
3. `made-platform` is a leaf and owns OS capabilities, not product behavior.
4. OS-specific code and APIs live only in `made-platform`; test-only fixture selection is the exception.
5. Only `made-bridge` binds a network socket.
6. Only the plugin calls OpenCode APIs. `made-app` consumes normalized snapshots and never parses terminal output or talks to OpenCode.
7. Only `made-explorer` reads workspace file contents for display. Read-only git metadata and closed-allowlist status are resolved by [`made-app::worktree`](../crates/made-app/src/worktree.rs); bridge config code is the only reader of the user's OpenCode config. Git observation is read-only.
8. Owned values and events cross crate boundaries; the app event loop owns UI state and no cross-crate mutable singleton exists.

Heavy UI/GPU dependencies are confined to `made-app`: eframe 0.37 brings egui, winit, and wgpu. Domain crates stay UI-free. `made-terminal` uses alacritty_terminal 0.26 behind `TerminalCore`; `made-explorer` uses notify and syntect; `made-bridge` uses tungstenite, serde, sha2, getrandom, and thiserror.

## Supported targets

Made uses one source tree and one UI workflow for **macOS arm64/x64**, **Linux x64 on X11 and Wayland**, and **Windows x64** with native OpenCode. WSL-hosted OpenCode is not a substitute for native Windows support; it remains out of scope. Build and automated coverage is defined for all four targets, while native runtime validation is tracked in the [platform matrix](testing/platform-matrix.md).

## Runtime model

[`AppModel`](../crates/made-app/src/app.rs) owns an ordered collection of workspaces keyed by stable `WorkspaceId` and one active id. A [`Workbench`](../crates/made-app/src/workbench.rs) is the complete per-tab runtime. It owns:

- one terminal pane, PTY/process session, and launch-local shell bootstrap;
- one explorer root/generation/tree/watcher/preview and its width/visibility;
- one loopback bridge, token, pane id, and optional OpenCode connection/state;
- session-follow precedence, session map/focus state, worktree metadata, diagnostics, and metrics;
- left-surface state (`Terminal` or `Preview`) and focus transfer.

Opening another folder creates a new workbench without disturbing siblings. Closing a workspace shuts down only its own PTY/process group, bridge, queues, watcher, bootstrap, and pending work; late results are generation- or id-gated. Closing the final workspace returns to the picker and starts no process. There is no restart persistence.

Module boundaries in `made-app`:

| Module | Responsibility |
|---|---|
| [`app.rs`](../crates/made-app/src/app.rs) | Workspace collection, active id, picker transitions, per-tab ticking, shutdown. |
| [`ui.rs`](../crates/made-app/src/ui.rs) | Calm Editor composition, header/tabs, explorer and preview surfaces, diagnostics, picker. |
| [`theme.rs`](../crates/made-app/src/theme.rs) | Tokens, style installation, glass painting, clear color and transparency switch. |
| [`workbench.rs`](../crates/made-app/src/workbench.rs) | One workspace runtime: shell/PTY, root follow, bridge, optional OpenCode, diagnostics, metrics. |
| [`terminal_view.rs`](../crates/made-app/src/terminal_view.rs), [`render.rs`](../crates/made-app/src/render.rs) | Terminal input/visibility, selection/copy/IME, damage-driven grid rendering. |
| [`explorer_view.rs`](../crates/made-app/src/explorer_view.rs) | Full-height tree and source-control view, full-left preview, per-workspace width/visibility. |
| [`session_follow.rs`](../crates/made-app/src/session_follow.rs) | Shell/OpenCode root precedence, generations, unavailable roots. |
| [`session_map.rs`](../crates/made-app/src/session_map.rs), [`session_focus.rs`](../crates/made-app/src/session_focus.rs) | Optional session data and focus correlation/timeout. |
| [`integration_status.rs`](../crates/made-app/src/integration_status.rs) | Inactive, connected, degraded, unsupported states and reasons. |
| [`diagnostics.rs`](../crates/made-app/src/diagnostics.rs) | Selected workspace identity, cwd capability/root/source, versions, bounded redacted log. |
| [`metrics.rs`](../crates/made-app/src/metrics.rs) | Startup, cwd, activation, preview/layout, input and frame budgets. |

## Data flow

### Shell workspace

```text
launch / picker / home folder
  -> platform shell launch plan (user profile + launch-local OSC 7 emitter)
  -> PTY shell in workspace folder
  -> terminal input/output + VT state
  -> validated OSC 7 cwd (bounded local absolute path)
  -> session_follow arbitration -> explorer root generation
  -> tree / watcher / preview in the owning tab
```

OpenCode is not in this path and cannot gate startup or normal shell use.

### Optional OpenCode follow

```text
OpenCode v2 TUI (user-run child inside the workspace PTY)
  -> app-owned plugin over loopback WebSocket
  -> made-bridge (per-workspace token/pane; validates hello/version/revision)
  -> normalized SessionSnapshot
  -> session_follow root precedence: connected active session directory
     > usable connected view directory (tabs enabled, no active session)
     > latest shell cwd
  -> explorer root in that tab
  -> disconnect/exit falls back to latest shell cwd
```

The connected view directory is cleared on disconnect/exit, unsupported integration, session tabs becoming unavailable, or a new connection epoch.

A snapshot or event from another workspace, an unauthenticated client, a foreign disconnect, or a stale epoch cannot change a healthy workspace. Focus requests are scoped to the owning bridge and to open sessions.

### Preview and layout

Explorer selection -> canonical path guard -> preview loader/refresh -> full-left read-only surface. The PTY keeps running with bounded hidden output while preview is visible. Root changes clear preview and stale reads. Explorer hide/restore/resize is per workspace and never touches the shell process.

### Git observation

[`made-app::worktree`](../crates/made-app/src/worktree.rs) also reads closed-allowlist git status (`GitCommand::Status`) off the UI thread. The explorer paints that snapshot and does not invoke git. There is no git write. Porcelain v1 paths are always relative to the repository root, even with pathspec `.`. `read_status` canonicalizes the explorer root and toplevel before `resolve_status_path` strips the prefix. On macOS, `/var` versus `/private/var` drops every entry otherwise.

## Threading and bounded work

- The app event loop owns UI state; background workers report through bounded channels.
- PTY reads and VT parsing run off the UI thread. Each pane is budgeted per frame; inactive workspaces consume bounded output but do not paint grids.
- Tree loads and preview reads run off the UI thread, are cancellable, and carry root generations. Git status runs off the UI thread on `made-git-status` and is generation-gated; the path-prefix rule is under Git observation.
- Active repaint cadence is 14 ms. Paint timing uses an end-of-pass callback so activation, cwd, preview, and layout metrics end at the first submitted paint pass.
- Bridge socket/accept threads never block the event loop. One pending state snapshot per authenticated epoch is kept and replaced in place; lifecycle events outrank best-effort notices.
- Hidden-tab queues and scrollback are bounded; final software measurements recorded zero hidden-grid paints.

## Shared-contract ownership

| Contract | Owner | Consumers |
|---|---|---|
| `SessionSnapshot`, session records, protocol messages | `made-bridge::state`, `made-bridge::server` | `made-app` |
| Integration status and reasons | `made-app::integration_status` | views, diagnostics |
| Workspace root, effective root/source, shell precedence | `made-app::session_follow` | views, explorer root |
| Root generation and stale-event rejection | `made-explorer::root_switch` | tree, watcher, preview |
| Canonical path guard | `made-explorer::paths` | tree, preview, watcher |
| Terminal screen/render model | `made-terminal::emulation` | `made-app::render`, terminal view |
| Platform capabilities, launch plans, app paths, backdrop status | `made-platform` | `made-terminal`, `made-app` |
| OpenCode config composition and no-write guard | `made-bridge::config`, `config_guard` | app launch path |
| Read-only git status and porcelain path prefix | `made-app::worktree` | explorer source-control view |

`made-explorer` receives an already-validated canonical path plus a generation token, not an app-owned `WorkspaceRoot`. `made-app` keeps the richer root/source/display model.

## Change discipline

- Do not introduce a reverse dependency to reuse app types; move shared models upstream or pass owned values.
- Do not duplicate snapshots, path guards, or platform traits in sibling crates.
- Do not add OS branches outside `made-platform`.
- Do not add a network listener outside `made-bridge`.
- Keep shell and explorer usable when optional integration is absent, disconnected, or unsupported.
- Keep hidden workspaces bounded and never paint inactive grids.
- Record deviations in the relevant spec change before implementation.
