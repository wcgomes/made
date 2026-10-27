# Design System

Made uses the implemented **Calm Editor** visual system: quiet dark chrome, one mint-green accent, layered surfaces, and opaque reading surfaces. The terminal is the hero; chrome recedes. The normative visual contract is [`specs/changes/made-ui-ux-v1/spec.md`](../../specs/changes/made-ui-ux-v1/spec.md). Implementation lives in [`theme.rs`](../../crates/made-app/src/theme.rs) and the views under [`crates/made-app/src`](../../crates/made-app/src).

## Intent

- Depth comes from layered tonal surfaces, 1px borders, 8–10px gutters, and one soft painted shadow on the Explorer glass — not gradients or glows.
- One accent (mint green) marks the wordmark initial, primary action, focus, spinner, terminal cursor, and splitter drag. Semantic teal/amber/red/blue are reserved for status; success is a cooler teal than the accent.
- Terminal and preview are fully opaque and high-contrast. The header paints no band; workspace tabs use fill-only states: idle translucent black, hover dark green-gray, selected opaque dark green-gray. Only the Explorer card/header uses in-app glass.
- Hover and selection share one borderless fill language across header icons, session tabs, workspace tabs, Explorer rows, and picker rows. Workspace tabs use dedicated idle/hover/selected fills with no stroke; only their text opacity and fill change.
- Workspace tabs look editor-like, with a 28px wide / 20px compact wordmark-to-first-tab gap and no divider. OpenCode sessions are an optional, visually distinct map, not workspace tabs.
- No routine status bar, degraded banner, shell-running pill, path row, version row, empty preview, or empty session strip.

## Core colors

| Token | Value | Use |
|---|---|---|
| `BG_CLEAR` | `rgba(10,11,14,225)` | Window clear/gutter without native effect; darker fallback for bright desktops. |
| `BG_CLEAR_NATIVE` | `rgba(10,11,14,150)` | Window clear when a native backdrop is applied. |
| `BG_CLEAR_SOLID` | `#0A0B0E` | Opaque window clear fallback. |
| `BG_CHROME` | `#14161B` | Opaque chrome fallback and focus gap. |
| `BG_TERMINAL` | `#0B0D10` | Terminal canvas/card. |
| `BG_CARD` | `#171A20` | Preview, diagnostics, opaque card fallback. |
| `BG_HEADER` | `#14171C` | Opaque non-glass headers, session strip. |
| `BG_DEEP` | `#12151A` | Diagnostics log, preview gutter. |
| `BG_ELEVATED` | `#1B1F26` | Secondary buttons, chips, tooltips. |
| `BG_HOVER` | `#1D2129` | Unified row/button/session-tab hover; not workspace tabs. |
| `BG_SELECTED` | `#222A35` | Selected row/session-tab/icon/path-chip fill; not workspace tabs. |
| `BG_TAB_IDLE` | `rgba(0,0,0,84)` | Inactive workspace-tab fill. |
| `BG_TAB_HOVER` | `#151C18` | Workspace-tab hover fill; selected tabs keep their selected fill. |
| `BG_TAB_SELECTED` | `#1B231F` | Opaque selected workspace-tab fill. |
| `BG_CARD_GLASS` | `rgba(23,26,32,202)` | Frosted full-height Explorer card. |
| `BG_HEADER_GLASS` | `rgba(20,23,28,208)` | Frosted Explorer header. |
| `BORDER` | `#262B34` | Card borders and decorative separators. |
| `BORDER_HEADER` | `#20252D` | Header dividers inside cards. |
| `BORDER_CONTROL` | `#5E697B` | Control boundary and splitter hover line/grip; never rely on `BORDER` alone. |
| `TEXT_PRIMARY` | `#E6E8EC` | Titles and active values. |
| `TEXT_SECONDARY` | `#BCC2CC` | Body and default labels. |
| `TEXT_TERTIARY` | `#9AA2B0` | Section labels, paths, metadata. |
| `TEXT_DISABLED` | `#7A8494` | Disabled only. |
| `ACCENT` | `#8BD5A7` | Wordmark initial, primary action, focus accent, spinner, terminal cursor, splitter drag. |
| `ACCENT_HOVER` | `#A3E3BA` | Accent hover. |
| `ACCENT_PRESSED` | `#72BF8E` | Accent pressed. |
| `ACCENT_TEXT` | `#08140D` | Text on accent. |
| `ACCENT_TINT` | `#26352C` | Accent-tinted inset surfaces. |
| `SUCCESS` | `#4FCFA8` | Connected, running, read-only; cool teal, distinct from the mint accent. |
| `WARNING` | `#E9B872` | Degraded, busy, attention, exited. |
| `ERROR` | `#E58A8A` | Error, workspace-tab close hover, unsupported, failed. |
| `INFO` | `#8FB8E8` | Unread, informational. |
| `FOCUS` | `#B8F0C9` | Focus ring. |
| `TERMINAL_CURSOR` | `#8BD5A7` | Terminal cursor. |
| `SCROLL_TRACK` | `#14171D` | Terminal track only while the bar is hovered or dragged. |
| `SCROLL_THUMB` | `#626C7D` | Floating scrollbar thumb. |
| `SCROLL_THUMB_HOVER` | `#7A8494` | Thumb on bar hover or drag. |

Tints are opaque precomposited colors, not alpha fills. The persistent alpha surfaces are the window clear, Explorer card/header, and idle workspace tabs. Hover and selected workspace tabs are opaque fills. The header itself paints no band. Inactive workspace tabs use `TEXT_SECONDARY` at 0.72 opacity; inactive header icons use `TEXT_TERTIARY` at 0.80 opacity; hover and selection restore full opacity.

## Glass and native backdrop

| Surface | Alpha |
|---|---:|
| Window backdrop/gutter, no native effect | 225/255 |
| Window backdrop/gutter, native applied | 150/255 |
| Floating workspace header | 0 (no band) |
| Hover workspace tab | 255 (`BG_TAB_HOVER`) |
| Selected workspace tab | 255 (`BG_TAB_SELECTED`) |
| Inactive workspace tab | 84/255 black |
| Explorer card | 202/255 |
| Explorer header | 208/255 |
| Terminal, preview, picker, diagnostics, tooltip, toast | 255 |

The in-app glass is a translucent tonal fill plus a painted soft shadow and 1px top highlight on the Explorer card/header. Workspace tabs do not use Explorer glass: they are fill-only, with an opaque dark green-gray hover/selected state and a translucent black idle state. The workspace header paints no band fill, border, shadow, or highlight and floats directly over the clear/native backdrop. `BG_CLEAR` then renders as a solid `#0A0B0E` backdrop while the rest of the UI is unchanged. The no-native clear alpha is 225/255, while the native-effect clear stays 150/255; Xvfb without a compositor cannot visually validate blending over a light desktop, so the tokens are asserted by tests.

`made-platform::native_backdrop` optionally layers an OS material behind the window:

- **macOS:** `NSVisualEffectView` HudWindow vibrancy, active state.
- **Windows 11:** Mica only. Acrylic/blur are intentionally not used because `window-vibrancy` documents resize/drag lag.
- **Linux/unsupported compositor/failure:** `InAppGlass` with `applied=false` and an honest reason.
- `MADE_NATIVE_BACKDROP=auto` (default, including unset/unrecognized values) attempts the effect once on the first UI frame; `off` skips it. There is no retry or runtime material switch.
- The clear color uses alpha 150 only when the status says the native effect was applied.
- Native macOS/Windows rendering is compile-checked only in this repository; Linux Xvfb confirms the fallback. No native blur claim is made without runtime evidence.

`TRANSLUCENT_WINDOW` in [`theme.rs`](../../crates/made-app/src/theme.rs) is the transparency kill switch. When false, the window is opaque and uses no native backdrop. The UI has no OS branch; platform selection stays in `made-platform`.

## Typography

Bundled egui fonts only: **Ubuntu-Light** proportional and **Hack** monospace. Ubuntu-Light has one weight, so `.strong()` changes color rather than weight. Hierarchy comes from size, color, and letter spacing.

| Role | Size / line | Family |
|---|---:|---|
| Brand `MADE` | 14 / 18, tracking 1.8 | Hack |
| Workspace/session tab, selected | 13.0 / centered | Ubuntu-Light |
| Workspace/session tab, inactive | 12.5 / centered | Ubuntu-Light |
| Hero (picker) | 22 / 28 | Ubuntu-Light |
| Body | 12.5 / 18 | Ubuntu-Light |
| Button | 12 / 16 | Ubuntu-Light |
| Section label | 10.5 / 14, uppercase, 0.6 tracking | Ubuntu-Light |
| Meta | 10.5 / 15 | Ubuntu-Light |
| Micro | 10 / 14 | Ubuntu-Light |
| Terminal | 13.0 / measured | Hack |
| Preview code | 13.0 / 18 | Hack |
| Preview line number | 10.5 / 18 | Hack |
| Diagnostics value/log | 10.5 / 16 and 15 | Hack |

No text below 10px. `Style.text_styles`: Small 10.0, Body 12.5, Button 12.0, Heading 20.0, Monospace 13.0. Card titles are section labels, not `Heading`. The selected tab label is 13.0px versus 12.5px inactive; this small size difference is the non-color selection cue and does not change tab width or animate layout.

## Spacing, radii, and motion

- Base rhythm 4pt, practical spacing 8pt. Spacing scale: 2, 4, 6, 8, 10, 12, 16, 20, 24, 32.
- Window gutter 10 wide / 8 compact; pane gap 8/6; workspace strip and tabs 26/24; card content top 42/36; card header 34/30; card content padding 12/10; tree row 24; workspace-tab radius 6; picker path-chip radius 4; buttons 26–28; interactive targets at least 24×24.
- Global spacing: `item_spacing(8,6)`, `button_padding(10,4)`, `interact_size(28,26)`, `window_margin 14`, `indent 14`, `tooltip_width 320`, `animation_time 0.12`.
- Header geometry: the row spans gutter..width-gutter with no extra horizontal padding; tabs start at the card/window content left edge; the wordmark sits before the right icon group with a reserved 24/18 strip-side gap and an exact 24/18 icon-side gap; the last icon’s 26/24 target, selected/hover fill, and focus ring all end exactly at the card right (`content.right`) with no gutter overhang; the 14px glyph stays centered in its target with normal optical padding. Session-map strip/tabs remain 36/34 and 28/26.
- Floating scrollbars are overlay-only: 8px expanded bar, 3px thin thumb, 24px minimum handle, and zero allocated width. They are hidden at rest, fade in on scroll-area hover, and expand with a faint track only on bar hover or drag.
- Shadows: `SHADOW_FLOAT` for floating overlays, `SHADOW_POPUP`, `SHADOW_TOAST`, and `GLASS_SHADOW` for the Explorer card/header. The floating header/selected tab and terminal/preview cards have no shadow.
- Motion is limited to spinners, hover-triggered scrollbar fades (~120–160 ms), the Explorer splitter fade (~0.14 s), the Explorer hide/show width transition (~150 ms, quadratic ease-out), and standard control animation. Tabs never animate width; the close X is hidden at rest, appears on hover/focus at a fixed size, and never changes its 24×24 hit rect. No essential information exists only in motion.

## Contrast and interaction

Key opaque-surface ratios: primary/card 14.21:1; secondary/card 9.73:1; tertiary/card 6.78:1; accent/card 10.10:1; accent text/accent 10.91:1; success/success tint 7.12:1; focus/card 13.56:1; terminal foreground/background 14.15:1; control border/card 3.14:1. `BORDER` is decorative and never the sole boundary of an interactive control. Native glass/blur contrast is not certified.

| Element | Rest | Hover | Selected/Active |
|---|---|---|---|
| Workspace tab | `BG_TAB_IDLE`; `TEXT_SECONDARY` 12.5px at 0.72; no stroke | `BG_TAB_HOVER`; primary label; no stroke | `BG_TAB_SELECTED`; `TEXT_PRIMARY` 13.0px; no stroke, underline, or dot |
| Workspace tab close | hidden; 24×24 target reserved | visible at fixed 14px; no fill; `ERROR` glyph on direct X hover; parent tab uses its hover fill (selected keeps `BG_TAB_SELECTED`); “Close workspace” suppresses the parent path tooltip | n/a |
| Open-folder `+` | tab-strip icon; tertiary at 0.80; focus ring; Enter/Space | `BG_HOVER`; primary | n/a |
| Header icon | transparent; `TEXT_TERTIARY` at 0.80 | borderless radius-6 `BG_HOVER`; primary | `BG_SELECTED`; primary; no border or accent bar |
| Session tab | transparent; secondary at 0.72 | borderless radius-6 `BG_HOVER`; primary | `BG_SELECTED`; primary; no underline or border |
| Explorer row | transparent | borderless radius-6 `BG_HOVER` | inset radius-6 `BG_SELECTED`; primary text; no border, accent bar, or focus outline |
| Picker tree row | transparent | borderless radius-6 `BG_HOVER` | inset radius-6 `BG_SELECTED`; primary text; app-drawn disclosure; no border, accent bar, or focus outline |
| Picker path chip | inset `BG_SELECTED` radius 4; primary mono 10.5; no border | no fill change | selected tree-row path, or the root when nothing is selected |
| Explorer splitter | paints nothing; 8 px hit area | 1 px line + 6×32 grip fade to partial-opacity `BORDER_CONTROL` over 0.14 s | drag: same geometry in restrained `ACCENT` |
| Picker secondary action | borderless `BG_ELEVATED` | borderless `BG_HOVER` | borderless `BG_SELECTED` |
| Primary action | `ACCENT` | `ACCENT_HOVER` | `ACCENT_PRESSED` |

- Focus is a 2px surface-colored gap plus 2px `FOCUS` outside ring; inside clipped scroll areas use a 1.5px inside ring. Never rely on fill change alone. Deliberate exception: Explorer tree rows/container and picker rows paint no focus ring; their focus/selection is the inset `BG_SELECTED` fill plus primary text. Picker card/buttons and all other controls keep their rings; the workspace-tab close uses an inside ring.
- Keyboard focus entering the Explorer with no prior selection selects the first visible row without opening a preview.
- Workspace tabs are fill-only: no selected/hover stroke, underline, dot, or width animation. The selected tab is `BG_TAB_SELECTED` at 13px primary; hover is `BG_TAB_HOVER` at 12.5px primary; idle stays `BG_TAB_IDLE` with reduced-opacity secondary text.
- Expanded Explorer folders are marked only by a small app-drawn disclosure triangle and a brighter label; no border, accent side bar, or extra expansion fill. Files keep a quiet app-drawn bullet. Markers occupy the 16px slot and share the label galley's optical center.
- The picker is an inline expandable tree rooted at the start directory. Root children are depth 0; expanding inserts children below the parent and keeps ancestors visible; collapsing removes only the subtree. There is one flattened selected row, 32px high with 4px inset, radius 6, 14px depth, and app-drawn disclosure triangles.
- Picker keyboard: Up/Down selects visible rows; Right expands or selects the first child; Left collapses or selects the parent; Enter toggles expansion; Cmd/Ctrl+Enter confirms; Backspace or Up re-roots to the parent and clears expansion/selection. Refresh preserves valid expansion/selection, each directory keeps the 1000-entry cap, and an unreadable child stays collapsed with an error. Minimal scrolling reveals the selected row and its visible ancestors without centering.
- Scrollbars are hidden at rest. The floating overlay fades in as a thumb over about 120–160 ms when the scroll area is hovered, expands and strengthens on bar hover/drag with a faint track only then, and fades out on leave. They allocate zero width and never shift layout. The custom terminal scrollback bar matches this behavior: full 8px hit area, thumb only on pane hover, and a track only on bar hover/drag.
- Status uses shape + text + color: busy spinner, attention `!`, unread number, renaming `*`, tree error `!`, shell state text. Success is a cooler teal than the mint accent, and no status relies on hue alone.
- Close hover carries the `ERROR` tone plus the parent hover/selected fill and label; visibility is driven by hover/focus, not motion, so the state is not cue-only. The pointer-only Explorer divider is not a keyboard stop; its fade is independent of the 8px hit area, resize cursor, and tooltip, and the header Explorer toggle remains the keyboard hide/restore path.
- `accesskit` is not enabled in `Cargo.toml`. Icon actions set tooltip and `Response::widget_info` names, but platform screen-reader exposure is not claimed.

## Implementation notes

- Theme install runs once per egui context behind `theme_ready`; headless UI tests use the same path.
- `paint_glass_card` and `paint_glass_card_header` paint the Explorer alpha/shadow/highlight treatment. Workspace tabs paint `BG_TAB_IDLE`, `BG_TAB_HOVER`, or `BG_TAB_SELECTED` with no stroke. `wordmark_galley` caches the two-tone layout by pixels-per-point.
- `paint_disclosure` and `paint_bullet` draw the Explorer/picker markers from the row's optical center; `BG_TAB_IDLE` supplies the idle workspace-tab fill. close visibility is state-driven at a fixed glyph size (no scale), and `splitter_opacity` drives the supplementary splitter fade.
- `scroll_style` installs floating overlay scrollbars with dormant track/thumb opacity 0 and zero allocated width; the terminal paints its own matching scrollback bar. `wordmark_galley` and the brand-gap layout place the wordmark 28px (20 compact) before the first tab without a divider.
- Root frame has margin but no fill, otherwise the translucent backdrop is hidden.
- Terminal card border is overpainted after content because card and terminal fills are both `BG_TERMINAL`.
- `native_options.clipboard_shortcuts = false` so terminal copy/paste semantics are not consumed by egui-winit.

Related: [Layout and controls](layout-and-controls.md), [ADR 0003](../decisions/0003-in-app-glass-native-backdrop.md), [UI spec](../../specs/changes/made-ui-ux-v1/spec.md).
