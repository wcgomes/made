# Tasks: opencode-session-follow-v1

> **Checkbox scope:** checked items reflect implementation and automated Linux evidence in this
> tree, not cross-platform manual validation; tasks with broader feature descriptions may still
> have manual-only sub-items recorded in the [v1 evidence record](../../../wiki/records/20261026-v1-evidence.md). Tasks 4.8 (manual platform results) and 5.11 (all-OS end-to-end) remain open, and checkpoint boxes remain unchecked until every scenario has evidence. See the [v1 evidence record](../../../wiki/records/20261026-v1-evidence.md).

> **Approved in-place terminal-first revision:** Phases 0–5 are historical work/checkpoints and retain their 84 historical checks honestly; their old implementation descriptions are superseded by `spec.md`. Phases 6–10 checkboxes reflect Linux code/automated evidence only, not cross-platform delivery: real zsh/fish/PowerShell, native Linux/macOS/Windows, native pointer/keyboard, native home resolution and hardware-GPU evidence remain pending. Phase 10 implementation/test items are checked; its manual checkpoints stay unchecked, as do 4.8, 5.11, 9.6 and 9.8, until every native/platform scenario has evidence. See the [v1 evidence record](../../../wiki/records/20261026-v1-evidence.md).
>
> **UI/UX split:** UI-owned tasks and evidence moved to `specs/changes/made-ui-ux-v1/tasks.md` (former core 1.20–1.21, 2.12–2.14, 3.12, 8.1–8.2, 8.5–8.9, and Phase 11). Core retains 8.3–8.4 diagnostics data and every runtime/integration/security/preview-safety task. Core checkpoints for moved UI scenarios rely on the UI change; no UI scenario is silently dropped.

## 0. Phase 0 — Bridge spike gate (blocking)

> The Phase 0 spike harness was removed after the gate passed. Historical task paths below point to the preserved gate record; outcomes, acceptance evidence, and residual risks are in [`wiki/records/20261025-phase-0-gate.md`](../../../wiki/records/20261025-phase-0-gate.md).

- [x] 0.1 Record the spike acceptance matrix and the fixture workspace for plugin loading, active-tab tracking, worktree location changes, focus, and config immutability (path: `wiki/records/20261025-phase-0-gate.md`)
- [x] 0.2 Create a fixture OpenCode config with a pre-existing user plugin entry and a baseline content hash (path: `wiki/records/20261025-phase-0-gate.md`)
- [x] 0.3 Write failing spike checks that assert plugin load, active-tab state, worktree directory change, focus result, and unchanged config hash (path: `wiki/records/20261025-phase-0-gate.md`)
- [x] 0.4 Implement the spike plugin that reports tabs and session locations and accepts focus commands (path: `wiki/records/20261025-phase-0-gate.md`)
- [x] 0.5 Implement the spike bridge harness with loopback binding, token check, and focus command dispatch (path: `wiki/records/20261025-phase-0-gate.md`)
- [x] 0.6 Implement the spike launcher that injects the composed config into a normal terminal OpenCode process without writing user config (path: `wiki/records/20261025-phase-0-gate.md`)
- [x] 0.7 Run the spike against OpenCode v2 in a normal terminal, not the Made app, and record evidence for every acceptance check (path: `wiki/records/20261025-phase-0-gate.md`)
- [x] 0.8 Record the gate decision and unresolved risks; block all app UI work if any gate check fails (path: `wiki/records/20261025-phase-0-gate.md`)

## Checkpoint

- [ ] Gate: "Plugin loads on first OpenCode launch" is validated at spike level; the plugin loads from injected config and connects.
- [ ] Gate: active-tab tracking and "Switch to a session in a different worktree" location changes are validated at spike level.
- [ ] Gate: "Click focuses session" is validated at spike level; the plugin receives focus and OpenCode changes the active session.
- [ ] Gate: "User config unchanged" is validated at spike level; the fixture config hash is identical before and after.
- [ ] Gate: no app UI work starts until all Phase 0 checks pass; failures and unresolved risks are recorded in `wiki/records/20261025-phase-0-gate.md`.

## 1. Phase 1 — Native shell, terminal, and workspace launch

- [x] 1.1 Write a failing launch test for an explicit workspace argument, terminal cwd, and explorer root (path: `crates/made-app/tests/launch_workspace.rs`)
- [x] 1.2 Implement app bootstrap and explicit workspace argument handling (path: `crates/made-app/src/main.rs`)
- [x] 1.3 Write a failing PTY test for shell spawn, cwd, resize, and termination (path: `crates/made-terminal/tests/pty.rs`)
- [x] 1.4 Implement the PTY adapter over the platform layer (path: `crates/made-terminal/src/pty.rs`)
- [x] 1.5 Write failing terminal emulation tests for truecolor, alternate screen, scrollback, title, and Unicode width (path: `crates/made-terminal/tests/emulation.rs`)
- [x] 1.6 Implement the terminal core adapter and screen state (path: `crates/made-terminal/src/emulation.rs`)
- [x] 1.7 Write failing input encoding tests for keyboard, mouse, bracketed paste, and focus reporting (path: `crates/made-terminal/tests/input.rs`)
- [x] 1.8 Implement the input encoder (path: `crates/made-terminal/src/input.rs`)
- [x] 1.9 Write a failing renderer test for damage-driven grid drawing, frame pacing, and resize (path: `crates/made-app/tests/renderer.rs`)
- [x] 1.10 Implement the terminal grid renderer and frame pacing (path: `crates/made-app/src/render.rs`)
- [x] 1.11 Write a failing platform contract test for PTY, paths, clipboard, IME, and process lifecycle (path: `crates/made-platform/tests/contract.rs`)
- [x] 1.12 Implement the platform trait and shared types (path: `crates/made-platform/src/lib.rs`)
- [x] 1.13 Implement the Linux platform backend [P] (path: `crates/made-platform/src/linux.rs`)
- [x] 1.14 Implement the macOS platform backend [P] (path: `crates/made-platform/src/macos.rs`)
- [x] 1.15 Implement the Windows platform backend with ConPTY [P] (path: `crates/made-platform/src/windows.rs`)
- [x] 1.16 Write a failing OpenCode TUI smoke test for colors, alternate screen, resize, mouse, paste, Unicode/emoji, and IME (path: `crates/made-app/tests/opencode_tui.rs`)
- [x] 1.17 Wire the terminal view, selection, copy/paste, title, and focus reporting (path: `crates/made-app/src/terminal_view.rs`)
- [x] 1.18 Write a failing invalid-workspace test for missing and unreadable folders (path: `crates/made-app/tests/invalid_workspace.rs`)
- [x] 1.19 Implement workspace validation and the non-blocking error state (path: `crates/made-app/src/workspace.rs`)

> Picker UI test/implementation (former 1.20–1.21) moved to `made-ui-ux-v1` tasks 1.5–1.6. Core launch behavior remains covered by 10.1–10.6.

## Checkpoint

- [ ] verify "Workspace chosen via picker or argument"
- [ ] verify "No workspace provided"
- [ ] verify "Workspace remains usable before OpenCode starts"
- [ ] verify "Invalid workspace folder"
- [ ] verify "OpenCode TUI renders"
- [ ] verify "Resize"
- [ ] verify "Mouse"
- [ ] verify "Bracketed paste"
- [ ] verify "Unicode and emoji"
- [ ] verify "IME composition"
- [ ] verify "Scrollback and copy"
- [ ] verify "Title and focus reporting"
- [ ] verify the platform contract test passes for the Linux, macOS, and Windows backends; runtime parity is validated in Phase 4.

## 2. Phase 2 — Explorer and read-only preview

- [x] 2.1 Write a failing tree test for lazy directory listing, ordering, unreadable entries, and expansion and selection state (path: `crates/made-explorer/tests/tree.rs`)
- [x] 2.2 Implement the lazy tree model with unreadable rows and expansion and selection state (path: `crates/made-explorer/src/tree.rs`)
- [x] 2.3 Write a failing watcher test for create, delete, rename, one-second update latency, debounce, and expansion and selection preservation (path: `crates/made-explorer/tests/watch.rs`)
- [x] 2.4 Implement the filesystem watcher, debounce, update latency, and expansion and selection preservation (path: `crates/made-explorer/src/watch.rs`)
- [x] 2.5 Write a failing path-boundary test for canonicalization, symlink escape, and traversal rejection (path: `crates/made-explorer/tests/paths.rs`)
- [x] 2.6 Implement the root-constrained path resolver (path: `crates/made-explorer/src/paths.rs`)
- [x] 2.7 Write a failing preview test for text, binary, large, deleted, and truncated files (path: `crates/made-explorer/tests/preview.rs`)
- [x] 2.8 Implement the read-only preview loader with size and binary guards (path: `crates/made-explorer/src/preview.rs`)
- [x] 2.9 Write a failing preview-refresh test for external change and debounce (path: `crates/made-explorer/tests/preview_refresh.rs`)
- [x] 2.10 Implement preview auto-refresh (path: `crates/made-explorer/src/preview_refresh.rs`)
- [x] 2.11 Write a failing read-only UI test asserting no edit/save controls and inert save shortcuts (path: `crates/made-app/tests/read_only_ui.rs`)
- [x] 2.15 Write a failing root-switch test for stale loads and stale filesystem events (path: `crates/made-explorer/tests/root_switch.rs`)
- [x] 2.16 Implement root generation tracking and stale-load/event rejection (path: `crates/made-explorer/src/root_switch.rs`)

> Explorer tree/preview panel implementation (former 2.12) moved to `made-ui-ux-v1` task 2.10. Syntax-highlighting presentation/test (former 2.13–2.14) moved to `made-ui-ux-v1` tasks 2.12–2.13. Core retains read-only/safety tests and the preview loader/path guard.

## Checkpoint

- [ ] verify "Created file appears"
- [ ] verify "Deleted file disappears"
- [ ] verify "Renamed file updates"
- [ ] verify "Active root switch discards stale events"
- [ ] verify "Unreadable directory shows an error row"
- [ ] verify "Expansion and selection preserved"
- [ ] verify "Binary file"
- [ ] verify "Large file"
- [ ] verify "External change refresh"
- [ ] verify "File disappears"
- [ ] verify "Root change clears preview"
- [ ] verify "Preview keeps terminal process and input isolated"
- [ ] verify "Path traversal blocked"
- [ ] verify "Read-only preview enforced"

## 3. Phase 3 — Bridge integration, session map, worktree follow, and fallback

- [x] 3.1 Write a failing protocol test for hello, authentication, version negotiation, malformed messages, and revision ordering (path: `crates/made-bridge/tests/protocol.rs`)
- [x] 3.2 Implement the bridge server and protocol state machine (path: `crates/made-bridge/src/server.rs`)
- [x] 3.3 Write a failing state-normalization test for sessions, active session, directories, and stale revisions (path: `crates/made-bridge/tests/state.rs`)
- [x] 3.4 Implement the session snapshot model and validation (path: `crates/made-bridge/src/state.rs`)
- [x] 3.5 Write a failing config-injection test for existing plugins, existing inline config, forced tabs settings, and no file writes (path: `crates/made-bridge/tests/config_injection.rs`)
- [x] 3.6 Implement config composition and child environment injection (path: `crates/made-bridge/src/config.rs`)
- [x] 3.7 Write a failing plugin lifecycle test for connect, reconnect, heartbeat, focus result, and unload (path: `plugins/opencode-made/tests/lifecycle.test.ts`)
- [x] 3.8 Implement the OpenCode plugin (path: `plugins/opencode-made/src/index.ts`)
- [x] 3.9 Write a failing session-follow test for active-session change, different worktree, stale event, and missing directory (path: `crates/made-app/tests/session_follow.rs`)
- [x] 3.10 Implement the session-follow coordinator and explorer root switching (path: `crates/made-app/src/session_follow.rs`)
- [x] 3.11 Write a failing session-map test for open, close, active, busy, attention, and long labels (path: `crates/made-app/tests/session_map.rs`)

> Session-map UI implementation (former 3.12) moved to `made-ui-ux-v1` task 3.5. Core retains the map data test 3.11 and focus request/response behavior in 3.13–3.14.
- [x] 3.13 Write a failing focus test for click, focus command, state update, and timeout failure (path: `crates/made-app/tests/session_focus.rs`)
- [x] 3.14 Implement session focus command dispatch and error handling (path: `crates/made-app/src/session_focus.rs`)
- [x] 3.15 Write a failing worktree-indicator test for linked worktree, branch, non-git folder, and missing git (path: `crates/made-app/tests/worktree.rs`)
- [x] 3.16 Implement the read-only git metadata reader and worktree indicator (path: `crates/made-app/src/worktree.rs`)
- [x] 3.17 Write a failing degraded-state test for plugin missing, OpenCode missing or older than v2, extension loading disabled, disconnect, version mismatch, OpenCode exit, tabs unavailable, config failure, and reconnect (path: `crates/made-app/tests/degraded.rs`)
- [x] 3.18 Implement the integration status state machine, OpenCode version gate, and last-known-root fallback (path: `crates/made-app/src/integration_status.rs`)
- [x] 3.19 Write a failing config-immutability integration test that hashes the user config before and after a session (path: `crates/made-bridge/tests/config_immutability.rs`)
- [x] 3.20 Implement launch-time hash verification and the diagnostics event (path: `crates/made-bridge/src/config_guard.rs`)

## Checkpoint

- [ ] verify "Plugin loads on first OpenCode launch"
- [ ] verify "Existing user plugins are preserved"
- [ ] verify "No OpenCode session yet"
- [ ] verify "Switch between sessions in the same worktree"
- [ ] verify "Switch to a session in a different worktree"
- [ ] verify "Active session returns to main checkout"
- [ ] verify "Rapid tab switching"
- [ ] verify "Active session directory unavailable"
- [ ] verify "OpenCode-created worktree becomes active"
- [ ] verify "Worktree metadata unavailable"
- [ ] verify "Worktree removal by OpenCode"
- [ ] verify "Multiple open sessions"
- [ ] verify "Session closes"
- [ ] verify "Click focuses session"
- [ ] verify "Focus command fails"
- [ ] verify "Plugin unavailable"
- [ ] verify "OpenCode missing or older than v2"
- [ ] verify "OpenCode started with extension loading disabled"
- [ ] verify "Bridge disconnects mid-session"
- [ ] verify "Incompatible protocol version"
- [ ] verify "OpenCode exits normally"
- [ ] verify "Reconnect"
- [ ] verify "Session tabs unavailable"
- [ ] verify "Configuration composition fails"
- [ ] verify "User config unchanged"

## 4. Phase 4 — Cross-platform builds and platform validation

- [x] 4.1 Write a failing cross-platform contract test that asserts the same UI workflow and platform isolation (path: `crates/made-platform/tests/cross_platform.rs`)
- [x] 4.2 Add the CI matrix for macOS arm64/x64, Linux x64, and Windows x64 with build and test steps (path: `.github/workflows/ci.yml`)
- [x] 4.3 Add the Linux X11 and Wayland smoke job [P] (path: `.github/workflows/ci-linux-display.yml`)
- [x] 4.4 Add the Windows ConPTY and native OpenCode TUI smoke job [P] (path: `.github/workflows/ci-windows.yml`)
- [x] 4.5 Add the macOS OpenCode TUI smoke job [P] (path: `.github/workflows/ci-macos.yml`)
- [x] 4.6 Add development artifact upload for every target (path: `.github/workflows/ci-artifacts.yml`)
- [x] 4.7 Record platform-specific validation steps and known constraints (path: `wiki/testing/platform-matrix.md`)
- [ ] 4.8 Run manual validation on macOS, Linux X11, Linux Wayland, and Windows and record results (path: `wiki/records/20261026-v1-evidence.md`)

## Checkpoint

- [ ] verify "CI builds all targets"
- [ ] verify "Linux X11 and Wayland"
- [ ] verify "Windows native OpenCode"
- [ ] verify "macOS"
- [ ] verify "No per-OS UI rewrite"

## 5. Phase 5 — Diagnostics, security hardening, performance, and UX validation polish

- [x] 5.1 Write a failing diagnostics test for statuses, active session, integration version, and redacted logs (path: `crates/made-app/tests/diagnostics.rs`)
- [x] 5.2 Implement the diagnostics panel and structured log access (path: `crates/made-app/src/diagnostics.rs`)
- [x] 5.3 Write a failing bridge security test for unauthorized clients, non-loopback bind, malformed hello, and connection limits [P] (path: `crates/made-bridge/tests/security.rs`)
- [x] 5.4 Implement bridge hardening and token redaction (path: `crates/made-bridge/src/security.rs`)
- [x] 5.5 Write a failing performance-budget test for startup, session switch, input latency, and sustained output [P] (path: `crates/made-app/tests/performance.rs`)
- [x] 5.6 Implement metrics and budget assertions (path: `crates/made-app/src/metrics.rs`)
- [x] 5.7 Write a failing no-webview check [P] (path: `crates/made-app/tests/no_webview.rs`)
- [x] 5.8 Implement the process-level assertion and build metadata check (path: `crates/made-app/src/runtime_guard.rs`)
- [x] 5.9 Write a failing scope-boundary test for absent non-OpenCode agent integrations, absent replacement AI/chat UI, and absent worktree management controls (path: `crates/made-app/tests/scope_boundary.rs`)
- [x] 5.10 Write the end-to-end UX validation checklist covering every spec scenario (path: `tests/e2e/ux_validation.md`)
- [ ] 5.11 Run end-to-end validation on each supported OS and record evidence (path: `wiki/records/20261026-v1-evidence.md`)
- [x] 5.12 Document v1 constraints and known limitations (path: `wiki/testing/platform-matrix.md`, `wiki/records/20261026-v1-evidence.md`)
- [x] 5.13 Perform the final readiness review against the spec and close or record every remaining risk (path: `wiki/testing/platform-matrix.md`, `wiki/records/20261026-v1-evidence.md`)

## Checkpoint

- [ ] verify "Connected diagnostics"
- [ ] verify "Degraded diagnostics"
- [ ] verify "Unsupported integration"
- [ ] verify "Logs redact token"
- [ ] verify "Unauthorized bridge client rejected"
- [ ] verify "Non-loopback bind"
- [ ] verify "Plugin from app-owned path"
- [ ] verify "Startup budget"
- [ ] verify "Session switch budget"
- [ ] verify "Input latency"
- [ ] verify "Sustained output"
- [ ] verify "No webview"
- [ ] verify "Unsupported agent not offered"
- [ ] verify "No replacement chat UI"
- [ ] verify "Worktree lifecycle remains in OpenCode"
- [ ] verify "Direct launch"
- [ ] verify every scenario in `spec.md` has passing evidence in `wiki/records/20261026-v1-evidence.md`; no Phase 0 gate failure remains open.

## Coverage summary

| Requirement | Task coverage | Checkpoint |
|---|---|---|
| Workspace Launch | 1.1–1.2, 1.18–1.19, 10.1–10.6 | Phase 1, Phase 10 (picker UI in `made-ui-ux-v1`) |
| Shell Directory Follow | 6.1–6.7, 6.11 (including 6.3a–6.3b), 8.3–8.4 | Revision 6, 8, 9 |
| Independent Workspace Tabs | 7.1–7.9 (including 7.7a), 9.1–9.4 | Revision 7, 9 |
| OpenCode Integration Loading | 0.1–0.8, 3.5–3.8, 3.19–3.20 | Phase 0, Phase 3 |
| Active Session Follow | 3.9–3.10, 3.17–3.18 | Phase 3 |
| Worktree Awareness | 3.9–3.10, 3.15–3.16 | Phase 3 |
| Session Map | 3.11, 3.13–3.14; UI tasks 3.1–3.2, 3.5 for map presentation | Phase 3 (presentation in `made-ui-ux-v1`) |
| Read-Only File Preview | 2.5–2.11, 2.15–2.16; UI tasks 2.2, 2.10, 2.12–2.13 for preview/explorer/highlight presentation | Phase 2, UI checkpoint (presentation in `made-ui-ux-v1`) |
| Explorer Live Updates | 2.1–2.4, 2.15–2.16 | Phase 2 (no-Refresh UI in `made-ui-ux-v1`) |
| Degraded Integration | 3.1–3.8, 3.17–3.18 | Phase 3 (quiet chrome in `made-ui-ux-v1`) |
| Terminal Fidelity | 1.3–1.17 | Phase 1 |
| Cross-Platform Delivery | 4.1–4.8, 10.3–10.4 | Phase 4, Phase 10 |
| Security and Local-Only Integration | 2.5–2.6, 2.11, 3.5–3.6, 3.19–3.20, 5.3–5.4, 10.4–10.6 | Phase 2, Phase 3, Phase 5, Phase 10 |
| Responsiveness and Lightness | 5.5–5.8, 9.1–9.4 | Phase 5, Phase 9 |
| Diagnostics | 5.1–5.4, 8.3–8.4 | Phase 5, Phase 8 (presentation in `made-ui-ux-v1`) |
| V1 Scope Boundary | 0.7–0.8, 3.15–3.16, 5.9–5.11 | Phase 0, Phase 3, Phase 5 |

UI requirements and their task coverage live in `specs/changes/made-ui-ux-v1/tasks.md`; this table covers core scenarios only.

## 6. Revision — Terminal-first startup and safe cwd follow

Execute each test task first and record a meaningful failing assertion against current behavior before its implementation tasks. Retain existing tests as regressions; do not weaken assertions to match fallback. The shell matrix must distinguish required supported interactive modes from explicitly unsupported/restricted modes. Phase 6 precedes workspace composition; isolated bridge security work 7.1–7.2 may proceed in parallel with it.

- [x] 6.1 Write capability/bootstrap tests for real bash/zsh/fish/PowerShell cd (spaces/Unicode), existing profile/prompt-hook preservation, initial report, unsupported cmd/unknown/policy modes, user-file hashes and tab-local cleanup; confirm failures (path: `crates/made-platform/tests/shell_integration.rs`)
- [x] 6.2 Implement launch-local shell bootstrap plans and explicit capability/reason results; no persistent config writes, policy bypass, prompt/title scraping or runtime PTY command injection (path: `crates/made-platform/src/shell_integration.rs`)
- [x] 6.3 Wire shell launch descriptions, child arguments/environment and process-owned bootstrap cleanup through the existing platform boundary; retain default startup semantics (path: `crates/made-platform/src/lib.rs`)
- [x] 6.3a Extend PTY tests to prove launch plans preserve resolved shell arguments/environment, initial cwd, bootstrap lifetime and relaunch behavior; confirm failures before wiring plans into spawn (path: `crates/made-terminal/tests/pty.rs`)
- [x] 6.3b Apply the platform launch plan at the existing resolved-shell-to-PTY-request boundary and tie bootstrap resources to process lifetime, retaining nonblocking IO/termination behavior (path: `crates/made-terminal/src/pty.rs`)
- [x] 6.4 Extend receiver tests for bounded local absolute OSC 7 reports, URI escaping/Unicode/Windows paths, malformed/remote/relative data and absence of title-based cwd; confirm new failures (path: `crates/made-terminal/tests/emulation.rs`)
- [x] 6.5 Harden cwd receiver validation only as needed by the new failing cases without changing terminal input/VT behavior (path: `crates/made-terminal/src/emulation.rs`)
- [x] 6.6 Extend precedence tests for latest shell cwd tracking under active OpenCode, no-active-directory snapshots, disconnect/exit/unsupported fallback and unavailable roots; confirm failures (path: `crates/made-app/tests/session_follow.rs`)
- [x] 6.7 Implement shared shell/OpenCode root precedence and generation invalidation, never retaining old agent root ahead of latest shell cwd (path: `crates/made-app/src/session_follow.rs`)
- [x] 6.8 Extend launch tests for absent/never-started OpenCode, no version/handshake wait and no automatic launch; confirm failures (path: `crates/made-app/tests/launch_workspace.rs`)
- [x] 6.9 Extend integration lifecycle tests for inactive shell use, config failure, missing/old/disabled plugin, tabs unavailable, mismatch, disconnect, normal exit and reconnect with diagnostic-only reasons; confirm failures (path: `crates/made-app/tests/degraded.rs`)
- [x] 6.10 Implement optional integration lifecycle (inactive versus evidenced failure), preserving actionable diagnostic reasons and bounded reconnect semantics (path: `crates/made-app/src/integration_status.rs`)
- [x] 6.11 Wire independent shell/explorer startup, safe launch plan, cwd updates and fallback transitions; optional setup failure cannot gate normal shell use or trigger OpenCode launch (path: `crates/made-app/src/workbench.rs`)

## Checkpoint — Phase 6

- [ ] Record red→green real-shell evidence for "Real shell directory change", "Shell hooks preserve user setup", "Unsupported or restricted cwd reporting", "Malformed or unavailable shell directory" and "Active OpenCode overrides shell then returns"; unavailable executables/platforms remain pending, not passed through mocks (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] Verify modified launch, integration-loading/no-session and all degraded-integration scenarios against 6.6/6.8/6.9 and retained config-immutability tests; run terminal PTY/emulation/input regressions (path: `wiki/records/20261026-v1-evidence.md`)

## 7. Revision — Independent workspace runtime and bridge isolation

Depends on Phase 6 except 7.1–7.2. Keep one stable workspace id and complete workbench per tab. Ownership change is app composition, not a rewrite of terminal/explorer domain crates.

- [x] 7.1 Write tests for distinct workspace endpoints/secrets and expected pane-id binding, cross-tab hello rejection before replacement/disclosure, same-tab reconnect and stale epochs; confirm failures [P] (path: `crates/made-bridge/tests/security.rs`)
- [x] 7.2 Bind each server to its expected pane identity and reject cross-workspace authentication/routing while preserving loopback, random port, bounded queues and same-tab replacement (path: `crates/made-bridge/src/server.rs`)
- [x] 7.3 Write tests for open/switch/close/final-close, live commands, cwd/session/preview/layout/focus isolation, hidden-output fairness, late callbacks and no workspace-id reuse; confirm failures (path: `crates/made-app/tests/workspace_tabs.rs`)
- [x] 7.4 Replace single workbench ownership with collection/active id; add folder-to-new-tab and final-close-to-picker flow, bounded fair ticking and shutdown-all on window close (path: `crates/made-app/src/app.rs`)
- [x] 7.5 Implement stable workspace ownership, distinct bridge credentials/pane ids, workspace-scoped root/load delivery, ephemeral layout/left-surface state and tab-only shutdown; invalidate late events before dropping resources (path: `crates/made-app/src/workbench.rs`)
- [x] 7.6 Extend lifecycle tests to show closing one child/process tree and bootstrap cannot terminate siblings through platform cleanup; confirm failures (path: `crates/made-platform/tests/contract.rs`)
- [x] 7.7 Scope child/bootstrap cleanup to the owning process session and retain window-level cleanup for all sessions, avoiding global cleanup during individual tab close (path: `crates/made-platform/src/lib.rs`)
- [x] 7.7a Implement native Windows workspace-owned descendant cleanup required by failing 7.6 tests, with scoped handles and no sibling termination; current immediate-child-only termination is not evidence of full tab cleanup (path: `crates/made-platform/src/windows.rs`)
- [x] 7.8 Extend terminal view tests for visible/focused input ownership, tab/surface focus and IME transitions, hidden output processing without grid paint, and preserved keyboard/mouse/paste behavior; confirm failures (path: `crates/made-app/src/terminal_view.rs`)
- [x] 7.9 Implement visibility-aware polling versus painting and input/focus routing without changing key encodings or relaunching hidden shells (path: `crates/made-app/src/terminal_view.rs`)

## Checkpoint — Phase 7

- [ ] Verify "Open and switch independent workspaces", "Workspace bridge isolation", "Close workspace releases only its resources", "Close final workspace" and "Hidden workspaces remain current and bounded" through model, real process and bridge tests; assert sibling survival and bounded queues/no hidden paint (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] Re-run unauthorized/non-loopback/protocol, config immutability/redaction, session focus, terminal fidelity and workspace picker regressions after ownership changes (path: `wiki/records/20261026-v1-evidence.md`)

## 8. Revision — Diagnostics data (UI tasks moved)

UI-owned tasks 8.1–8.2 and 8.5–8.9 moved to `made-ui-ux-v1` tasks 2.x and 3.x. Core retains the diagnostics data/access slice:

- [x] 8.3 Extend diagnostics tests for tab identity, inactive integration, cwd capability/reason, latest shell cwd, effective root/source, metadata and failures; assert displayed/copied secret redaction; confirm failures (path: `crates/made-app/tests/diagnostics.rs`)
- [x] 8.4 Implement selected-workspace diagnostics data/access while preserving existing versions, metrics, logs and copy actions (path: `crates/made-app/src/diagnostics.rs`)

## Checkpoint — Phase 8

- [ ] Verify "Connected diagnostics", "Degraded diagnostics", "Unsupported integration", "Logs redact token" and "Shell-only diagnostics" with selected-workspace data and redaction; presentation is owned by `made-ui-ux-v1` (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] Verify retained preview/security/terminal input scenarios after diagnostics data changes; no save/write API, terminal Ctrl+S remains XOFF, preview save remains inert (path: `wiki/records/20261026-v1-evidence.md`)

## 9. Revision — End-to-end performance, cross-spec visual evidence and platform gate

Depends on Phases 6–8. Readiness analysis is not delivery verification; keep this phase unchecked until actual evidence exists.

- [x] 9.1 Write performance tests/instrumented checks for real cd follow, at least three workspaces with hidden sustained output, tab activation, preview roundtrip, explorer hide/resize and 10k tree; confirm current missing instrumentation/behavior fails rather than treating model-only timings as GUI performance (path: `crates/made-app/tests/performance.rs`)
- [x] 9.2 Instrument cwd/session follow, activation, preview/layout roundtrip and bounded hidden work/paint counters without per-frame unbounded allocation; retain existing startup/input/frame metrics (path: `crates/made-app/src/metrics.rs`)
- [x] 9.2a Wire metric events and per-workspace visibility/queue counters into runtime transitions under the runtime owner's serialized file ownership; keep instrumentation off the input-critical blocking path (path: `crates/made-app/src/workbench.rs`)
- [x] 9.3 Extend the end-to-end checklist to every revised core scenario and retained UI evidence group, shell capability and supported OS, real process survival, isolation, security/input regressions and Calm Editor visual/focus checks (path: `tests/e2e/ux_validation.md`)
- [x] 9.4 Measure "Complete terminal-first performance evidence" and retained startup/session/input/output/no-webview budgets on documented hardware/backend; record machine, workload/tab count, thresholds, screenshots/frame evidence, software/model versus accelerated GUI results and explicitly pending platforms (path: `wiki/records/20261026-v1-evidence.md`)
- [x] 9.5 Update platform validation procedure for supported shell modes, honest cmd/policy fallback, new workspace lifecycle and preview/layout roundtrip (path: `wiki/testing/platform-matrix.md`)
- [ ] 9.6 Record actual macOS arm64/x64, Linux X11/Wayland and native Windows shell/OpenCode evidence; do not infer runtime passes from CI or software rendering (path: `wiki/records/20261026-v1-evidence.md`)
- [x] 9.7 Replace stale one-terminal/root-fallback/preview and optional-integration limitations with measured capability constraints, without claiming cmd live follow or restart persistence (path: `wiki/testing/platform-matrix.md`, `wiki/records/20261026-v1-evidence.md`)
- [ ] 9.8 Reassess readiness against persisted revised contract and all evidence; prior checked 5.13 is historical, not current approval (path: `wiki/testing/platform-matrix.md`, `wiki/records/20261026-v1-evidence.md`)

## Checkpoint — Phase 9

- [ ] Run `cargo fmt --all --check`, workspace tests, `cargo clippy --workspace --all-targets -- -D warnings`, plugin tests and applicable native GUI/process checks; retain manual-only gaps explicitly (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] Every core scenario in `spec.md` maps to passing evidence, including modified Diagnostics, integration, security and responsiveness requirements; UI scenarios are tracked in `specs/changes/made-ui-ux-v1/tasks.md`. Resolve missed budgets/visual regressions or record them as blocking readiness, not silently waived success (path: `wiki/records/20261026-v1-evidence.md`)

## 10. Revision — Home-default startup and explicit picker

Depends on the retained launch/workspace tests and Phase 6 startup wiring. Phase 10 may proceed in parallel with the UI work in `made-ui-ux-v1`, except shared documentation and any `workbench` wiring; keep `cli.rs`, `workspace.rs` and `platform` writes serialized. Execute each test task first and record a meaningful failing assertion against current behavior before its implementation tasks.

- [x] 10.1 Write failing CLI parsing tests for no-arg home, `--picker`, explicit `--workspace`/positional precedence over `--picker`, duplicate workspace rejection, unknown option, and updated usage; confirm failures [P] (path: `crates/made-app/src/cli.rs`)
- [x] 10.2 Write failing launch/home tests for no-arg home startup, explicit `--picker`, explicit override, no process before picker confirmation, invalid/unreadable home (visible error, usable terminal fallback, no blocking picker), home-derived launch state not misreported as an explicit selection, and retained no-workbench `--smoke-test` behavior when no folder is supplied; confirm failures [P] (path: `crates/made-app/tests/launch_workspace.rs`)
- [x] 10.3 Write failing platform contract tests for cross-platform home resolution (HOME/USERPROFILE, empty/unset, missing/unreadable/non-directory) and verify home handling stays behind the platform boundary; confirm failures [P] (path: `crates/made-platform/tests/contract.rs`)
- [x] 10.4 Implement the bounded platform home capability and shared fallback semantics (path: `crates/made-platform/src/lib.rs`)
- [x] 10.5 Implement `--picker` parsing, explicit-path precedence and updated usage text (path: `crates/made-app/src/cli.rs`)
- [x] 10.6 Implement no-arg home launch, home-derived launch state, and non-blocking invalid-home terminal fallback while preserving explicit/picker and no-OpenCode startup (path: `crates/made-app/src/workspace.rs`)

## Checkpoint — Phase 10

- [ ] Verify "Workspace chosen via picker or argument" and "No workspace provided" with real home resolution and no blocking picker (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] Verify "Picker requested explicitly" and "Explicit workspace overrides picker", including no process before picker confirmation (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] Verify "Invalid workspace folder" and "Default home workspace unavailable"; fallback terminal is usable, visible error is shown, and no picker blocks startup (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] Re-run "Workspace remains usable before OpenCode starts" and retained explicit/picker/invalid workspace regressions; record unverified platforms honestly (path: `wiki/records/20261026-v1-evidence.md`)

## 11. Revision — UI moved to made-ui-ux-v1

Former Phase 11 icon-only header and quiet explorer tasks 11.1–11.8 moved to `made-ui-ux-v1` tasks 1.x and 2.x. Their checked Linux/headless evidence moved with them; native/manual checkpoints remain open there. This core change retains no UI implementation tasks.

## Analyze — Home-default startup and icon-only chrome revision (historical planning analysis)

**Pre-split historical record.** UI requirements and tasks now live in `made-ui-ux-v1`; Phase 11 moved there. The counts and references below describe the pre-split bundle and are retained for history, not as the current ownership map.

**Planning assessment (historical).** At analysis time this was a documentation-only refinement of the existing active, undelivered v1: no new change directory, no archive, no code/evidence claims. Normative guarantees remain in `spec.md`; home resolution, CLI precedence, icon-action mechanics and divider painting are implementation detail in `design.md`/`tasks.md`. The revision adds no AI/chat UI, editing, remote/WSL support, worktree management, automatic OpenCode launch, restart persistence, or other agent integration. Terminal-first behavior, workspace tabs, full-left preview, optional OpenCode, and security/read-only/performance guarantees are retained. The implementation is now present; checked Phase 10–11 items and the Linux/Xvfb evidence in `wiki/records/20261026-v1-evidence.md` record delivery in that scope, while native/cross-platform checkpoints remain open.

All new/modified scenarios and the requirement-wide changes below have task coverage. Every implementation follows its listed failing test; Phase 10–11 implementation/test items are checked with Linux automated/Xvfb evidence, and their manual/native checkpoints remain unchecked.

| New/modified scenario (exact spec title) | Test-first tasks → implementation | Evidence checkpoint |
|---|---|---|
| Workspace chosen via picker or argument | 10.1–10.2 → 10.5–10.6; retained 1.20–1.21 | 10 |
| No workspace provided | 10.1–10.3 → 10.4–10.6 | 10 |
| Picker requested explicitly | 10.1–10.2 → 10.5–10.6; retained 1.20–1.21 and 11.3 for the header Open folder action | 10, 11 |
| Explicit workspace overrides picker | 10.1–10.2 → 10.5–10.6 | 10 |
| Invalid workspace folder | 10.2 → 10.5–10.6; retained 1.18–1.19 | 10 |
| Default home workspace unavailable | 10.2–10.3 → 10.4–10.6 | 10 |
| Quiet shell workspace | 11.1 → 11.2–11.3; retained 8.1, 8.5 | 11 |
| Icon-only header controls | 11.1 → 11.2–11.3 | 11 |
| Explorer hide and restore | 11.4 → 11.6–11.7; retained 8.1, 8.5 | 11 |
| Explorer header action removal | 11.4 → 11.6 | 11 |
| Explorer resize and compact layout | 11.4 → 11.7; retained 8.1, 8.5–8.7 | 11 |
| Hover-only explorer divider | 11.4 → 11.7 | 11 |
| No manual refresh control | 11.5 → 11.6; retained 2.1–2.4, 8.6 | 11 |

Requirement-wide coverage: Workspace Launch 10.1–10.6; Minimal Workspace Layout 11.1–11.8 plus retained 8.1–8.7; Explorer Live Updates 11.4–11.8 plus retained 2.1–2.4/2.15–2.16; Security and Local-Only Integration 10.4–10.6 plus retained 2.5–2.6, 3.5–3.6, 3.19–3.20, 5.3–5.4; Diagnostics retained 5.1–5.4/8.3–8.4; Cross-Platform Delivery 10.3–10.4 plus retained 4.1–4.8; end-to-end checklist/evidence 11.8.

| Analyze gate | Result | Basis |
|---|---|---|
| In-place scope and live-contract consistency | PASS | Existing active bundle refined in place; no new change directory, version, archive, or unrelated feature |
| Outcome versus implementation separation | PASS | Home/picker precedence and observable UI guarantees in `spec.md`; platform home resolution, icon primitive and divider mechanics in `design.md`/`tasks.md` |
| Scenario/test-first coverage | PASS | 13 new/modified scenarios have exact rows above; 17 requirements and 97 scenarios retain or gain task/checkpoint coverage; every implementation follows a failing test |
| Historical completion integrity | PASS | All existing checked task/checkpoint items retained; Phase 10–11 tasks were added unchecked at planning time, then implementation/test items were checked with Linux evidence; checkpoints remain unchecked and no historical checkbox was rewritten |
| Security, read-only and input preservation | PASS | Default home is an owning-tab root grant only; path guard, no-write preview, config immutability, token redaction, terminal input/IME and workspace isolation regressions remain |
| UI consistency and keyboard access | PASS | Icon-only actions carry tooltips/accessible names/focus; hide/restore remains keyboard reachable; pointer-only resize is explicitly not a keyboard stop; no manual Reload remains; no at-rest divider line |
| Dependency/ownership and bounded resources | PASS | Phase 10 CLI/home/platform before/parallel with Phase 11 UI; shared `ui.rs`/`theme.rs`/`explorer_view.rs` writes serialized; no new process or persistence |
| Delivery/verification readiness | PASS — Linux automated/Xvfb scope; native pending | Home/picker, invalid-home fallback, icon header, explorer action removal, hide/restore and hover-only divider are implemented and evidenced on Linux; native Linux/macOS/Windows checkpoints remain open |

No contradictions or unresolved clarification markers remain. The only intentional removal is keyboard width stepping, which is replaced by the explicit pointer-only resize requirement; hidden/restore and all persistent actions remain keyboard reachable. The test-only `--smoke-test` no-workbench path is preserved so automated checks do not open a real home shell. Icon actions set tooltip and accessible-name metadata; platform accesskit exposure remains the existing caveat and is not claimed by this revision. Linux implementation and Xvfb evidence now exist; native/cross-platform delivery remains unverified and full scenario conformance must be rerun before any archive request.

## Analyze — approved revision readiness and scenario traceability (historical, Phases 6–9)

Historical record retained for Phases 6–9; see the current home/icon analysis above for the active revision. **PASS for implementation planning, not delivery.** Propose: existing undelivered v1 refined in place. Clarify: approved precedence/optional OpenCode and explicit shell capability policy are recorded; final-close-to-existing-picker is a bounded lifecycle design decision, not a claim of separately approved user dialogue. Plan: unchecked Phases 6–9 are dependency-ordered and test-first, one path per task. Analyze: normative guarantees remain in `spec.md`; shell/bootstrap, bridge-per-tab and workbench reuse choices stay in design/tasks. No unresolved clarification markers, new version, persistence, splits, editing, remote support, agent integrations or automatic OpenCode launch are introduced. Legacy tasks/checkpoints retain historical truth; old coverage plus new work covers every requirement/scenario. Known Windows immediate-child-only cleanup is explicitly assigned new implementation/test work rather than silently treated as sufficient.

All new/modified scenarios are listed below. Requirement-wide policy changes also require rerunning their unchanged scenarios at the referenced checkpoint; technical status presentation does not remove their behavior. Each implementation follows its listed failing test; Phase 9 verifies actual GUI/platform evidence and no code has been certified by this analysis.

| New/modified scenario (exact spec title) | Test-first tasks → implementation | Evidence checkpoint |
|---|---|---|
| Workspace remains usable before OpenCode starts | 6.8 → 6.11; 8.1 → 8.5 | 6, 8 |
| Plugin loads on first OpenCode launch | 6.9, 8.3 → 6.10–6.11, 8.4–8.5; retained 3.7 | 6, 8 |
| Existing user plugins are preserved | retained 3.5, 3.19; 6.8 → 6.11 | 6 |
| No OpenCode session yet | 6.6, 6.9 → 6.7, 6.10–6.11 | 6 |
| Real shell directory change | 6.1, 6.3a, 6.4, 6.6 → 6.2–6.3, 6.3b, 6.5, 6.7, 6.11 | 6, 9 |
| Shell hooks preserve user setup | 6.1, 6.3a → 6.2–6.3, 6.3b | 6, 9 |
| Unsupported or restricted cwd reporting | 6.1, 8.3 → 6.2–6.3, 8.4 | 6, 8 |
| Malformed or unavailable shell directory | 6.4, 6.6 → 6.5, 6.7, 6.11 | 6 |
| Active OpenCode overrides shell then returns | 6.6 → 6.7, 6.11 | 6, 9 |
| Open and switch independent workspaces | 7.3, 7.8, 8.1 → 7.4–7.5, 7.9, 8.5 | 7, 8, 9 |
| Workspace bridge isolation | 7.1, 7.3 → 7.2, 7.4–7.5 | 7 |
| Close workspace releases only its resources | 7.3, 7.6 → 7.4–7.5, 7.7–7.7a | 7 |
| Close final workspace | 7.3 → 7.4–7.5 | 7 |
| Hidden workspaces remain current and bounded | 7.3, 7.8, 9.1 → 7.4–7.5, 7.9, 9.2 | 7, 9 |
| Quiet shell workspace | 8.1 → 8.5–8.7 | 8 |
| Explorer hide and restore | 8.1 → 8.5–8.7 | 8, 9 |
| Explorer resize and compact layout | 8.1, 9.1 → 8.5–8.7, 9.2 | 8, 9 |
| OpenCode-created worktree becomes active | 8.8, 8.3 → 8.4–8.5, 8.9; retained 3.9, 3.15 | 8 |
| Worktree metadata unavailable | 8.8, 8.3 → 8.4–8.5, 8.9; retained 3.15 | 8 |
| Text file preview | 8.2 → 8.5–8.6 | 8 |
| Preview terminal roundtrip | 8.2, 7.8 → 8.5–8.6, 7.9 | 8, 9 |
| Root change clears preview | 8.2, 6.6 → 6.7, 7.5, 8.6 | 8 |
| Plugin unavailable | 6.9 → 6.10–6.11 | 6 |
| OpenCode missing or older than v2 | 6.8–6.9 → 6.10–6.11 | 6 |
| OpenCode started with extension loading disabled | 6.9 → 6.10–6.11 | 6 |
| Bridge disconnects mid-session | 6.6, 6.9 → 6.7, 6.10–6.11 | 6 |
| Incompatible protocol version | 6.9 → 6.10–6.11 | 6 |
| OpenCode exits normally | 6.6, 6.9 → 6.7, 6.10–6.11 | 6 |
| Reconnect | 6.9, 7.1 → 6.10–6.11, 7.2, 7.5 | 6, 7 |
| Session tabs unavailable | 6.9 → 6.10–6.11 | 6 |
| Configuration composition fails | 6.8–6.9 → 6.10–6.11 | 6 |
| User config unchanged | 6.1; retained 3.19 → 6.2–6.3, 6.11 | 6, 7 |
| Complete terminal-first performance evidence | 9.1 → 9.2, 9.2a, 9.3–9.6 | 9 |
| Degraded diagnostics | 8.3 → 8.4–8.5 | 8 |
| Shell follow diagnostics without chrome noise | 8.3, 8.1 → 8.4–8.5 | 8 |

Additional requirement-wide coverage: Active Session Follow 6.6–6.7; optional Session Map 8.1/8.8–8.9 plus retained Phase 3 focus tests; Security and Local-Only Integration 6.1/6.4, 7.1–7.2 and retained path/immutability/redaction tests; Minimal Workspace Layout 8.1–8.7; Responsiveness and Lightness 7.8–7.9/9.1–9.4; Diagnostics 8.3–8.4. Unchanged scenario checkpoints in Phases 0–5 remain required under the new lifecycle.

| Analyze gate | Result | Basis |
|---|---|---|
| In-place scope and live-contract consistency | PASS | Existing active bundle only; stale single-workbench, last-agent-root, stacked-preview and default-degraded decisions corrected across proposal/spec/design/UI contract |
| Outcome versus implementation separation | PASS | Observable guarantees/scenarios in spec; shell-hook candidates, per-tab bridges and file ownership in design/tasks |
| Scenario/test-first coverage | PASS | 17 requirements and 90 unique scenarios at the Phase 6–9 baseline had task/checkpoint coverage; all 35 then-new/modified scenario titles had exact rows above; implementation followed failing assertions |
| Historical completion integrity | PASS | 84 checked historical tasks retained; all 49 revision task/checkpoint items unchecked; prior readiness documents explicitly require reassessment |
| Security, read-only and input preservation | PASS | Per-tab authentication/routing and process cleanup assigned tests; config/path/redaction/no-write/keyboard/IME regressions remain mandatory |
| Dependency/ownership and bounded resources | PASS | Launch/receiver → root policy → workspace runtime → visible layout → measured evidence; only isolated bridge work parallelizes across those phases |
| Delivery/verification readiness | FAIL — not delivered | No revised product behavior, real-shell matrix, native Windows descendant cleanup or hardware GUI performance is certified by this documentation-only revision; Phases 6–9 must supply evidence |

Readiness analysis found and corrected missing requirement-summary coverage for the three added requirement groups, explicitly assigned the existing Windows descendant-cleanup gap, and removed stale UI/design root/status/layout contradictions. There are no unresolved clarification markers. Missing platform executables/displays/hardware are execution evidence risks, not permission to claim a pass or to expand scope. Full scenario conformance must be rerun after implementation before any archive request.

## Analyze — UI/core contract split (current)

**Readiness assessment:** PASS for the core/UI contract split and task ownership; delivery remains Linux-only with native/cross-platform evidence pending.

| Analyze gate | Result | Basis |
|---|---|---|
| In-place scope and live-contract consistency | PASS | Both changes remain active; no version, archive, or new product scope. Core retains product/runtime/integration/security/performance; UI owns visual/presentation |
| Outcome versus implementation separation | PASS | Core spec keeps observable behavior/data/security outcomes; UI spec keeps observable presentation/interaction outcomes; expanded visual rationale is non-normative in `specs/changes/made-ui-ux-v1/design.md` |
| Scenario traceability and no loss | PASS | 97 original scenarios accounted for: 85 retained in core, 12 moved to `made-ui-ux-v1`; 2 derived core scenarios and 21 derived UI scenarios cover split-out behavior/presentation without dropping a title |
| Unresolved clarification markers | PASS | No unresolved clarification markers, TODO, or unresolved placeholder in either active change |
| Duplicated normative contract | PASS | Minimal Workspace Layout and UI-only scenarios removed from core; core requirements cross-reference the UI spec; core safety/read-only/security guarantees remain only in core |
| Dependency/task ownership | PASS | UI files/tasks moved to `made-ui-ux-v1`; core retains diagnostics data (8.3–8.4), runtime, integration, security and preview safety. Shared `ui.rs`/`theme.rs`/`explorer_view.rs` writes are serialized under the UI change |
| Historical completion integrity | PASS | Moved checked items retain their original checked state and Linux evidence; no historical checkbox was rewritten; native/manual checkpoints remain open |
| Honest delivery status | PASS — NOT DELIVERED | 602 tests, Linux/Xvfb/headless evidence and preserved captures prove the Linux UI scope; native macOS/Windows, native Linux X11/Wayland, native backdrop/compositor, native pointer/keyboard and hardware-GPU evidence remain pending |
| Cross-spec coherence | PASS | Core proposal/spec/design/tasks link to `made-ui-ux-v1`; UI design links back; architecture/UI reference updated where the split requires it |

The split adds no runtime code, no new feature, no archive, and no delivery claim. Every moved UI task is now owned by `made-ui-ux-v1`; core checkpoints for moved scenarios explicitly rely on that change. Native/cross-platform conformance must be rerun for both changes before any archive request.

## Implementation handoff and ownership

Spec ref: `specs/changes/opencode-session-follow-v1/`

UI/UX presentation ownership: `specs/changes/made-ui-ux-v1/`. UI files, tasks and native UI evidence are owned there; this table covers the core runtime/integration/safety scope only.

| Bounded implementation scope | Owned files | Dependencies |
|---|---|---|
| Shell capabilities/bootstrap and cwd receiver | `crates/made-platform/src/shell_integration.rs` (new), `crates/made-platform/src/lib.rs`, `crates/made-platform/tests/shell_integration.rs` (new), `crates/made-terminal/src/pty.rs`, `crates/made-terminal/tests/pty.rs`, `crates/made-terminal/src/emulation.rs`, `crates/made-terminal/tests/emulation.rs` | Phase 6; audit existing shell launch options rather than altering global config |
| CLI, home startup and invalid-home fallback | `crates/made-app/src/cli.rs`, `crates/made-app/src/workspace.rs`, `crates/made-platform/src/lib.rs`, `crates/made-app/tests/launch_workspace.rs`, `crates/made-platform/tests/contract.rs` | Phase 10; coordinate `platform/lib.rs` and contract tests with shell/workspace owners and `launch_workspace.rs` with the root-arbitration owner; no new workspace model |
| Root arbitration and optional lifecycle | `crates/made-app/src/session_follow.rs`, `crates/made-app/src/integration_status.rs`, `crates/made-app/tests/session_follow.rs`, `crates/made-app/tests/launch_workspace.rs`, `crates/made-app/tests/degraded.rs` | Receiver/capability contract; workbench wiring serialized with runtime owner; coordinate `launch_workspace.rs` writes with Phase 10 |
| Isolated bridge | `crates/made-bridge/src/server.rs`, `crates/made-bridge/tests/security.rs` | 7.1–7.2 can run alongside Phase 6; no shared-listener multiplexing |
| Workspace runtime/lifecycle | `crates/made-app/src/app.rs`, `crates/made-app/src/workbench.rs`, `crates/made-app/src/terminal_view.rs`, `crates/made-app/tests/workspace_tabs.rs` (new), `crates/made-platform/tests/contract.rs`, `crates/made-platform/src/windows.rs` | Root/launch and bridge contracts; coordinate writes to platform lib with bootstrap owner; own all workbench writes (6.11, 7.5, 9.2a). Native Windows descendant cleanup is a delivery gate, not an inherited guarantee |
| Preview safety, session focus and diagnostics data | `crates/made-explorer/src/paths.rs`, `preview.rs`, `preview_refresh.rs`, `root_switch.rs`, `crates/made-app/src/session_focus.rs`, `crates/made-app/src/diagnostics.rs`, `crates/made-app/tests/session_map.rs`, `crates/made-app/tests/read_only_ui.rs`, `crates/made-app/tests/diagnostics.rs` | Runtime exposes stable active workspace/left-surface/focus state; UI layout/presentation is owned by `made-ui-ux-v1`; `session_map.rs` implementation and `read_only_ui.rs` presentation are shared with UI tasks, so writes are serialized |
| Evidence/performance/readiness | `crates/made-app/src/metrics.rs`, `crates/made-app/tests/performance.rs`, `tests/e2e/ux_validation.md`, `wiki/records/20261026-v1-evidence.md`, `wiki/testing/platform-matrix.md` | Phases 6–10, core scenarios and available native shells/displays/hardware; UI evidence is tracked in `specs/changes/made-ui-ux-v1/tasks.md`; report unavailable evidence as pending |

Before any new module is introduced, update its crate export in that crate's `src/lib.rs` only as needed; include this mechanical integration in the owning implementation scope. Existing terminal input, preview loaders/path guards, plugin APIs, runtime ownership and platform behavior are regression boundaries, not permission for refactoring. UI-owned `ui.rs`, `theme.rs`, `explorer_view.rs`, `session_map.rs` and picker presentation must be changed under `made-ui-ux-v1` ownership; coordinate shared-file writes serially. If implementation requires another source-file touch, justify it to the coordinator before expanding ownership. Do not mark revision work complete based on the preserved historical checkboxes.
