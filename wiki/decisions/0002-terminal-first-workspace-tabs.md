# 0002: Terminal-First Workspace Tabs

**Status:** Accepted for v1.

## Context

The product is a terminal-first agent workspace, not a chat application. Users need to open several folders in one window and keep each shell, Explorer, preview, and optional OpenCode connection alive while switching. An earlier layout used a picker-first start and stacked Explorer/preview surfaces; the approved revision makes the shell immediate and gives the terminal the full left pane.

## Decision

One native window with ordered, independent workspace tabs. Each tab is a complete `Workbench` rather than a view over shared global state.

- A new tab opens another folder without stopping sibling shells.
- Each tab owns its PTY/process session, terminal, Explorer root/generation/tree/watcher/preview, bridge/token/pane, optional OpenCode state, session map, layout/visibility, and focus.
- The left surface shows either the live terminal or a full-left read-only preview, never both or a stacked split.
- OpenCode sessions are an optional map inside the selected workspace, not workspace tabs.
- Hidden tabs continue bounded output/bridge work but never paint grids.
- Closing a tab releases only its resources; closing the last tab returns to the picker and starts no process.
- No restart persistence, terminal splits, or multi-window.

## Consequences

- UI widget ids, splitter width, visibility, preview mode, and focus must be workspace-scoped; shared egui state would leak between tabs.
- Late tree/preview/bridge results need workspace id plus generation/epoch checks.
- Shutdown and process cleanup must be per-workbench; shared platform cleanup must not kill another tab's children.
- Hidden tabs need bounded queues/scrollback and fair ticking to stay current without unbounded memory or painting.
- The design intentionally excludes split panes, multi-window, and tab restoration.

## Evidence

[`specs/changes/opencode-session-follow-v1/spec.md`](../../specs/changes/opencode-session-follow-v1/spec.md) independent-tabs requirement, [core design](../../specs/changes/opencode-session-follow-v1/design.md) D18, [UI design](../../specs/changes/made-ui-ux-v1/design.md), [`crates/made-app/tests/workspace_tabs.rs`](../../crates/made-app/tests/workspace_tabs.rs), [`crates/made-app/tests/ui_model.rs`](../../crates/made-app/tests/ui_model.rs). Current evidence: [v1 snapshot](../records/20261026-v1-evidence.md).
