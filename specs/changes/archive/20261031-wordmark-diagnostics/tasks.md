# Tasks: wordmark-diagnostics

Readiness: `analyze` must pass before implementation; review runs `verify` against the persisted spec.

## 1. Failing About contract tests

- [x] 1.1 Update the header wordmark assertions from the Diagnostics trigger to the About trigger: tooltip and accessibility name `About`, no Diagnostics icon, Sessions-then-Explorer order, About opening and closing only while a workspace is active, a target of at least 24×24, a pointer cursor, a visible focus ring, no `BG_HOVER`/`BG_SELECTED`/`BG_TAB_HOVER`/`BG_TAB_SELECTED` fill, and the exact 24/18 gaps at 1280×800 and 720×480 (path: crates/made-app/tests/ui_model.rs)
- [x] 1.2 Add failing About-screen assertions: the default view shows the MADE wordmark, the Made name and version, the product description, and `Show diagnostics`; the status pill, diagnostics sections, bounded log block, previous-log row, active-log row, and copy diagnostics actions stay hidden; `Show diagnostics` reveals the existing detail and becomes `Hide diagnostics`; `Hide diagnostics` hides it; closing and reopening About resets to the default view; the toggle is keyboard reachable with a visible focus ring and accessible name, uses the active accent treatment while diagnostics are open and the secondary treatment while hidden; the card has a compact height while hidden, animates through an intermediate height, and settles at the available app height when revealed (path: crates/made-app/tests/ui_model.rs)
- [x] 1.3 Update the existing diagnostics opening, previous-log, degraded, legacy-log, and window-rect assertions to open About, activate `Show diagnostics`, and use the About window id instead of the Diagnostics window id (path: crates/made-app/tests/ui_model.rs) [P]

## Checkpoint

- [x] confirm the original wordmark trigger assertions failed against the Diagnostics icon before implementation; the About default/reveal assertions were written with the About implementation

## 2. About trigger and state plumbing

- [x] 2.1 Rename the wordmark trigger helper and tooltip constant to About, add a non-interactive branded wordmark paint helper for the About card, and give the About diagnostics toggle a matching accessible name (path: crates/made-app/src/theme.rs) [P]
- [x] 2.2 Rename the window visibility state from Diagnostics to About across the app and workbench plumbing, and add per-workbench diagnostics-detail state that resets when About opens (path: crates/made-app/src/app.rs, crates/made-app/src/workbench.rs, crates/made-app/src/ui.rs)
- [x] 2.3 Add the short product description constant used by About (path: crates/made-app/src/lib.rs) [P]

## Checkpoint

- [x] confirm the wordmark trigger paints the two-tone wordmark with no fill, an `About` tooltip and accessibility name, and a target of at least 24×24 in a focused fixture
- [x] verify the picker wordmark is unchanged and not an About trigger

## 3. About window and diagnostics reveal

- [x] 3.1 Rename the card to About with the `About` title and `made-about-window` id, make it a fixed, centered, non-resizable card, render the default wordmark/name/version/description block plus the `Show diagnostics`/`Hide diagnostics` toggle with the active accent treatment while open, and reveal the existing diagnostics detail only while that toggle is on (path: crates/made-app/src/ui.rs)
- [x] 3.2 Keep the diagnostics detail, previous-log row, active-log path, log block, copy actions, notices, and responsive clamping unchanged when revealed; size the detail body from the available app height; animate the fade and height over about 220 ms in both directions; reset the detail and compact height when About reopens (path: crates/made-app/src/ui.rs)
- [x] 3.3 Update the README feature bullet from Diagnostics to About with on-demand diagnostics (path: README.md) [P]

## Checkpoint

- [x] verify Quiet shell workspace
- [x] verify Icon-only header controls
- [x] verify Header placement and order
- [x] verify Wordmark About trigger
- [x] verify Unified hover and selection states
- [x] verify MADE wordmark identity
- [x] verify Visible focus on persistent controls
- [x] verify Icon actions expose accessible names
- [x] verify About default content
- [x] verify About diagnostics reveal and reset
- [x] verify About diagnostics reveal animation
- [x] verify About height and resize behavior

## 4. Regression

- [x] 4.1 Run the made-app test suite and confirm no header layout, tab, picker, About, or diagnostics-detail regression, and no remaining test clicks a removed header Diagnostics icon (path: crates/made-app/tests/ui_model.rs)

## Checkpoint

- [x] verify Diagnostics panel presentation with diagnostics revealed still passes unchanged
- [x] verify Shell follow diagnostics without chrome noise with diagnostics revealed still passes unchanged
- [x] verify Previous log path and retention notice with diagnostics revealed still passes unchanged

## 5. Scenario coverage

- [x] verify Quiet shell workspace
- [x] verify Icon-only header controls
- [x] verify Header placement and order
- [x] verify Wordmark About trigger
- [x] verify Unified hover and selection states
- [x] verify Explorer folder and row presentation
- [x] verify Hover-only fading scrollbars
- [x] verify Explorer hide and restore
- [x] verify Explorer resize and compact layout
- [x] verify Explorer header action removal
- [x] verify Hover-only explorer divider
- [x] verify Explorer splitter fade
- [x] verify No manual refresh control
- [x] verify Quiet integration chrome
- [x] verify MADE wordmark identity
- [x] verify Workspace tab presentation
- [x] verify Workspace tab hover and close
- [x] verify Workspace tab close visibility
- [x] verify Workspace tab overflow and close
- [x] verify Visible focus on persistent controls
- [x] verify Icon actions expose accessible names
- [x] verify Keyboard workspace and session navigation
- [x] verify Status cues do not rely on color
- [x] verify About default content
- [x] verify About diagnostics reveal and reset
- [x] verify About diagnostics reveal animation
- [x] verify About height and resize behavior
- [x] verify Shell follow diagnostics without chrome noise
- [x] verify Diagnostics panel presentation
- [x] verify Previous log path and retention notice
