# Design System

Made uses the implemented **Calm Editor** visual system: quiet dark chrome, one amber accent, layered surfaces, and opaque reading surfaces. The terminal is the hero; chrome recedes. The normative visual contract is [`specs/changes/made-ui-ux-v1/spec.md`](../../specs/changes/made-ui-ux-v1/spec.md). Implementation lives in [`theme.rs`](../../crates/made-app/src/theme.rs) and the views under [`crates/made-app/src`](../../crates/made-app/src).

![Current workspace chrome and glass](../assets/screenshots/workspace-glass-1280.png)

## Intent

- Depth comes from layered tonal surfaces, 1px borders, 8–10px gutters, and one soft painted shadow on glass — not gradients or glows.
- One accent (amber) marks active state and primary action. Semantic green/amber/red/blue are reserved for status.
- Terminal and preview are fully opaque and high-contrast. Only the window clear, top chrome band, and Explorer card/header carry alpha.
- Workspace tabs look editor-like. OpenCode sessions are an optional, visually distinct map, not workspace tabs.
- No routine status bar, degraded banner, shell-running pill, path row, version row, empty preview, or empty session strip.

## Core colors

| Token | Value | Use |
|---|---|---|
| `BG_CLEAR` | `rgba(10,11,14,190)` | Window clear/gutter without native effect. |
| `BG_CLEAR_NATIVE` | `rgba(10,11,14,150)` | Window clear when a native backdrop is applied. |
| `BG_TERMINAL` | `#0B0D10` | Terminal canvas/card. |
| `BG_CARD` | `#171A20` | Preview, diagnostics, opaque card fallback. |
| `BG_HEADER` | `#14171C` | Opaque non-glass headers, session strip. |
| `BG_DEEP` | `#12151A` | Diagnostics log, preview gutter. |
| `BG_ELEVATED` | `#1B1F26` | Secondary buttons, chips, tooltips. |
| `BG_HOVER` | `#1D2129` | Row/button hover. |
| `BG_SELECTED` | `#222A35` | Selected tab/row. |
| `BG_CHROME_GLASS` | `rgba(20,22,27,205)` | Frosted top band. |
| `BG_CARD_GLASS` | `rgba(23,26,32,202)` | Frosted full-height Explorer card. |
| `BG_HEADER_GLASS` | `rgba(20,23,28,208)` | Frosted Explorer header. |
| `BORDER` | `#262B34` | Card/band borders and decorative separators. |
| `BORDER_CONTROL` | `#5E697B` | Control boundary; never rely on `BORDER` alone. |
| `TEXT_PRIMARY` | `#E6E8EC` | Titles and active values. |
| `TEXT_SECONDARY` | `#BCC2CC` | Body and default labels. |
| `TEXT_TERTIARY` | `#9AA2B0` | Section labels, paths, metadata. |
| `TEXT_DISABLED` | `#7A8494` | Disabled only. |
| `ACCENT` | `#E9A76A` | Active underline, primary action, focus. |
| `ACCENT_HOVER` | `#F2B87F` | Accent hover. |
| `ACCENT_PRESSED` | `#D89355` | Accent pressed. |
| `SUCCESS` | `#7CC79A` | Connected, running, read-only. |
| `WARNING` | `#E9B872` | Degraded, busy, attention, exited. |
| `ERROR` | `#E58A8A` | Error, unsupported, failed. |
| `INFO` | `#8FB8E8` | Unread, informational. |
| `FOCUS` | `#F4C48E` | Focus ring. |

Tints are opaque precomposited colors, not alpha fills. The only alpha surfaces are the window clear, top band, Explorer card, and Explorer header.

## Glass and native backdrop

| Surface | Alpha |
|---|---:|
| Window backdrop/gutter, no native effect | 190/255 |
| Window backdrop/gutter, native applied | 150/255 |
| Top chrome band | 205/255 |
| Explorer card | 202/255 |
| Explorer header | 208/255 |
| Terminal, preview, picker, diagnostics, tooltip, toast | 255 |

The in-app glass is a translucent tonal fill plus a painted soft shadow and 1px top highlight. It is the portable fallback and remains readable if the compositor ignores window alpha. `BG_CLEAR` then renders as a solid `#0A0B0E` backdrop while the rest of the UI is unchanged.

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

No text below 10px. `Style.text_styles`: Small 10.0, Body 12.5, Button 12.0, Heading 20.0, Monospace 13.0. Card titles are section labels, not `Heading`.

## Spacing, radii, and motion

- Base rhythm 4pt, practical spacing 8pt. Spacing scale: 2, 4, 6, 8, 10, 12, 16, 20, 24, 32.
- Window gutter 10 wide / 8 compact; gaps 8/6; card radius 10/8; header height 34/30; content padding 12/10; tree row 24; buttons 26–28; interactive targets at least 24×24.
- Global spacing: `item_spacing(8,6)`, `button_padding(10,4)`, `interact_size(28,26)`, `window_margin 14`, `indent 14`, `tooltip_width 320`, `animation_time 0.12`.
- Shadows: `SHADOW_FLOAT` for floating overlays, `SHADOW_POPUP`, `SHADOW_TOAST`, and `GLASS_SHADOW` for the band/card. Terminal and preview cards have no shadow.
- Motion is limited to spinners, the Explorer hide/show width transition (~150 ms, quadratic ease-out), and standard control animation. Tabs never animate width. No essential information exists only in motion.

## Contrast and interaction

Key opaque-surface ratios: primary/card 14.21:1; secondary/card 9.73:1; tertiary/card 6.78:1; accent/card 8.46:1; focus/card 10.90:1; terminal foreground/background 14.15:1; control border/card 3.14:1. `BORDER` is decorative and never the sole boundary of an interactive control. Native glass/blur contrast is not certified.

Interactive rules:

- Focus is a 2px surface-colored gap plus 2px `FOCUS` outside ring; inside clipped scroll areas use a 1.5px inside ring. Never rely on fill change alone.
- Tabs use a full-opacity selected background plus accent underline, without dot/hollow markers or active borders. Tree selection adds a 2px accent left bar.
- Status uses shape + text + color: busy spinner, attention `!`, unread number, renaming `*`, tree error `!`, shell state text.
- The pointer-only Explorer divider is not a keyboard stop; the header Explorer toggle is the keyboard hide/restore path.
- `accesskit` is not enabled in `Cargo.toml`. Icon actions set tooltip and `Response::widget_info` names, but platform screen-reader exposure is not claimed.

## Implementation notes

- Theme install runs once per egui context behind `theme_ready`; headless UI tests use the same path.
- `paint_glass_band`, `paint_glass_card`, and `paint_glass_card_header` paint the alpha/shadow/highlight treatment. `wordmark_galley` caches the two-tone layout by pixels-per-point.
- Root frame has margin but no fill, otherwise the translucent backdrop is hidden.
- Terminal card border is overpainted after content because card and terminal fills are both `BG_TERMINAL`.
- `native_options.clipboard_shortcuts = false` so terminal copy/paste semantics are not consumed by egui-winit.

Related: [Layout and controls](layout-and-controls.md), [ADR 0003](../decisions/0003-in-app-glass-native-backdrop.md), [UI spec](../../specs/changes/made-ui-ux-v1/spec.md).
