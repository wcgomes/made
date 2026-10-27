# Design: made-ui-ux-v1

## Approach

This change extracts the UI/UX/visual contract that was embedded in the active v1 bundle into a dedicated active change. The core change remains the owner of product, runtime, integration, security, and performance behavior; this design owns the durable visual and interaction rationale and the implementation direction for the native Calm Editor shell.

The implemented stack is one native eframe/egui 0.37 window with no webview. The terminal grid is painted by a dedicated renderer; egui composes the workspace strip, optional session map, explorer, full-left preview, picker, diagnostics, tooltips, and toast. The visual language is dark, quiet, terminal-first, and intentionally narrow: one calm mint-green accent, layered shortcuts, and no persistent technical chrome.

Alternatives considered:

| Alternative | Why rejected |
|---|---|
| Keep the full UI contract inside the core change | Mixed product/runtime guarantees with visual presentation, made the core delta hard to review, and duplicated UI detail across spec, design, and a separate UI reference document |
| Treat UI as post-delivery polish | The v1 exit test explicitly includes Calm Editor quality, fluidity, responsive layout, and accessibility; these are acceptance conditions, not decoration |
| Adopt a webview or per-OS UI fork | Violates the no-webview/lightness goal and the one-source-tree rule; the platform layer owns OS capabilities only |
| Introduce a component library or design system package | The implemented UI is a small native surface; a new abstraction would add ceremony without solving a current coupling problem |
| Add animation everywhere to signal quality | Motion is limited to spinners, hover-triggered scrollbar fades, and the explorer transition; animation must not carry essential information or animate tab widths |

This document is the durable visual reference. It is non-normative; the observable guarantees are in [`spec.md`](spec.md). The core implementation design is [`../opencode-session-follow-v1/design.md`](../opencode-session-follow-v1/design.md).

## Significant decisions

### U1: Split UI from runtime behavior

The core change keeps launch resolution, terminal fidelity, tab lifecycle, root arbitration, session follow, bridge/config security, preview safety, watcher behavior, diagnostics data, and performance budgets. This change keeps layout, chrome, wordmark, tabs/session-map presentation, picker interaction, preview/diagnostics presentation, glass/backdrop, motion, responsive behavior, keyboard focus, and accessibility presentation.

Why: the two concerns change for different reasons. A terminal PTY fix should not rewrite a visual contract; a wordmark or spacing change should not reopen root-precedence scenarios. The split also gives each active change a truthful task/evidence owner.

Consequence: some requirements have a deliberate seam. The core spec states that a preview is read-only, keeps the terminal process alive, and rejects stale roots; this spec states that the preview occupies the full left pane and offers the documented return actions. Reviewers must check both, but neither spec duplicates the other's guarantee.

### U2: Calm Editor as the visual source of truth

The implemented theme is retained rather than redesigned. The palette is dark with one calm mint-green accent; semantic success (cool teal), warning (amber), error (red), and info (blue) remain reserved for status and are shape/text-backed, so the accent never doubles as status. Depth comes from layered surfaces, 1px borders, 8–10px gaps, and one soft painted shadow on the explorer card/header, not gradients or glows; the workspace header floats directly on the clear/native backdrop without a persistent band. Terminal and all reading surfaces stay opaque and high-contrast.

Why: the terminal is the hero. Chrome that competes with terminal content reduces legibility and makes the product feel like a dashboard rather than a workspace. A single accent keeps selection, focus, and primary action coherent.

### U3: In-app glass is the portable fallback; native backdrop is optional

The in-app glass approximation is present on the full-height explorer card/header: translucent tonal fill, a painted soft shadow on the explorer card, and a 1px top highlight on the explorer header. The workspace header has no persistent fill, border, shadow, or highlight; its wordmark, tabs, and icon actions float directly on the clear/native backdrop. Inactive tabs use the translucent black idle fill; hover uses the opaque `BG_TAB_HOVER` fill; selected uses the opaque dark green-gray `BG_TAB_SELECTED` fill, with no tab stroke. On macOS, `made_platform::native_backdrop` may request `NSVisualEffectView` HudWindow vibrancy; on Windows 11 it may request Mica only. Linux, unsupported compositors, `MADE_NATIVE_BACKDROP=off`, and failures keep `InAppGlass` for the explorer with `applied=false` and an honest reason. The fallback clear alpha is 225/255 and drops to 150/255 only when the native effect was applied.

Why: native blur is compositor-dependent and cannot be promised from a compile check. The portable explorer fallback keeps the UI readable and allows the native effect to improve depth where it is validated. A floating header avoids a second competing glass slab against the terminal. Windows acrylic/blur are excluded because `window-vibrancy` documents resize/drag lag.

### U4: Quiet chrome, contextual failures

The header carries workspace tabs from the card/window-gutter left edge, the wordmark before the right icon group, and icon-only actions; the row spans gutter..width-gutter with no extra horizontal padding. The app-drawn `+` Open folder action sits in the tab strip after the last visible tab and before any overflow trigger; the wordmark sits between the tab strip and the icon group with a reserved 24px wide / 18px compact strip-side gap and an exact 24/18 icon-side gap; the strip-side gap is reserved from the tab-fitting boundary, not measured from the last visible control, so few tabs leave more visible space; the right icon group orders Sessions (when available), Diagnostics, Explorer so the Explorer toggle is rightmost; the last 26/24 target, visible glyph, selected/hover fill, and focus ring all end at the card right border with no gutter overhang; the 14px glyph is optically right-aligned inside its target. There is no routine bottom status bar, shell-running pill, path row, version row, watcher banner, degraded banner, empty session strip, or empty preview placeholder. Technical detail lives in Diagnostics. Genuine shell/file/user-action errors stay contextual and non-blocking.

Why: persistent technical chrome consumes terminal height and turns normal optional integration into apparent failure. Icon-only actions reduce width pressure; tooltips, accessible names, and visible focus preserve discoverability.

### U5: Full-height explorer, full-left preview

The explorer owns the full right content column and never shares that column with a preview. A file opens as a read-only surface across the entire left content pane, replacing the terminal visually while its PTY keeps running. Closing/returning restores the same terminal and focus. The explorer is hidden/restored from the header and resized only by a pointer-drag divider; it has no Wider/Narrower/Reload/Hide text actions.

Why: a full-height tree is easier to scan, a full-left preview is easier to read, and a single left-surface choice keeps focus/input ownership unambiguous. A pointer-only divider removes permanent chrome while hide/restore remains keyboard reachable.

### U6: Keyboard and accessibility are acceptance criteria

Every persistent control is keyboard reachable with visible focus. Icon actions expose tooltip/accessibility-name metadata. The pointer-only divider is intentionally not a keyboard stop. Tree/picker row focus is deliberately shown by the selection fill plus primary text rather than an outline, while picker card/buttons and all other controls keep their rings. Status uses shape plus text plus color. Targets are at least 24×24 and text never falls below 10px. Platform screen-reader exposure is not claimed while accesskit is absent.

Why: the visual contract cannot pass on appearance alone. A keyboard user must be able to open folders, switch/close workspaces, hide/restore the explorer, open diagnostics, operate the picker, return from preview, and navigate the tree. The accesskit caveat is stated rather than hidden.

## Flow

```
Window
├─ Floating header: workspace tabs + open-folder `+` | 24/18 gap | MADE wordmark | 24/18 gap | right icons (Sessions?, Diagnostics, Explorer)
├─ Main content
│  ├─ Left surface: terminal OR full-left read-only preview
│  └─ Right surface: full-height explorer (hideable/resizable)
├─ Optional session map (selected workspace, initially hidden)
├─ Contextual toast (action errors)
└─ Diagnostics floating card (on request)
```

Interaction flows:

1. **Launch:** no argument opens the platform-resolved home workspace; `--picker` or Open folder show the picker; an explicit path wins over `--picker`. The picker starts no process until confirmation.
2. **Workspace change:** selecting a workspace tab switches the selected left surface and explorer state without stopping sibling shells.
3. **File open:** explorer selection replaces the left terminal visual with preview; the PTY continues consuming output; Close/Terminal returns to the same live terminal.
4. **Explorer hide/show:** the panel width animates over ~150 ms; content is laid out at its stored width and clipped to the sliding panel; reversal settles at the stored width.
5. **Explorer resize:** pointer drag only, clamped to leave usable left/right widths; hover fades in a discreet thin line/grip over ~140 ms, drag uses the restrained green accent, and rest paints nothing.
6. **Session map:** optional, scoped to the selected workspace, initially hidden; activity indicators update without converting sessions into workspace tabs.
7. **Diagnostics:** opens as an opaque floating card over the workspace; copying diagnostics/log paths never includes workspace credentials.

## Design tokens

### Colors

| Token | Value | Use |
|---|---|---|
| `BG_CLEAR` | `rgba(10,11,14,225)` | Default/fallback window clear (no native effect) |
| `BG_CLEAR_NATIVE` | `rgba(10,11,14,150)` | Clear when a native backdrop is applied |
| `BG_CHROME` | `#14161B` | Opaque chrome fallback, focus gap |
| `BG_CARD` | `#171A20` | Preview/diagnostics cards and opaque fallback |
| `BG_HEADER` | `#14171C` | Opaque session strip/non-glass headers |
| `BG_DEEP` | `#12151A` | Diagnostics log, preview gutter |
| `BG_TERMINAL` | `#0B0D10` | Terminal canvas/card |
| `BG_ELEVATED` | `#1B1F26` | Secondary buttons, chips, tooltips |
| `BG_HOVER` | `#1D2129` | Unified row/button/session-tab hover (not workspace tabs) |
| `BG_SELECTED` | `#222A35` | Unified selected row/icon/session-tab fill |
| `BG_TAB_IDLE` | `rgba(0,0,0,84)` | Inactive workspace-tab fill at rest |
| `BG_TAB_HOVER` | `#151C18` | Opaque fill-only workspace-tab hover |
| `BG_TAB_SELECTED` | `#1B231F` | Opaque dark green-gray selected workspace-tab fill |
| `BG_CARD_GLASS` | `rgba(23,26,32,202)` | Frosted explorer card |
| `BG_HEADER_GLASS` | `rgba(20,23,28,208)` | Frosted explorer header |
| `GLASS_HIGHLIGHT` | `rgba(255,255,255,12)` | 1px inner top highlight |
| `GLASS_SHADOW` | `rgba(0,0,0,58)` | Painted soft glass shadow |
| `BORDER` | `#262B34` | Card borders; decorative separators |
| `BORDER_HEADER` | `#20252D` | Header dividers inside cards |
| `BORDER_CONTROL` | `#5E697B` | Secondary control boundary |
| `BORDER_CONTROL_HOVER` | `#7A8494` | Hover boundary |
| `TEXT_PRIMARY` | `#E6E8EC` | Titles, active values |
| `TEXT_SECONDARY` | `#BCC2CC` | Body, inactive tab titles |
| `TEXT_TERTIARY` | `#9AA2B0` | Section labels, paths, metadata |
| `TEXT_DISABLED` | `#7A8494` | Disabled only |
| `ACCENT` | `#8BD5A7` | Primary action, wordmark M, focus accent, splitter drag (not a tab outline) |
| `ACCENT_HOVER` | `#A3E3BA` | Accent hover |
| `ACCENT_PRESSED` | `#72BF8E` | Accent pressed |
| `ACCENT_TEXT` | `#08140D` | Text on accent |
| `ACCENT_TINT` | `#26352C` | Accent-tinted inset surfaces |
| `SUCCESS` | `#4FCFA8` | Connected, running, read-only (cool teal, distinct from the mint accent) |
| `SUCCESS_TINT` | `#232F2F` | Success/read-only pill |
| `WARNING` | `#E9B872` | Degraded, busy, attention, exited |
| `WARNING_TINT` | `#302D2A` | Warning banner/pill |
| `ERROR` | `#E58A8A` | Error, unsupported, failed, close-X hover |
| `ERROR_TINT` | `#30272D` | Error banner/pill |
| `INFO` | `#8FB8E8` | Unread, informational |
| `INFO_TINT` | `#252D38` | Unread badge |
| `FOCUS` | `#B8F0C9` | Focus ring |
| `SCROLL_TRACK` | `#14171D` | Terminal scrollbar track while revealed |
| `SCROLL_THUMB` | `#626C7D` | Scrollbar thumb at rest opacity |
| `SCROLL_THUMB_HOVER` | `#7A8494` | Scrollbar thumb on bar hover/drag |

Tints are precomposited opaque colors, not alpha fills. The explorer card/header are the persistent alpha fills; inactive workspace tabs use the translucent black idle fill, while selected/hover tabs use opaque green-gray fills. `BORDER` is decorative for cards and is never the sole boundary of a button-like control; use `BORDER_CONTROL` or `ACCENT`. Workspace tabs MUST NOT use an accent or border outline; close-X hover uses `ERROR`; the dragging splitter uses `ACCENT`. Inactive header tabs use `TEXT_SECONDARY` at 0.72 opacity; inactive header icons use `TEXT_TERTIARY` at 0.80 opacity; hover/selection restores full opacity. These are text/fill opacities, not reading-surface transparency.

### Alpha and glass layering

| Surface | Alpha |
|---|---:|
| Window backdrop/gutter, no native effect | `225/255` |
| Window backdrop/gutter, native effect applied | `150/255` |
| Floating header | `0` (no fill) |
| Explorer card | `202/255` |
| Explorer card header | `208/255` |
| Selected workspace tab | `255` (opaque `BG_TAB_SELECTED`) |
| Inactive workspace tab | black `84/255`; hover `255` (opaque `BG_TAB_HOVER`) |
| Terminal, preview, picker, diagnostics, tooltip, toast | `255` |

`MADE_NATIVE_BACKDROP=auto` (default, including unset/unrecognized) attempts the native effect; `off` skips it. macOS requests HudWindow vibrancy; Windows 11 requests Mica only. Linux, unsupported compositors and failures use the in-app explorer glass. The native macOS/Windows effect is compile-checked but not runtime-validated in this repository; no live-blur claim is made for those targets. If the compositor ignores alpha, the RGB clear renders as a solid dark backdrop and the UI is otherwise unchanged. `TRANSLUCENT_WINDOW` is the kill switch; `false` requests no transparency and uses opaque clear.

### Typography

Bundled egui fonts only: proportional Ubuntu-Light, monospace Hack. Ubuntu-Light has one weight; `.strong()` changes color, not weight.

| Role | Size / line | Family | Color |
|---|---:|---|---|
| Brand wordmark | 14 / 18, tracking 1.8 | monospace | `ACCENT` M + `TEXT_PRIMARY` ADE |
| Workspace/session tab, selected | 13 / centered | proportional | `TEXT_PRIMARY` |
| Workspace/session tab, inactive | 12.5 / centered | proportional | `TEXT_SECONDARY` at 0.72 opacity |
| Hero (picker) | 22 / 28 | proportional | `TEXT_PRIMARY` |
| Body | 12.5 / 18 | proportional | `TEXT_SECONDARY` |
| Button | 12 / 16 | proportional | per state |
| Section label | 10.5 / 14, uppercase, spacing 0.6 | proportional | `TEXT_TERTIARY` |
| Meta | 10.5 / 15 | proportional | `TEXT_TERTIARY` |
| Micro | 10 / 14 | proportional | `TEXT_TERTIARY` |
| Terminal | 13.0 / measured | monospace | palette |
| Preview code | 13.0 / 18 | monospace | syntax-adjusted |
| Preview line number | 10.5 / 18 | monospace | `TEXT_TERTIARY` |
| Diagnostics value | 10.5 / 16 | monospace | `TEXT_SECONDARY` |
| Diagnostics log | 10.5 / 15 | monospace | `TEXT_SECONDARY` |

`Style.text_styles`: Small 10.0, Body 12.5, Button 12.0, Heading 20.0, Monospace 13.0. No text below 10px. Avoid `Heading` for card titles.

### Spacing, radii, sizes

Base 4pt, practical rhythm 8pt.

| Token | Wide | Compact (<900) |
|---|---:|---:|
| Window gutter | 10 | 8 |
| Gap between header/cards | 8 | 6 |
| Workspace strip/essential controls | 26 | 24 |
| Card radius | 10 | 8 |
| Card header | 34 | 30 |
| Workspace header horizontal padding | 0 | 0 |
| Card header horizontal padding | 12 | 10 |
| Wordmark reserved gap (strip-side and icon-side) | 24 | 18 |
| Card content padding | 12 | 10 |
| Optional session strip | 36 | 34 |
| Workspace tab height | 26 | 24 |
| Session-map tab height | 28 | 26 |
| Tree row | 24 | 24 |
| Buttons | 26 ghost / 28 primary-secondary | 24 / 26 |
| Status pill | 20 | 18 |
| Badge | 14 | 14 |
| Right explorer default | 440 | 288, constrained by usable left width |
| Explorer height | full content | full content |
| Tree indent | 14/depth | 14 |
| Preview code line | 18 | 18 |

Spacing scale: `2, 4, 6, 8, 10, 12, 16, 20, 24, 32`. Global spacing: `item_spacing(8,6)`, `button_padding(10,4)`, `interact_size(28,26)`, `window_margin 14`, `menu_margin 6`, `indent 14`, `tooltip_width 320`, `animation_time 0.12`. Scrollbars: floating overlay, 8px expanded width, 3px thin width, min handle 24, zero allocated width, dormant track/thumb opacity 0, 0.12s fade-in/out, interactive opacity 1.0.

Shadows: floating overlays plus the explorer glass approximation. The floating header and terminal/preview cards have none.

```text
SHADOW_FLOAT  = offset [0,12], blur 32, color black 120/255
SHADOW_POPUP  = offset [0,6],  blur 18, color black 100/255
SHADOW_TOAST  = offset [0,4],  blur 14, color black 90/255
GLASS_SHADOW  = offset [0,2],  expand 1px, color black 58/255
```

### Interaction states

| Element | Rest | Hover | Selected/Active | Focus | Disabled |
|---|---|---|---|---|---|
| Ghost button | transparent, tertiary | `BG_HOVER`, primary | `BG_SELECTED`, primary, no border | 2px `FOCUS` double ring | `TEXT_DISABLED` |
| Secondary button | `BG_ELEVATED`, 1px `BORDER_CONTROL` | `BG_HOVER` + hover border | `BG_SELECTED` + `BORDER_CONTROL` | 2px `FOCUS` ring | `BG_CARD`, `BORDER`, disabled |
| Primary button | `ACCENT`, `ACCENT_TEXT` | `ACCENT_HOVER` | `ACCENT_PRESSED` | `FOCUS` ring | muted accent |
| Header tab | translucent black `BG_TAB_IDLE`, secondary at 0.72, no border | opaque `BG_TAB_HOVER`, primary, no border | opaque `BG_TAB_SELECTED`, primary; 13px label; no stroke/underline/dot | 2px `FOCUS` ring | n/a |
| Header tab close | no fill, tertiary glyph | no fill, `ERROR` glyph scaled 1.0→1.15 over 0.13s; parent tab also gets its `BG_TAB_HOVER` fill/primary label (or keeps `BG_TAB_SELECTED` when selected) | n/a | 1.5px inside ring | n/a |
| Header icon | transparent, tertiary at 0.80 | `BG_HOVER`, primary | `BG_SELECTED`, primary; no border | 2px `FOCUS` ring | n/a |
| Open-folder `+` | tab-strip icon, tertiary at 0.80 | `BG_HOVER`, primary | n/a | 2px `FOCUS` ring; Enter/Space activates | n/a |
| Explorer splitter | paints nothing; 8px hit area | 1px line + 6×32 grip fade in to partial-opacity `BORDER_CONTROL` over 0.14s | while dragging: same geometry in restrained-opacity `ACCENT` | none; not focusable | n/a |
| Session tab | transparent, secondary at 0.72 | `BG_HOVER`, primary | `BG_SELECTED`, primary; no underline/border | 2px `FOCUS` ring | n/a |
| Tree row | transparent | `BG_HOVER` radius 6 | inset `BG_SELECTED` radius 6 + primary text; no accent bar; no focus outline | none by design; fill + primary text | n/a |
| Picker row | transparent | `BG_HOVER` radius 6 | inset `BG_SELECTED` radius 6 + primary text; disclosure triangle; no border/accent bar; no focus outline | none by design; fill + primary text | n/a |
| Picker path chip | inset `BG_SELECTED` radius 4, primary mono 10.5, no border | no fill change | carries the selected tree-row path or root when nothing is selected | always visible; tooltip full path | n/a |
| Picker secondary action | `BG_ELEVATED`, no border | `BG_HOVER` | `BG_SELECTED` | 2px `FOCUS` ring | muted fill/text |
| Explorer glass card | glass fill, 1px border, shadow | no fill change | n/a | n/a | n/a |
| Card | `BG_CARD`, 1px border | no change | n/a | n/a | n/a |
| Diagnostics pill | tone tint + tone text | no hover | n/a | n/a | n/a |

Focus ring: 2px surface-colored gap at `rect.expand(1.0)`, then 2px `FOCUS` at `rect.expand(3.0)`, `StrokeKind::Outside`. Inside clipped scroll areas use a 1.5px inside ring where a ring applies. Deliberate exception: explorer tree rows/container and picker rows paint no focus ring; their focus/selection is the inset `BG_SELECTED` fill plus `TEXT_PRIMARY` text. The picker card and picker buttons keep their focus rings.

Selected workspace/session tab labels use 13.0px versus 12.5px for inactive tabs, vertically centered. This is the deliberate non-color selection cue; it does not change tab width, layout, or animation. The selected workspace tab uses opaque `BG_TAB_SELECTED #1B231F` with a luminance step above idle/hover and no stroke; inactive tabs use `BG_TAB_IDLE` with no rest border; hover switches to opaque `BG_TAB_HOVER #151C18` with no border, and a selected tab keeps `BG_TAB_SELECTED` on hover. Workspace tabs start at the window gutter/card left edge. The header row spans gutter..width-gutter with no extra horizontal padding, and the last right-side icon’s 26/24 target, visible glyph, selected/hover fill, and focus ring all end at the card right edge with no gutter overhang; the 14px glyph is right-aligned inside the target. The wordmark sits to their right, immediately before the icon group. The tab-fitting budget reserves a 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark; the visible flexible space after the last visible control is larger when few tabs are shown. The wordmark-to-first-icon gap is exactly 24px wide / 18px compact. That right cluster is reserved and does not add a divider. The top strip is 26/24 high and the tabs fill it; the card top is 42/36 with one 8/6 pane gap above and below the strip. The last right-side icon’s 26/24 target, visible glyph, selected/hover fill, and focus ring all end at the card right border with no gutter overhang. This does not change the session-map strip metrics.

## Layout

```
root Frame::NONE, inner margin = gutter, no fill
├─ Panel::top floating header + icon actions (42/36 panel, 26/24 strip, no band)
├─ Panel::right explorer, full content height, hideable/resizable
└─ CentralPanel terminal OR full-left preview
   + optional selected-workspace session map
   + contextual action-error toast (bottom-right)
   + diagnostics window (overlay)
```

Panel separator lines are off. Cards and gaps do the grouping.

### 1280×800

Content after 10px gutter: 1260×780.

| Region | Geometry |
|---|---|
| Floating header | 1260×26 at `(10,8)` |
| Gap | 8 |
| Main content | 1260×748 at `y=42` |
| Right column | 448 total: 8 gap + 440 card at `x=822` |
| Left terminal OR preview | 812×748 at `(10,42)` |
| Full-height explorer | 440×748 at `(830,42)` |
| Explorer hidden | left surface 1260×748 |

No empty session strip, terminal path row, shell-running pill, preview placeholder, or bottom status bar. Preview owns the full left region. Terminal column/row count derives from measured font/grid metrics.

### 720×480

Content after 8px gutter: 704×464.

| Region | Geometry |
|---|---|
| Floating header | 704×24 at `(8,6)` |
| Gap | 6 |
| Main content | 704×436 at `y=36` |
| Right column | 294 total: 6 gap + 288 tree at `x=418` |
| Left terminal OR preview | 410×436 at `(8,36)` |
| Full-height explorer | 288×436 at `(424,36)` |
| Explorer hidden | left surface 704×436 |

Keep the open-folder `+` immediately after the visible tabs (before overflow), tab close/switch, Explorer toggle, and Diagnostics reachable without clipping; optional controls may use a labeled overflow. Preview states and Close/return remain usable at compact size.

### Responsive rules

| Width | Behavior |
|---|---|
| ≥1100 | Folder context may show truncated path; explorer defaults to 440 |
| 900–1099 | Truncated folder context; explorer default 35% within usable bounds |
| <900 | Compact labels, gutter 8, top strip 24, radius 8; explorer default 288 with accessible hide/resize |
| <820 | Optional Sessions may move to labeled overflow; workspace/Explorer/Diagnostics remain reachable |
| Height <600 | Hide preview meta; picker list max 160; diagnostics clamps |

No control may vanish through clipping. Workspace/session overflow lists all of their own tabs; truncate labels with `…` and tooltip full values. Splitter width/visibility and widget ids are per workspace and clamp again when the window shrinks.

## Components

### Workspace strip and header

- `Panel::top("made-header")`, exact 42/36 (8/6 top gap + 26/24 strip + 8/6 bottom gap), no fill, no border, no shadow, no top highlight, no separator.
- Header: transparent over the window clear/native backdrop. No horizontal content padding: the row starts at the gutter/card left edge and ends at `width - gutter`/card right edge. Controls share one vertical center, square 24–26px hit targets and vertically centered glyphs (the last glyph is right-aligned); the row has one 8/6 pane gap above and below. The strip and tabs are 26/24; card content starts at 42/36 and session-map tabs stay 28/26.
- Left: editor-like workspace tabs starting at the window gutter/card left edge, with folder labels, full-path tooltips, reachable close X, stable tab widths and workspace-scoped widget ids, no width animation, and an overflow list when needed. Cached two-tone monospace `MADE` wordmark (mint accent M, primary ADE, 14px, 1.8px tracking, tooltip “Minimal Agentic Development Environment”) sits to the right of the tab strip, immediately before the icon group, with a reserved 24px wide / 18px compact strip-side gap (measured from the tab-fitting boundary, so few tabs leave more visible space before the wordmark) and an exact 24/18 wordmark-to-icon gap; it has no fill/hover/selection and reserves its whole right cluster in the tab-fitting budget. Selected: opaque `BG_TAB_SELECTED` dark green-gray fill, `TEXT_PRIMARY` 13.0px label, no stroke/border, no underline/dot/hollow marker/accent bar. Inactive: `BG_TAB_IDLE` translucent black, secondary 12.5px at 0.72 opacity, no border at rest. Hover: opaque `BG_TAB_HOVER` fill, no border, restore the label to full-opacity primary; selected tabs keep `BG_TAB_SELECTED`; never substitute `BG_HOVER`. Close X: no hover fill; direct hover paints only the glyph `ERROR` and scales it 1.0→1.15 over 0.13s centered in the same 24×24 hit rect, while applying the parent tab's hover fill/label (or keeping the selected fill); the tab width and layout do not change. The selected label’s size is the quiet non-color selection cue; labels are vertically centered in the fixed-height tab.
- Tab strip tail: an app-drawn `+` Open folder action immediately after the last visible tab, before the overflow `…` trigger when tabs overflow. Its 24–28px square target, tooltip “Open folder”, accessible name, visible focus, and Enter/Space activation match the other icon actions; its width and inter-item gaps are reserved by the tab-fitting budget so tabs never overlap it and both `+` and `…` stay reachable.
- Right: icon-only Sessions (when available), Diagnostics, Explorer left-to-right so Explorer is rightmost, with the last icon’s 26/24 target, visible glyph, selected/hover fill, and focus ring all ending at the card right border; the 14px glyph is right-aligned inside its target and nothing overhangs the gutter; no persistent text labels. Each icon is an app-drawn monochrome glyph with a square 24×24 minimum target matching the tab height, matching tooltip, accessible name (`Response::widget_info`), and visible focus ring. Optional Sessions is visually distinct from workspace tabs. Compact overflow is labeled and keyboard reachable.
- Ghost icon button: square, minimum 24; radius 6; inactive glyph `TEXT_TERTIARY` at 0.80 opacity; hover `BG_HOVER`; selected `BG_SELECTED`; no selected border or accent bar. Tooltip is the action name (dynamic Hide/Show for Explorer).
- No version in the header; version is in the picker and diagnostics.

### Optional session map

Initially hidden, scoped to the selected workspace, visually separate from workspace tabs. Opening it reveals session data and focus behavior; normal shell use reserves no strip. It does not create another terminal or convert sessions into workspace tabs.

- Strip 36/34, `BG_HEADER`, top corners only, bottom 1px `BORDER_HEADER`, padding 4/3.
- Tabs 28/26 high, radius 6, gap 4, width `clamp(112, measured + 26, 220)` (compact max 184).
- Content: title 13.0 proportional when active/selected, 12.5 when inactive; vertically centered; busy spinner, attention badge, unread badge, `*` for renaming.
- States: inactive transparent/secondary at 0.72 opacity; hover `BG_HOVER`/primary; active/selected `BG_SELECTED`/primary; no underline, dot/hollow marker, border, or width animation. Focus double ring.
- Indicators: busy 10px accent spinner; attention 12px `WARNING_TINT` circle + `!`; unread 14px `INFO_TINT` badge with `INFO` number; unread pulse 0 = 6px info dot; renaming `*`.
- Click selects and focuses if not already active; Enter/Space activates; Left/Right moves focus.
- Path: secondary line for active/keyboard-focused session; tooltip per tab; overflow rows show all paths.
- Focus error: 24px strip under tabs, `ERROR_TINT`, 2px left `ERROR`, `! Focus failed — …`, Dismiss ghost.
- Empty only when explicitly opened: “No open sessions” + “Open OpenCode in the terminal.” Never reserve by default.
- Overflow: horizontal `ScrollArea` (hidden bar); `...` menu when rows exceed 5 wide / 3 compact; lists all sessions with title and mono path; same focus action; no gradient fade.

### Terminal card

- Fill `BG_TERMINAL`, 1px `BORDER`, radius 10/8, no card padding.
- No default terminal header/path/status row or running pill. Essential folder context is in workspace tabs/explorer; optional session map appears only on request. Shell exit/error retains the contextual recovery surface.
- Body: terminal padding 8; scrollbar 8. No scroll area, opacity, or input-handling change.
- Corner handling: terminal paints a square background; draw the card border after content with `rect_stroke`.
- Shell overlay: bottom-left inset 10px, max width 520/340, `BG_ELEVATED`, 1px `WARNING`, radius 8, padding 10×8; message 11.5 warning; “Restart shell” 24px warning ghost.
- Terminal unavailable: inset error card 24px margin, max 520, `ERROR_TINT`, 1px `ERROR`, radius 8, padding 12; title 13, message 11.5, hint 10.5 tertiary.
- Palette: background `#0B0D10`, foreground `#D7DDE7`, cursor `#8BD5A7`, selection `#2E3A4E`/`#FFFFFF`; keep ANSI 1–7/9–15; raise ANSI 8 to `#707B8E` (4.55:1). The custom scrollback bar has no track or thumb at rest; when the pane is hovered it fades in over 0.14s as a thumb only, and when the bar itself is hovered or dragged the thumb widens to the 8px hit area and a faint track appears. It uses `SCROLL_THUMB`/`SCROLL_THUMB_HOVER` with opacity, fades out on leave, and never changes the grid width.

### Explorer card

- Fill `BG_CARD_GLASS` 202/255, 1px `BORDER`, radius 10/8, painted `GLASS_SHADOW`; optional native material behind the window.
- Full right content height, no preview beneath. Header 34/30, `BG_HEADER_GLASS` 208/255 with 1px `GLASS_HIGHLIGHT`, padding 12/10, bottom divider: “EXPLORER” and root folder 11 mono tertiary with full-path tooltip. No Wider, Narrower, Reload, or Hide text actions. Watcher/live/starting state lives in Diagnostics.
- Hide/show animates panel width over about 150 ms quadratic ease-out. Lay out the tree at its stored width and clip to the sliding panel; reverse cleanly; settle at stored width; stop tree/splitter input once closed. The left surface receives reclaimed space without losing shell/root/selection/preview state.
- Divider: pointer-drag only, clamped to usable left/tree widths. Transparent at rest: no permanent line/grip. Hover fades in over about 140 ms a discreet 1px line and 6×32 grip using `BORDER_CONTROL` at partial opacity; while dragging the same geometry uses the green `ACCENT` at a restrained opacity; leaving fades both out. Keep the 8px pointer hit area, resize cursor, and “Drag to resize explorer” tooltip. Hover alone does not change stored width. Not a keyboard stop; header Explorer toggle remains the keyboard path. Width/visibility/expansion/valid selection survive tab switching and hide/restore, not restart.
- Body: 4px vertical / 8px horizontal padding; vertical floating scroll area (hidden at rest, fading overlay, zero allocated width).
- Row 24px high, horizontal 4px inset, indent 14/depth, radius 6; hover `BG_HOVER`; selected inset `BG_SELECTED` with primary text; no accent side bar, border, or extra expansion fill. No focus outline on the selected row or the tree container: selection/focus is the fill plus primary text. If the container receives keyboard focus before a row is selected, the first visible row becomes selected (fill + primary text, no preview).
- Markers: app-drawn 16px-slot disclosure triangle for directories, app-drawn small bullet for files, `@` symlink, `?` other, `!` error. The triangle/bullet and the laid-out label share one vertical optical center in the row (the label is centered from its galley, not a fixed top offset). Text starts 24px from the row's left edge plus 14px per depth. Expanded folders use the down-triangle and a brighter `TEXT_PRIMARY` label only; files keep a quiet tertiary bullet. Name 12.5 proportional; single-line ellipsis; error rows `ERROR` + tooltip.
- Empty: “This folder is empty.” 11 tertiary. Loading: 12px spinner + “Loading folder…”.
- Root error: inset `ERROR_TINT`, radius 6, padding 10; title 13 error, message 11.5, Retry secondary (error recovery only, not routine reload).
- Root unavailable: 26px `WARNING_TINT` strip with `!` + 10.5 warning, tooltip full.
- Live refresh is watcher-driven; no manual Reload. Explicit file failures retain contextual tree feedback.

### Full-left preview

- Opening a file replaces the entire left terminal surface visually with preview, including binary/size/loading/error states. No preview exists before file open; never stack under Explorer or split beside the terminal. Fill `BG_CARD`, 1px `BORDER`, radius 10/8.
- Header 34/30, padding 12/10, bottom divider: filename 12.5 primary with full-path tooltip and understated read-only label; “Copy path”, “Refresh”, “Close”, and a clearly labeled return-to-terminal control. Close and return restore the same live terminal and focus. No save/edit actions.
- Meta 20/18: `4.2 KB · Rust · 236 lines · updated 0.8s ago` 10.5 tertiary; compact `4.2 KB · 236 lines`; hidden below height 600.
- Content: line-number gutter 44px right-aligned 10.5 mono tertiary, 1px divider, 8px gap; code 13 mono, 18px lines; horizontal scroll; no hover on code.
- Syntax contrast: preserve syntect spans but blend low-contrast colors toward `TEXT_PRIMARY` until ≥4.5:1 against `BG_CARD`, once at document build time on a background thread, not per frame.
- No default empty-preview placeholder. Root change clears preview, invalidates stale reads, returns terminal. Hidden shell keeps consuming output, not preview keystrokes; return does not replay an unbounded backlog or relaunch.
- Loading: 12px spinner + “Loading preview…”. Error: 26px `ERROR_TINT` strip + 10.5 error text; Refresh remains.
- Binary/too large/encoding/missing/unreadable/forbidden: inset state card radius 6, padding 12; warning tint for binary/too-large, error tint for unavailable; title 13, message 11.5, size if known; no raw binary; no edit/save.
- Empty file: “Empty file” + “0 bytes”.

### Contextual feedback

- No persistent bottom status bar. Integration status/versions/revision/heartbeat, shell cwd capability, root source, metadata, and metrics live in Diagnostics. Normal shell use is healthy/inactive optional integration, not degraded.
- Genuine shell/input, root/file, and explicitly requested action errors remain contextual and non-blocking; do not hide recovery actions with removed chrome.
- Toast: bottom-right `Area` within content, max width 360/300, `BG_ELEVATED`, 1px tone border, radius 8, `SHADOW_TOAST`, padding 10×8, text 11; reuse existing TTL; no layout shift.

### Diagnostics floating card

- `Window` frame: `BG_CARD`, 1px `BORDER`, radius 10, inner 14, `SHADOW_FLOAT`, opaque.
- Size: `w = (content_w - 64).clamp(440, 680)`, `h = (content_h - 64).clamp(340, 560)`; centered default; resizable; not collapsible; keep `.open` and close button.
- Header: status pill, reason 10.5 tertiary, “Copy diagnostics” secondary right.
- Body in vertical scroll; section labels “CONNECTION”, “SESSION”, “WORKTREE”, “VERSIONS”, “METRICS”, “LOG” 10 uppercase tertiary; grid labels 10.5 tertiary, values 10.5 mono secondary, spacing 18×4; stack label above value below 560 width.
- Metrics/log blocks `BG_DEEP`, radius 6, padding 8; log max height 160; `#seq` 10 tertiary, level 10 semantic, message 10.5 mono secondary. No zebra rows.
- Footer: log path 10 mono tertiary + “Copy log path” ghost. Preserve every existing row/action.
- Include selected workspace identity, latest shell cwd, cwd reporting capability/reason, and effective root/source. Preserve integration reason, versions, revisions, watcher/metadata state, and metrics when removed from routine chrome; redact every workspace credential from display, logs, and copied diagnostics. Inactive optional integration is not a degraded application state.

### Picker

- No workspace chrome. Brand row 36/32: two-tone `MADE` wordmark + version 10 tertiary; right icon-only Close X with “Close picker” tooltip, plus “Choose a workspace” or Back action.
- Hero: “Open a workspace” 22/28 primary; subtitle 12/18 secondary.
- Card width `min(760, available_w - 48)` (compact 672), `BG_CARD`, 1px `BORDER`, radius 12, no padding.
- Toolbar 40/36: Up/Refresh ghost 24; the path is a compact inset `BG_SELECTED` chip with radius 4, no border, 10.5 mono `TEXT_PRIMARY` text and a full-path tooltip. The chip sizes to the truncated selected tree-row path (or the root when nothing is selected) and stays inside the toolbar without overlapping Up/Refresh or the card edge. Divider.
- List max height `(available_h - 280).clamp(120, 320)` (compact 160); inline expandable tree. Root child folders are depth-0 rows; expanding inserts children below the parent and keeps every ancestor visible. Rows are 32px with a 4px horizontal inset and radius 6; the app-drawn disclosure triangle sits in a 16px slot at the row inset + 14px per depth, and the name starts 24px from the row's left edge + 14px per depth; triangle and label share one vertical optical center. Hover `BG_HOVER`; selected uses only a rounded inset `BG_SELECTED` fill + primary text, with no border, left accent bar, or row/container focus outline. Collapsing removes only the subtree.
- Navigation: one click selects a row and toggles expansion; an expanded folder collapses and stays selected, a collapsed folder expands, and child folders behave the same way. The path chip and “Use current folder” target are the selected row path or the root when nothing is selected; selection is one flattened visible-row index, with no separate hidden child index. The second click of a double-click is ignored so a double-click toggles once and never starts a workspace. Keyboard: Up/Down move visible rows (first/last from no selection); Right expands the selected folder or selects its first child when already expanded; Left collapses the selected folder or selects its parent row when collapsed (root children have no parent row); Enter toggles expansion; Cmd/Ctrl+Enter confirms the selected/current target; Backspace or Up re-roots to the parent directory and clears expansion/selection. Expanding reveals the selection with minimal scrolling (bottom-aligned only when needed) so the visible ancestor chain stays on screen when it fits and otherwise the nearest ancestors plus the selected row stay visible. Refresh re-reads the root and every expanded directory, preserving valid expansion and selection by path; each directory read keeps the 1000-entry cap; no picker state persists across restart. An unreadable expanded child stays collapsed and shows the existing non-blocking error.
- Footer 52/48: right-aligned “Open selected folder” quiet secondary 28 (`BG_ELEVATED` rest, `BG_HOVER` hover, `BG_SELECTED` pressed, no border; disabled when no row is selected), and “Use current folder” primary 28 (only accent-filled control; opens the selected path target or root).
- Error: inline `ERROR_TINT` strip above footer, 10.5 error, tooltip full.
- Hint below card 10.5 tertiary: “Click a folder to expand or collapse it. Up/Down select, Right expands, Left collapses, Enter toggles, Ctrl/Cmd+Enter opens.”
- Place card at `max(24, (available_h - 600)/3)` top padding; never stretch the list to fill the window.
- No-arg startup opens the home workspace directly; picker is explicit via `--picker`, Open folder, or final close. Explicit path wins over `--picker`. Picker starts no process until confirmation. Escape/Close return to the live workspace when present, otherwise default home/terminal fallback; Back remains for add-workspace; confirmation opens a new tab without stopping the existing shell.

### Shared controls

- Ghost 26/24 high; secondary 28/26 with `BORDER_CONTROL`; quiet picker secondary 28/26 borderless; primary 28/26 accent; warning action warning tint/border.
- Diagnostics status pill 20/18; optional session unread badge 14 high, min 18 wide; preview read-only label is contextual, not routine status.
- Interactive targets ≥24×24. Card titles are section labels, not headings.

### Floating scrollbars

- Explorer tree, picker list, preview code, diagnostics body/log, and optional session-map overflow use egui floating overlay scrollbars: 8px expanded bar, 3px thin bar, 24px minimum handle, zero allocated width, dormant track/thumb opacity 0, active-handle fade over ~0.12s, interactive opacity 1.0. The bar area expands on bar hover/drag; content never reflows when a bar appears.
- The terminal scrollback bar is custom-painted with the same rest/hover/fade behavior and `SCROLL_THUMB`/`SCROLL_THUMB_HOVER` colors. On pane hover it paints only the thumb; the faint track appears only on bar hover/drag. It has no always-on track and keeps its full 8px hit area while visually thin.
- Scrollbar fades never carry essential state: scroll position, overflow, and hit area remain available without the animation.

## Transparency and native backdrop policy

- The window clear (`BG_CLEAR` 225/255 fallback, or `BG_CLEAR_NATIVE` 150/255 when native) carries alpha. The explorer card/header are the in-app glass surfaces; selected/hover workspace tabs are opaque fills (`BG_TAB_SELECTED`/`BG_TAB_HOVER`), while idle tabs use the translucent `BG_TAB_IDLE` black at 84/255.
- The explorer surfaces are the in-app frosted-glass approximation: translucent tonal fill + painted `GLASS_SHADOW` + 1px `GLASS_HIGHLIGHT`. This is portable and readable when alpha is ignored. The floating header paints no fill, border, shadow, or highlight.
- `made_platform::native_backdrop` optionally requests a native material once on the first UI frame: macOS `NSVisualEffectView` HudWindow (active), Windows 11 Mica only. Linux, unsupported compositors, `MADE_NATIVE_BACKDROP=off`, and failures keep `InAppGlass` with `applied=false` and an honest reason. Windows acrylic/blur are not used.
- Native macOS/Windows rendering is compile-checked only; no runtime native-effect/blur claim is made. Linux Xvfb confirms the fallback.
- Terminal, preview, picker, diagnostics, tooltips, and toast are opaque reading surfaces. Hover/selection/tints are opaque precomposited colors.
- Do not use `ui.set_opacity` above the terminal or place alpha on terminal/preview/picker text.
- Kill switch: `TRANSLUCENT_WINDOW: bool = true`; when false, no transparent request and alpha 1.0. The UI has no OS branches; platform-specific backdrop selection stays in `made-platform`.

## Motion

- Spinners, standard control animation, hover-triggered scrollbar fades (~0.12s), the workspace-tab close glyph scale (~0.13s), the explorer splitter fade (~0.14s), and the explorer hide/show width transition (~150 ms quadratic ease-out) are the only motion.
- The explorer lays out its content at the stored width and clips to the sliding panel; it does not reflow during the transition, reverses cleanly, and stops tree/splitter input once closed.
- Tabs do not animate width. The close glyph scales within its existing rect without changing the hit area, tab width, or layout; the splitter fade paints only inside its existing 8px hit area and allocates no width. Scrollbars are hidden at rest, fade in only while the scroll area is hovered, and allocate zero width, so the fade never shifts content. Motion never carries essential status/selection/error information.
- Native animation smoothness, native pointer feedback, and native scrollbar hover/fade remain manual validation gaps; Xvfb/headless checks cover the intermediate frame, clipping, reversal, closed input behavior, close-scale geometry, splitter opacity/fade state, and the floating style contract. Temporary Xvfb checks cover explorer/terminal bars at rest and on hover; the fade durations are style/helper-asserted, not measured from captures.

## Accessibility

Contrast (opaque reading surfaces):

| Pair | Ratio |
|---|---:|
| Primary / card | 14.21:1 |
| Secondary / card | 9.73:1 |
| Tertiary / card | 6.78:1 |
| Tertiary / chrome | 6.90:1 |
| Accent / card | 10.10:1 |
| Accent text / accent | 10.91:1 |
| Success / success tint | 7.12:1 |
| Warning / warning tint | 7.54:1 |
| Error / error tint | 5.73:1 |
| Info / info tint | 6.74:1 |
| Focus / card | 13.56:1 |
| Selected tab primary / `BG_TAB_SELECTED` | 13.10:1 |
| Hover tab primary / `BG_TAB_HOVER` | 14.13:1 |
| Terminal fg / bg | 14.15:1 |
| Terminal selection | 11.47:1 |
| ANSI red / terminal | 4.76:1 |
| ANSI bright black adjusted / terminal | 4.55:1 |
| Control border / card | 3.14:1 |

- `BORDER` card edges are decorative; never the sole control boundary.
- Glass surfaces composite over the dark backdrop. The ratios cover opaque reading surfaces; native backdrop rendering, native blur, and glass text contrast are not separately certified.
- ANSI 0 is a terminal background slot and cannot be made readable as foreground. Arbitrary ANSI/truecolor terminal content is outside app control; the default terminal pair meets AA.
- Focus: every keyboard-reachable interactive control has a visible focus state; never rely on fill change alone. Custom tabs/ghost buttons use the double ring; picker card/buttons keep their focus rings. Deliberate exception: explorer tree rows/container and picker rows have no focus outline—their focus/selection is the inset `BG_SELECTED` fill plus `TEXT_PRIMARY` text. Focusing the tree before any selection selects the first visible row without opening a preview. The pointer-only divider is not focusable and uses hover/drag feedback.
- Keyboard: labeled workspace open/switch/close, icon-only Explorer/Diagnostics (tooltip/accessibility names), optional Sessions; workspace/session tabs (Tab in, Left/Right, Enter/Space); terminal as one focus stop; explorer tree (arrows, Enter, Left/Right); preview actions/return; diagnostics normal egui focus. The resize divider is pointer-drag only and not a keyboard stop; the header Explorer toggle is the keyboard hide/restore path. Only the selected visible surface receives input, with visible focus after every transition.
- Colorblind-safe: status = shape + text + color (`●`, `!`, `×`); the cool-teal success tone stays distinct from the mint accent; selected rows use the `BG_SELECTED` fill plus primary text; workspace tabs use the opaque `BG_TAB_SELECTED` luminance step, primary text, and a 13.0px label versus 12.5px inactive, with no dot/underline/hollow marker or border; busy = spinner; attention = `!`; unread = number; renaming = `*`; shell = running/exited/error text; tree errors = `!`. Tree/picker focus uses the same luminance/text cue rather than an outline or hue-only accent. Close-hover scale is a supplementary cue: the `ERROR` tone and parent hover fill/label carry the state without motion. Splitter hover/drag has a resize cursor, tooltip, and hit area independent of the fade.
- Targets ≥24×24; text never below 10px; body 12.5.
- Screen-reader caveat: `accesskit` is not enabled in `Cargo.toml`. This contract improves contrast/focus/keyboard/non-color cues but does not provide a platform accessibility tree. Icon-only header actions set tooltip text and `Response::widget_info` accessible names now; platform screen-reader exposure is not claimed.

## Performance and visual acceptance

- No webview; one native app process plus one shell/process tree per live workspace.
- Terminal responsiveness has priority over background work.
- Startup to interactive terminal and browsable explorer: 2 s for a 10,000-file workspace (SHOULD).
- Reported cwd/active-session change to explorer update: 500 ms (SHOULD).
- Terminal input-to-render: 50 ms under normal load (SHOULD).
- Sustained output: near 60 fps without losing input; drop intermediate frames, never input.
- The UI uses lazy tree loading, damage-driven terminal rendering, bounded channels/scrollback, cancellable tree/preview loads, and no hidden grid painting.
- Visual acceptance includes both 1280×800 and 720×480, long titles/paths, 10k files, binary/large/missing preview, degraded/unsupported state, keyboard-only walkthrough, colorblind/status cues, glass fallback, and no terminal input/selection regression.
- Evidence must distinguish Xvfb/headless software checks from native compositor, native pointer, native keyboard, hardware-GPU, and platform screen-reader evidence. Unverified platforms remain pending.

## Implementation notes

- `crates/made-app/src/theme.rs`: tokens, typography, explorer glass painting, inactive-tab/icon opacity, unified action states, floating scroll style, focus rings, icon primitive (including the `+` glyph), wordmark galley and wordmark-breathing-gap token, close-scale and splitter-fade helpers, clear color/kill switch.
- `crates/made-app/src/ui.rs`: floating workspace header, header actions, tabs/overflow, picker tree composition/rows/borderless secondary action, layout, explorer animation.
- `crates/made-app/src/workspace_picker.rs`: flattened picker tree model, per-directory child caches, expansion/selection/path target, refresh/Up state, per-directory cap and error handling.
- `crates/made-app/src/workspace.rs`: picker tree delegation/toggle/expand/collapse and confirmation target semantics.
- `crates/made-app/src/explorer_view.rs`: full-height glass tree, unified borderless rows, hover-only divider, full-left preview.
- `crates/made-app/src/terminal_view.rs`: terminal grid and the custom hover-only scrollback bar.
- `crates/made-app/src/session_map.rs`: optional map presentation and indicators.
- `crates/made-app/src/diagnostics.rs`: opaque floating card, sections, copy actions.
- `crates/made-platform/src/native_backdrop.rs`: one-shot native request and honest fallback status.
- `crates/made-app/tests/ui_model.rs` and `tests/read_only_ui.rs`: headless visual/interaction evidence.
- [`wiki/records/20261026-v1-evidence.md`](../../../wiki/records/20261026-v1-evidence.md): Linux/Xvfb/headless evidence, pending native rows.
- This design plus [`spec.md`](spec.md): the durable non-normative contract and implementation reference; the legacy UI document was removed after migration.

Do not rewrite terminal input/IME/selection code for visual reasons. Keep `native_options.clipboard_shortcuts = false`. No `#[cfg(target_os)]` outside `made-platform`. Do not introduce a webview, per-OS UI branch, terminal splits, restart persistence, or editing.

## Risks and open evidence

| Risk | Impact | Mitigation or evidence |
|---|---|---|
| Native compositor ignores alpha or renders glass differently | Visual quality or contrast differs by platform | In-app glass fallback, kill switch, native compositor validation remains pending |
| Native macOS vibrancy / Windows Mica not runtime-validated | Native effect could fail or look wrong | Compile check only; honest `applied=false` fallback; native runtime task remains unchecked |
| Xvfb has no window manager/compositor | Hover, focus, animation smoothness are not native evidence | Headless/Xvfb checks plus explicit pending native pointer/keyboard rows |
| Accesskit absent | No platform screen-reader tree | Tooltip/accessibility-name metadata and keyboard/focus coverage; screen-reader claim withheld |
| Compact layout clips controls or overflows | Essential actions become unreachable | 720×480 tests, labeled overflow, focus/target checks; native window-manager validation pending |
| Icon-only actions reduce discoverability | Users cannot find actions | Tooltips, accessible names, visible focus, 24×24 targets, optional Sessions label in overflow |
| Pointer-only divider is not keyboard reachable | Width adjustment unavailable to keyboard users | Intentional trade-off; hide/restore remains keyboard reachable and the divider is not a keyboard stop |
| Removing tree/picker row outlines weakens focus visibility | Keyboard users may not see row focus | Deliberate exception: selection is the inset `BG_SELECTED` fill plus primary text (luminance/text, not hue); picker card/buttons and all other controls keep focus rings; covered by headless focus tests |
| Fill-only workspace-tab hover lacks a border cue | Hover may be subtle on a busy backdrop | `BG_TAB_HOVER` fill plus full-opacity primary label; selected tab also has the 13px cue and `BG_TAB_SELECTED` luminance step; close hover applies the parent fill; verified in headless tests and temporary Xvfb checks |
| Mint accent is confused with semantic success/status | Status meaning could blur | Success is a cooler teal and every status also uses a shape and text; accent is reserved for identity/action/focus/active cues |
| Close-scale or splitter-fade motion is the only cue | Reduced-motion or screenshot users miss state | Static `ERROR` tone/parent fill and cursor/tooltip/hit area carry the same meaning; motion is supplementary |
| Motion causes layout jank | UI feels less fluid | 150 ms width transition, clipped content, no reflow, intermediate-frame test; native smoothness pending |
| Visual regression from future UI changes | Contract drift | Token/contrast/layout/component checks in `ui_model`; two-size Xvfb checks (not stored); [`design.md`](design.md) as reference |

## Migration and rollback

There is no data migration and no persisted UI schema. This change is a contract split, not a new product version. Existing checked implementation tasks moved from the core change retain their Linux evidence; native/manual checkpoints remain unchecked. If the UI extraction proves wrong, the visual contract can be re-merged into the core change without changing runtime behavior, user state, or persisted data. No archive or version bump is part of this change.
