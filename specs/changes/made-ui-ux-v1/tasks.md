# Tasks: made-ui-ux-v1

> **Checkbox scope:** checked items reflect implementation and automated Linux/headless/Xvfb evidence in this tree, not native platform or manual visual validation. Moved task IDs retain their original core numbering in parentheses for traceability. Native macOS/Windows, native Linux X11/Wayland, native pointer/keyboard, native compositor/backdrop, accessibility-tree, and hardware-GPU checkpoints remain unchecked. See `wiki/records/20261026-v1-evidence.md`.

> **Contract split:** this change owns UI/UX/visual tasks only. Product/runtime/integration/security/performance tasks remain in `specs/changes/opencode-session-follow-v1/tasks.md`. No code behavior is changed by this split; no archive or version bump.

## 1. Visual system, brand, header and picker

- [x] 1.1 (moved core 8.7) Adjust layout bounds only as required for usable full-height explorer/left surfaces at both sizes; preserve Calm Editor tokens, contrast, opaque terminal/preview and focus styling (path: `crates/made-app/src/theme.rs`)
- [x] 1.2 (moved core 11.2) Add/rework the icon-action primitive with monochrome glyph, 24×24 minimum target, tooltip/accessibility metadata hook and focus styling (path: `crates/made-app/src/theme.rs`)
- [x] 1.3 (moved core 11.1) Extend UI tests for icon-only header actions (matching tooltip/accessibility name, visible focus, no text labels, workspace tab close affordance, 1280×800/720×480 reachability); replace existing text-label assertions without weakening control-reachability coverage; confirm failures (path: `crates/made-app/tests/ui_model.rs`)
- [x] 1.4 (moved core 11.3) Implement icon-only Open folder, Explorer toggle, Diagnostics and optional Sessions actions; remove header text action labels; preserve focus order and workspace-tab close affordances (path: `crates/made-app/src/ui.rs`)
- [x] 1.5 (moved core 1.20) Write a failing folder-picker test for no-workspace launch and picker selection (path: `crates/made-app/tests/workspace_picker.rs`)
- [x] 1.6 (moved core 1.21) Implement the in-app folder picker and selected-workspace open (path: `crates/made-app/src/workspace_picker.rs`)
- [x] 1.7 Verify Calm Editor glass alphas, clear-color/native fallback, two-tone wordmark, tab/no-dot treatment, compact overflow and picker selection/Close/Escape polish through headless tests and Xvfb captures (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] 1.8 Record native macOS HudWindow vibrancy, Windows 11 Mica, compositor alpha/glass and native contrast at both window sizes; keep compile-only checks marked as non-evidence (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] 1.9 Record native header/tab/picker keyboard, tooltip, pointer and focus behavior on macOS, Linux X11/Wayland and Windows (path: `wiki/records/20261026-v1-evidence.md`)

## Checkpoint — Phase 1

- [ ] Verify "Calm Editor tokens and typography", "Glass chrome and opaque reading surfaces", "Native backdrop auto and fallback", "Reading-surface contrast"
- [ ] Verify "MADE wordmark identity", "Workspace tab presentation", "Workspace tab overflow and close"
- [ ] Verify "Picker selection and close", "Picker keyboard navigation", "Picker preserves live workspace"
- [x] Headless/Xvfb checks cover the implemented Linux UI; native backdrop/keyboard/pointer rows remain pending in `wiki/records/20261026-v1-evidence.md`

## 2. Layout, explorer and preview presentation

- [x] 2.1 (moved core 8.1) Extend UI tests for quiet default shell, separate workspace/session tabs, initially hidden session map, full-height tree, explorer hide/restore/resize, compact layout, keyboard controls and per-tab widget ids; confirm failures (path: `crates/made-app/tests/ui_model.rs`)
- [x] 2.2 (moved core 8.2) Extend preview UI tests for full-left file-open, no default placeholder, both close/return paths to live terminal, output continuity, input isolation, root-change clearing and stale reads, while retaining every binary/size/encoding/no-write guard; confirm failures (path: `crates/made-app/tests/read_only_ui.rs`)
- [x] 2.3 (moved core 8.5) Replace default status/banner/empty-session chrome with workspace tab controls and discoverable Explorer/Diagnostics/optional Sessions controls; implement new-folder, switch and close actions, full-height hideable/resizable right tree and mutually exclusive full-left terminal/preview, workspace-scoped ids and explicit focus transfer (path: `crates/made-app/src/ui.rs`)
- [x] 2.4 (moved core 8.6) Separate tree and preview rendering, remove embedded/right-column preview and watcher status chrome, reconcile hidden-tree changes and reject stale root/preview deliveries; retain all guarded preview states and actions (path: `crates/made-app/src/explorer_view.rs`)
- [x] 2.5 (moved core 11.4) Extend UI tests for explorer header action removal, hide/restore through the header icon, hover-only divider with no at-rest line/grip, pointer-drag width, no keyboard width stop, per-workspace width retention, and no Reload action; replace the existing visible-at-rest splitter assertion without weakening drag coverage; confirm failures (path: `crates/made-app/tests/ui_model.rs`)
- [x] 2.6 (moved core 11.5) Extend live-update regression to prove create/delete/rename still update within 1 second with no manual Reload control present; confirm failures (path: `crates/made-app/tests/ui_model.rs`)
- [x] 2.7 (moved core 11.6) Remove explorer header Wider/Narrower/Reload/Hide actions while retaining root identity and contextual tree errors [P] (path: `crates/made-app/src/explorer_view.rs`)
- [x] 2.8 (moved core 11.7) Implement header Explorer toggle hide/restore wiring and pointer-drag-only divider geometry/painting (transparent at rest, hover/drag line/grip, no keyboard stop) while retaining stored per-workspace width and usable left pane (path: `crates/made-app/src/ui.rs`)
- [x] 2.9 (moved core 11.8) Extend the end-to-end UX validation checklist with the moved UI scenarios, including wordmark, glass, picker, icon tooltip/focus, explorer action removal, hide/restore via header, hover-only divider, no manual Reload, responsive and motion checks; confirm each has an evidence row (path: `tests/e2e/ux_validation.md`)
- [x] 2.10 (moved core 2.12) Implement the explorer tree and preview panels with keyboard navigation (path: `crates/made-app/src/explorer_view.rs`)
- [ ] 2.11 Record native pointer drag/hover, keyboard focus, explorer animation smoothness, and 1280×800/720×480 visual review on native desktops (path: `wiki/records/20261026-v1-evidence.md`)
- [x] 2.12 (moved core 2.13) Write a failing syntax-highlighting fallback test (path: `crates/made-explorer/tests/highlight.rs`)
- [x] 2.13 (moved core 2.14) Implement optional syntax highlighting with plain-text fallback (path: `crates/made-explorer/src/highlight.rs`)

## Checkpoint — Phase 2

- [ ] Verify "Quiet shell workspace", "Icon-only header controls", "Explorer hide and restore", "Explorer resize and compact layout", "Explorer header action removal", "Hover-only explorer divider", "No manual refresh control", "Quiet integration chrome"
- [ ] Verify "Text file preview", "Preview terminal roundtrip", "Syntax highlighting"
- [ ] Verify "Responsive breakpoints preserve panes and controls", "Explorer hide-show animation", "Motion does not carry essential information"
- [x] Headless/Xvfb evidence covers the implemented Linux layout, animation, preview roundtrip and compact captures; native pointer/keyboard/animation rows remain pending in `wiki/records/20261026-v1-evidence.md`

## 3. Session map and diagnostics presentation

- [x] 3.1 (moved core 8.8) Extend optional session-map tests for selected-workspace ownership and no empty default strip; verify same/different worktree, metadata-unavailable and all focus outcomes remain available without default chrome; confirm failures before any necessary map changes (path: `crates/made-app/tests/session_map.rs`)
- [x] 3.2 (moved core 8.9) Make only optional-map ownership/presentation changes required by 3.1; retain every root session and supported activity indicator (path: `crates/made-app/src/session_map.rs`)
- [x] 3.3 Confirm the diagnostics floating card uses the documented opaque presentation, sectioned values, bounded log, copy actions and responsive clamps; cross-check the data/redaction coverage retained in core tasks 5.1–5.4 and 8.3–8.4 (path: `crates/made-app/tests/ui_model.rs`)
- [ ] 3.4 Record native diagnostics card focus, copy actions, contrast, resize and 1280×800/720×480 presentation (path: `wiki/records/20261026-v1-evidence.md`)
- [x] 3.5 (moved core 3.12) Implement the session map UI (path: `crates/made-app/src/session_map.rs`)

## Checkpoint — Phase 3

- [ ] Verify "Session map hidden by default", "Session activity indicators", "Session map overflow and long labels"
- [ ] Verify "Shell follow diagnostics without chrome noise", "Diagnostics panel presentation"
- [x] Headless/Xvfb evidence covers the implemented Linux map/diagnostics presentation; native session-map focus and diagnostics panel rows remain pending in `wiki/records/20261026-v1-evidence.md`

## 4. Responsive, motion and accessibility

- [x] 4.1 Extend/add headless UI tests for the 900–1099 and <900 breakpoints, compact labeled overflow, no clipped controls, explorer animation intermediate/reversal/closed-input behavior, focus rings, icon accessible names and non-color status cues (path: `crates/made-app/tests/ui_model.rs`)
- [x] 4.2 Implement/adjust focus rings, minimum target sizes, accessible metadata, responsive spacing/overflow and the documented motion while retaining the existing widget ids and terminal input invariants (path: `crates/made-app/src/theme.rs`)
- [ ] 4.3 Record a native keyboard-only walkthrough at 1280×800 and 720×480, including header actions, tabs, tree, preview return, picker and diagnostics, plus the documented contrast/colorblind spot-check (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] 4.4 Record native hover/drag pointer behavior, visible focus, tooltip/accessible-name metadata and (if enabled) platform accessibility-tree behavior; do not claim screen-reader exposure while accesskit is absent (path: `wiki/records/20261026-v1-evidence.md`)

## Checkpoint — Phase 4

- [ ] Verify "Visible focus on persistent controls", "Icon actions expose accessible names", "Keyboard workspace and session navigation", "Status cues do not rely on color"
- [x] Headless/Xvfb checks cover focus, targets, overflow and non-color cues on Linux; native keyboard/pointer/screen-reader rows remain pending in `wiki/records/20261026-v1-evidence.md`

## 5. Evidence and readiness

- [x] 5.1 Record Linux/Xvfb/headless UI evidence for 1280×800/720×480, glass/backdrop fallback, wordmark/tabs, picker, explorer animation/divider, preview and diagnostics; distinguish software from native/hardware evidence (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] 5.2 Record native macOS arm64/x64, Linux X11/Wayland and Windows x64 UI evidence, including compositor/backdrop, pointer/keyboard, animation and hardware-GPU visual checks (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] 5.3 Reassess UI-contract delivery against `spec.md` after native evidence; keep unverified platforms/scenarios open rather than inferring a pass (path: `wiki/testing/platform-matrix.md`, `wiki/records/20261026-v1-evidence.md`)

## Coverage summary

| Requirement | Task coverage | Checkpoint |
|---|---|---|
| Calm Editor Visual System | 1.1–1.2, 1.7–1.8, 4.3; retained theme tests | Phase 1, Phase 4 |
| Minimal Workspace Layout | 2.1, 2.3–2.11; 1.3–1.4; 4.1–4.2 | Phase 1, Phase 2, Phase 4 |
| Brand and Workspace Tabs | 1.4, 1.7, 2.3; retained tab tests | Phase 1, Phase 2 |
| Optional Session Map Presentation | 3.1–3.2, 3.5; core data/focus tasks 3.11, 3.13–3.14 | Phase 3 |
| Explicit Folder Picker | 1.5–1.7; core launch tasks 10.1–10.6 | Phase 1 |
| Full-Left Preview Presentation | 2.2–2.4, 2.10, 2.12–2.13; core preview/safety tasks 2.7–2.11, 2.15–2.16 | Phase 2 |
| Diagnostics Presentation | 3.3–3.4; core data/redaction tasks 5.1–5.4, 8.3–8.4 | Phase 3 |
| Responsive and Motion | 2.5, 2.8, 4.1–4.2 | Phase 2, Phase 4 |
| Keyboard Focus and Accessibility | 1.3, 4.1–4.4 | Phase 1, Phase 4 |

## Scenario disposition from the core change

Every UI scenario removed from the core spec is retained here under the same title unless noted.

| Core requirement/scenario | Disposition |
|---|---|
| Minimal Workspace Layout — Quiet shell workspace | Moved to Minimal Workspace Layout |
| Minimal Workspace Layout — Icon-only header controls | Moved to Minimal Workspace Layout |
| Minimal Workspace Layout — Explorer hide and restore | Moved to Minimal Workspace Layout |
| Minimal Workspace Layout — Explorer resize and compact layout | Moved to Minimal Workspace Layout |
| Minimal Workspace Layout — Explorer header action removal | Moved to Minimal Workspace Layout |
| Minimal Workspace Layout — Hover-only explorer divider | Moved to Minimal Workspace Layout |
| Session Map — Session activity indicators | Moved to Optional Session Map Presentation |
| Read-Only File Preview — Text file preview | Moved to Full-Left Preview Presentation |
| Read-Only File Preview — Syntax highlighting | Moved to Full-Left Preview Presentation |
| Read-Only File Preview — Preview terminal roundtrip | Moved to Full-Left Preview Presentation; the hidden-input/process-survival clause is retained in core as "Preview keeps terminal process and input isolated" |
| Explorer Live Updates — No manual refresh control | Moved to Minimal Workspace Layout |
| Diagnostics — Shell follow diagnostics without chrome noise | Moved to Diagnostics Presentation |
| Read-Only File Preview — (no exact predecessor) | Core retains safety/process scenario "Preview keeps terminal process and input isolated" |
| Diagnostics — (no exact predecessor) | Core retains data scenario "Shell-only diagnostics" |

Derived UI scenarios make previously implicit presentation guarantees explicit: Calm Editor tokens/contrast/glass/backdrop, wordmark/tab/overflow, session-map hidden/overflow, picker selection/keyboard/preserve-live, diagnostics card layout, responsive breakpoints, motion, focus/accessibility, and quiet integration chrome.

## Analyze — UI/core split (current)

**Readiness assessment:** PASS for the split contract; delivery remains Linux-only with native/manual UI evidence pending.

| Analyze gate | Result | Basis |
|---|---|---|
| Scope and live-contract consistency | PASS | Both changes remain active; core retains product/runtime/integration/security/performance; UI owns visual/presentation; no version/archive |
| Outcome versus implementation separation | PASS | Normative UI outcomes in `spec.md`; tokens/layout/components/motion/evidence rationale in non-normative `design.md` |
| Scenario traceability and no loss | PASS | 97 original scenarios accounted for: 85 retained in core, 12 moved here, plus 2 derived core and 21 derived UI scenarios; disposition table above |
| Unresolved clarification markers | PASS | No unresolved clarification markers, TODO, or unresolved placeholder in proposal/spec/tasks/design |
| Duplicated normative contract | PASS | UI layout/chrome/presentation removed from core requirements and replaced with cross-references; core safety/data/security guarantees retained once |
| Dependency/task ownership | PASS | UI tasks own `ui.rs`, `theme.rs`, `explorer_view.rs`, `session_map.rs` presentation, picker UI and UI evidence; core tasks own runtime/integration/security/preview safety/diagnostics data |
| Historical completion integrity | PASS | Moved checked items retain their original checked state and Linux evidence; no historical item was rewritten as checked without evidence |
| Honest delivery status | PASS — Linux UI evidence; native pending | 602 tests, headless/Xvfb captures and evidence rows cover Linux UI; native backdrop/compositor, native pointer/keyboard, native animation and hardware-GPU remain unchecked |
| Cross-spec coherence | PASS | Core spec/tasks/design link to this change; this design links back; wiki/architecture.md and this change's design updated where required |

Notes: The split intentionally adds derived UI scenarios for presentation behavior already implemented/discussed; it adds no new product scope. `accesskit` remains disabled, so platform screen-reader exposure is not claimed. Native UI evidence must be recorded before any archive request for either change.

## Implementation handoff and ownership

Spec ref: `specs/changes/made-ui-ux-v1/`

| Bounded implementation scope | Owned files | Dependencies |
|---|---|---|
| Theme, tokens, glass, native backdrop fallback, focus/accessible primitives | `crates/made-app/src/theme.rs`, `crates/made-platform/src/native_backdrop.rs`, `crates/made-app/tests/ui_model.rs` | Core runtime/workspace state; no OS branch outside `made-platform` |
| Header, wordmark, workspace tabs/overflow, picker composition and layout/motion | `crates/made-app/src/ui.rs`, `crates/made-app/src/workspace_picker.rs`, `crates/made-app/tests/ui_model.rs`, `crates/made-app/tests/workspace_picker.rs` | Core launch semantics and workspace ownership; serialized `ui.rs`/`theme.rs` writes |
| Explorer presentation, full-left preview presentation | `crates/made-app/src/explorer_view.rs`, `crates/made-explorer/src/highlight.rs`, `crates/made-explorer/tests/highlight.rs`, `crates/made-app/tests/read_only_ui.rs`, `crates/made-app/tests/ui_model.rs` | Core explorer/preview safety, root generation, and terminal visibility state; `read_only_ui.rs` is shared with core task 2.11, so writes are serialized |
| Session map and diagnostics presentation | `crates/made-app/src/session_map.rs`, `crates/made-app/src/diagnostics.rs`, `crates/made-app/tests/session_map.rs`, `crates/made-app/tests/ui_model.rs` | Core session-map data/focus and diagnostics data/redaction; `session_map.rs` tests are shared with core task 3.11, so writes are serialized |
| Evidence and platform UI validation | `tests/e2e/ux_validation.md`, `wiki/records/20261026-v1-evidence.md`, `wiki/testing/platform-matrix.md` | Phases 1–4 and available native platforms; report unavailable evidence as pending |

Do not change terminal input/IME/selection behavior, preview write guards, root arbitration, bridge/security, or runtime ownership as part of UI work. If a UI change requires a runtime contract change, return to the core change first. Do not mark native/manual checkpoints complete from Xvfb/headless or compile-only evidence.
