# 0001: OpenCode Plugin and Loopback Bridge

**Status:** Accepted for v1.

## Context

Made needs OpenCode's active session identity, open root-session tabs, and each session's working directory so the Explorer can follow sessions and worktrees. It also needs a supported way to focus an OpenCode session from Made. The alternatives were ACP, a separate `opencode serve` plus attach/proxy, terminal screen scraping, and terminal-title scraping. ACP is not the TUI extension path and still needs tab/worktree state; a server adds lifecycle and port management; screen/title scraping is brittle and cannot reliably identify a session or worktree, nor focus safely.

## Decision

Use the official OpenCode v2 CLI/TUI plugin API as the observation boundary, and a Made-owned loopback WebSocket bridge as the transport boundary.

- One bridge listener per workspace tab, bound to `127.0.0.1:0`, path `/made`.
- Per-workspace 256-bit token and pane id; authenticated `hello` before any state; one authenticated connection per pane; stale revisions and foreign epochs discarded.
- The plugin observes `context.ui.tabs` and `context.data.session`, normalizes full snapshots, and handles focus. The app consumes snapshots; it never parses terminal output or calls OpenCode APIs.
- Focus is a request/response command, not synthetic keystrokes.
- Made does not launch or manage OpenCode automatically or as a service; the user runs `opencode` in a terminal or starts it from the `+` menu.

## Consequences

- The plugin must remain a directory package with `package.json` and `tui.ts`; a bare `.ts` entry did not load in OpenCode v2.1.4.
- The integration is coupled to OpenCode's supported plugin surface, but protocol versioning and an `unsupported` state keep incompatibility from crashing the TUI. Minor API drift remains a risk.
- Per-tab bridges avoid a multiplexing protocol and preserve tab isolation, at the cost of one listener per live tab.
- The bridge adds a local trust boundary that must authenticate before disclosure and redact tokens.
- Observed 2.1.4 API divergences (no `unread`, synchronous `session.get`, focus can open closed tabs, reactive `context.location`, transient null active tab) are handled in the plugin/app protocol. See the [Phase 0 record](../records/20261025-phase-0-gate.md).
- Managed versus standalone OpenCode service topology is OpenCode-owned; Made must not collide with or attach to a user service.

## Evidence

[`specs/changes/archive/20261029-opencode-session-follow-v1/design.md`](../../specs/changes/archive/20261029-opencode-session-follow-v1/design.md) D2/D4, the [Phase 0 gate record](../records/20261025-phase-0-gate.md), [OpenCode bridge](../integration/opencode.md), [`plugins/opencode-made/README.md`](../../plugins/opencode-made/README.md). Implementation: [`made-bridge`](../../crates/made-bridge/src/server.rs), [`plugins/opencode-made`](../../plugins/opencode-made/tui.ts). The Phase 0 harness was removed after the gate.

## Amendment (2026-10-29)

The `+` new-tab menu is an explicit launch path. When `opencode` is found on `PATH`, the user can choose `OpenCode`, which starts the detected executable directly as the PTY child of a new home-directory tab. Made still never launches OpenCode automatically. This amends only the launch note in Decision; the rest of the decision is unchanged.
