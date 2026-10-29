# Proposal: Made UI/UX v1

## Intent

Made's active, undelivered v1 bundle currently mixes product/runtime/integration guarantees with a large Calm Editor visual contract. That makes the core change hard to review, duplicates UI detail across spec, design, tasks, and a separate legacy UI reference document, and obscures which evidence proves behavior versus presentation. This change separates the UI/UX/visual contract into a dedicated active change while keeping the core v1 change authoritative for runtime, integration, security, and performance behavior. It is a contract split, not a new version, archive, or product scope expansion.

## Scope

In scope:

- Calm Editor visual system: tokens, typography, floating header, explorer glass, native backdrop `auto`/`off` fallback, opaque reading surfaces, contrast, status cues, and the neutral terminal cursor with reported-style blink.
- Minimal workspace layout: quiet terminal-first chrome, icon-only header actions, full-height hideable/resizable explorer, hover-only divider, and no routine manual refresh or technical chrome.
- Brand and workspace tabs: two-tone `MADE` wordmark, editor-like workspace tabs, close affordances, active treatment, overflow, and distinction from OpenCode session tabs.
- Optional session-map presentation: initially hidden, selected-workspace scoped, activity indicators, empty/overflow behavior, and focus-error presentation.
- Explicit folder picker UX: card layout, Close/Escape/Back, inline expandable folder tree, single selected-row/path target, tree keyboard navigation, refresh/re-root state, and preservation of a live workspace when adding another folder.
- Full-left preview presentation: full-left surface, header/actions/return, meta/gutter/code styling, syntax highlighting, and safe-state styling.
- Diagnostics presentation: opaque floating card, sections, metrics/log block, previous-log path/copy row, retention notices present while the card renders, copy actions, responsive sizing.
- Responsive layout and motion: 1280×800/720×480 breakpoints, labeled overflow, explorer hide/show transition, and motion that carries no essential information.
- Keyboard focus and accessibility presentation: visible focus, accessible names for icon actions, workspace/session navigation, target sizes, non-color status cues, and the screen-reader caveat.
- Moving the UI-owned task/evidence checkpoints from the core change while preserving their truthful Linux/Xvfb/headless status and leaving native/manual checkpoints open.

## Non-goals

- Any new product feature or redesign beyond the documented Calm Editor refinements in tasks 6.x–17.x.
- Product/runtime/integration/security/performance ownership: terminal fidelity, session follow, bridge, config immutability, path guards, preview read-only/safety, watcher behavior, home resolution, workspace isolation, diagnostics data, and performance budgets remain in the core change.
- File editing, saving, LSP, diff, AI/chat UI, other agents, remote/WSL, worktree lifecycle management, terminal splits, multi-window, restart persistence, installers, signing, or auto-update.
- Archiving, versioning, or declaring v1 delivered. Full native UX/native contrast certification, Windows and macOS x64 rendering, native Linux X11/Wayland, native pointer/keyboard, and hardware-GPU evidence remain pending.
- Editing code or wiki as part of the original split. This proposal records that split; the later 6.x–17.x refinement tasks in the same active change do update UI code, tests, and the UI contract.
- Rewriting README or the broader documentation set; only cross-spec references are updated.

## Approach

Extract the durable visual contract into this change's `spec.md` (normative UI outcomes) and `design.md` (non-normative tokens, layout, components, motion, accessibility, and evidence rationale). The core change keeps its requirements but removes duplicated UI scenarios, replacing them with explicit cross-references. Moved scenarios retain their exact titles where possible; derived scenarios cover presentation surfaces that were previously implicit in a single mixed requirement. UI tasks carry the existing checked UI implementation/evidence items and add explicit unchecked native/manual checkpoints. The original split made no code changes; the later 6.x–17.x refinement tasks in this active change implement and test the documented UI adjustments.

## Decision index

- **Separate UI change, not a new version:** keeps the active v1 contract reviewable while preserving the undelivered v1 scope; no archive or version bump (design: U1, Migration and rollback).
- **Core keeps behavior, UI owns presentation:** clears duplicated guarantees and gives each contract a single owner (design: U1).
- **Retain the implemented Calm Editor theme:** visual polish is a v1 acceptance criterion, not post-delivery work (design: U2).
- **In-app glass is the portable fallback; native backdrop is optional:** no unverified live-blur claim; `MADE_NATIVE_BACKDROP=auto|off` and honest fallback (design: U3).
- **Soften the glass within the layered hierarchy:** the approved subtle increase sets fallback `BG_CLEAR` 215/255, native `BG_CLEAR_NATIVE` 135/255, fallback explorer card `BG_CARD_GLASS` 188/255, and fallback explorer header `BG_HEADER_GLASS` 194/255 (from 225/150/202/208); the 10–15/255 steps keep the fallback explorer surfaces close to their previous density, preserve the fallback terminal-glass < card < header < fallback-clear order, and leave preview, picker, diagnostics, tooltip, and toast opaque, so more backdrop shows through without claiming certified glass text contrast (design: U3, Transparency and native backdrop policy).
- **Terminal glass follows the terminal mode:** primary-screen terminal surfaces use `BG_TERMINAL_GLASS` 160/255 over the `BG_TERMINAL` RGB (11/13/16), while alternate-screen TUIs (OpenCode/vim/less) switch the card/chrome and body to opaque `BG_TERMINAL` 255/255; this keeps shell prompts lightly transparent without compositing TUI cells over a backdrop, and terminal-glass text contrast is not separately certified (design: U3, Terminal card, Transparency and native backdrop policy).
- **Conditional Explorer tint follows the actual native backdrop:** Explorer card/header use `BG_CARD_GLASS_NATIVE` 24/255 and `BG_HEADER_GLASS_NATIVE` 32/255 only when the recorded native status reports applied, so the macOS HudWindow or Windows Mica material reads through; otherwise the darker `BG_CARD_GLASS` 188/255 and `BG_HEADER_GLASS` 194/255 keep Explorer text legible without blur. Windows uses the same applied/fallback selection, the fallback legibility check is required over a bright composited desktop, and native Explorer tint contrast is not separately certified (design: U3, Explorer card, Transparency and native backdrop policy).
- **Terminal cursor stays neutral and blink follows the terminal:** use `TERMINAL_FOREGROUND #D7DDE7` instead of the mint accent; blink at ~530 ms only when the terminal reports a blinking style and the pane is focused, keep steady/unfocused cursors visible, reset the on phase when input is sent, and repaint only at the next phase boundary rather than adding a 16 ms idle loop (design: U2, Terminal card).
- **Quiet chrome with contextual failures:** technical detail belongs in Diagnostics; normal shell use is not degraded (design: U4).
- **Full-height explorer, full-left preview:** one left surface and one right explorer; preview never shares the explorer column (design: U5).
- **Keyboard focus and non-color status are acceptance criteria:** icon actions expose tooltip/accessibility names, persistent controls are keyboard reachable, and platform screen-reader exposure is not claimed (design: U6).
- **Preserve historical evidence honestly:** moved checked tasks keep Linux evidence; native/manual checkpoints remain unchecked (design: Risks and open evidence, Migration and rollback).

## Clarifications

- **Q:** Does this split create a new v1 version or archive the current change?
  **A:** No. Both changes remain active; `opencode-session-follow-v1` remains the core undelivered v1, and `made-ui-ux-v1` owns its UI/UX contract. Nothing is archived.
- **Q:** Which spec is normative for a UI behavior that also touches runtime behavior?
  **A:** The core spec owns the observable behavior/data/security outcome; this spec owns the observable presentation/interaction outcome. Requirements cross-reference rather than duplicate each other.
- **Q:** Are the UI requirements implemented?
  **A:** Yes on Linux: 654 tests pass, with Xvfb/headless test evidence summarized in the [v1 evidence record](../../../../wiki/records/20261026-v1-evidence.md). Visual captures are ad hoc validation artifacts and are not committed. macOS arm64 HudWindow vibrancy and UI rendering were manually verified natively on 2026-10-28 (windowed, resized, and fullscreen); Windows and macOS x64 remain compile-checked, while full native UX and native contrast certification, native Linux desktop, native pointer/keyboard, and hardware-GPU evidence remain pending.
- **Q:** Does this change add scope beyond the active v1?
  **A:** No. It extracts already implemented/discussed UI behavior; derived scenarios only make implicit presentation guarantees explicit. The 6.x–17.x refinements stay inside this UI contract and do not add runtime/product scope.
- **Q:** Does the proposal's “no code changes” statement still apply?
  **A:** It describes the original contract split only. The later 6.x (scrollbars/header/tabs), 7.x (tab glass, focus exception, picker navigation), 8.x (mint accent, brand spacing, header order, close hover treatment, splitter fade), 9.x (inline picker tree, path target, keyboard tree navigation, refresh/re-root state), 10.x (darker fallback clear, opaque fill-only selected/hover workspace tabs), 11.x (tabs-left header, right-side wordmark with 24/18 breathing gaps, reserved right cluster, 34/30 strip (Phase 11; superseded by Phase 12) and 26/24 workspace tabs with session-map metrics preserved), 12.x (gutter-to-gutter header row, card-edge-aligned tabs/icons, 26/24 strip and tabs, 42/36 card top with equal outer pane gaps, and a 360px wide explorer default with the compact 288px default preserved), 13.x (diagnostics previous-log path row), 14.x (hidden-at-rest/focus-visible workspace-tab close with fixed glyph size and no scale animation), and 15.x (softer fallback/native clear, explorer card/header glass alphas, and conditional primary/alternate-screen terminal glass), and 16.x (conditional Explorer native/fallback glass tints), 17.x (neutral terminal cursor `#D7DDE7`, focused reported-style blink, input-phase reset, and boundary repaint scheduling) refinement tasks in this same active change do change UI code and tests; those changes are recorded in `tasks.md`, not retroactively in the original split rationale.
- **Q:** Is the legacy UI design document replaced?
  **A:** Yes. This change's [`design.md`](design.md) is now the durable non-normative contract and implementation reference; the legacy document was removed after migration.
