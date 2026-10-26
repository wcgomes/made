# Design: made-ui-ux-v1

## Approach

This change extracts the UI/UX/visual contract that was embedded in the active v1 bundle into a dedicated active change. The core change remains the owner of product, runtime, integration, security, and performance behavior; this design owns the durable visual and interaction rationale and the implementation direction for the native Calm Editor shell.

The implemented stack is one native eframe/egui 0.37 window with no webview. The terminal grid is painted by a dedicated renderer; egui composes the workspace strip, optional session map, explorer, full-left preview, picker, diagnostics, tooltips, and toast. The visual language is dark, quiet, terminal-first, and intentionally narrow: one amber accent, layered shortcuts, and no persistent technical chrome.

Alternatives considered:

| Alternative | Why rejected |
|---|---|
| Keep the full UI contract inside the core change | Mixed product/runtime guarantees with visual presentation, made the core delta hard to review, and duplicated UI detail across spec, design, and a separate UI reference document |
| Treat UI as post-delivery polish | The v1 exit test explicitly includes Calm Editor quality, fluidity, responsive layout, and accessibility; these are acceptance conditions, not decoration |
| Adopt a webview or per-OS UI fork | Violates the no-webview/lightness goal and the one-source-tree rule; the platform layer owns OS capabilities only |
| Introduce a component library or design system package | The implemented UI is a small native surface; a new abstraction would add ceremony without solving a current coupling problem |
| Add animation everywhere to signal quality | Motion is limited to spinners and the explorer transition; animation must not carry essential information or animate tab widths |

This document is the durable visual reference. It is non-normative; the observable guarantees are in [`spec.md`](spec.md). The core implementation design is [`../opencode-session-follow-v1/design.md`](../opencode-session-follow-v1/design.md).

## Significant decisions

### U1: Split UI from runtime behavior

The core change keeps launch resolution, terminal fidelity, tab lifecycle, root arbitration, session follow, bridge/config security, preview safety, watcher behavior, diagnostics data, and performance budgets. This change keeps layout, chrome, wordmark, tabs/session-map presentation, picker interaction, preview/diagnostics presentation, glass/backdrop, motion, responsive behavior, keyboard focus, and accessibility presentation.

Why: the two concerns change for different reasons. A terminal PTY fix should not rewrite a visual contract; a wordmark or spacing change should not reopen root-precedence scenarios. The split also gives each active change a truthful task/evidence owner.

Consequence: some requirements have a deliberate seam. The core spec states that a preview is read-only, keeps the terminal process alive, and rejects stale roots; this spec states that the preview occupies the full left pane and offers the documented return actions. Reviewers must check both, but neither spec duplicates the other's guarantee.

### U2: Calm Editor as the visual source of truth

The implemented theme is retained rather than redesigned. The palette is dark with one amber accent; semantic green/amber/red/blue are reserved for status. Depth comes from layered surfaces, 1px borders, 8–10px gaps, and one soft painted shadow on the glass band/explorer card, not gradients or glows. Terminal and all reading surfaces stay opaque and high-contrast.

Why: the terminal is the hero. Chrome that competes with terminal content reduces legibility and makes the product feel like a dashboard rather than a workspace. A single accent keeps selection, focus, and primary action coherent.

### U3: In-app glass is the portable fallback; native backdrop is optional

The in-app glass approximation is always present: translucent tonal fills, a painted soft shadow, and a 1px top highlight on the top band and explorer card/header. On macOS, `made_platform::native_backdrop` may request `NSVisualEffectView` HudWindow vibrancy; on Windows 11 it may request Mica only. Linux, unsupported compositors, `MADE_NATIVE_BACKDROP=off`, and failures keep `InAppGlass` with `applied=false` and an honest reason. The window clear alpha drops from 190/255 to 150/255 only when the native effect was applied.

Why: native blur is compositor-dependent and cannot be promised from a compile check. The portable fallback keeps the UI readable and allows the native effect to improve depth where it is validated. Windows acrylic/blur are excluded because `window-vibrancy` documents resize/drag lag.

### U4: Quiet chrome, contextual failures

The header carries only the wordmark, workspace tabs, and icon-only actions. There is no routine bottom status bar, shell-running pill, path row, version row, watcher banner, degraded banner, empty session strip, or empty preview placeholder. Technical detail lives in Diagnostics. Genuine shell/file/user-action errors stay contextual and non-blocking.

Why: persistent technical chrome consumes terminal height and turns normal optional integration into apparent failure. Icon-only actions reduce width pressure; tooltips, accessible names, and visible focus preserve discoverability.

### U5: Full-height explorer, full-left preview

The explorer owns the full right content column and never shares that column with a preview. A file opens as a read-only surface across the entire left content pane, replacing the terminal visually while its PTY keeps running. Closing/returning restores the same terminal and focus. The explorer is hidden/restored from the header and resized only by a pointer-drag divider; it has no Wider/Narrower/Reload/Hide text actions.

Why: a full-height tree is easier to scan, a full-left preview is easier to read, and a single left-surface choice keeps focus/input ownership unambiguous. A pointer-only divider removes permanent chrome while hide/restore remains keyboard reachable.

### U6: Keyboard and accessibility are acceptance criteria

Every persistent control is keyboard reachable with visible focus. Icon actions expose tooltip/accessibility-name metadata. The pointer-only divider is intentionally not a keyboard stop. Status uses shape plus text plus color. Targets are at least 24×24 and text never falls below 10px. Platform screen-reader exposure is not claimed while accesskit is absent.

Why: the visual contract cannot pass on appearance alone. A keyboard user must be able to open folders, switch/close workspaces, hide/restore the explorer, open diagnostics, operate the picker, return from preview, and navigate the tree. The accesskit caveat is stated rather than hidden.

## Flow

```
Window
├─ Top band: MADE wordmark | workspace tabs | icon actions (Open folder, Explorer, Diagnostics, Sessions?)
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
5. **Explorer resize:** pointer drag only, clamped to leave usable left/right widths; hover reveals a thin line/grip, rest paints nothing.
6. **Session map:** optional, scoped to the selected workspace, initially hidden; activity indicators update without converting sessions into workspace tabs.
7. **Diagnostics:** opens as an opaque floating card over the workspace; copying diagnostics/log paths never includes workspace credentials.

## Design tokens

### Colors

| Token | Value | Use |
|---|---|---|
| `BG_CLEAR` | `rgba(10,11,14,190)` | Default window clear/backdrop |
| `BG_CLEAR_NATIVE` | `rgba(10,11,14,150)` | Clear when a native backdrop is applied |
| `BG_CHROME` | `#14161B` | Opaque chrome fallback, focus gap |
| `BG_CARD` | `#171A20` | Preview/diagnostics cards and opaque fallback |
| `BG_HEADER` | `#14171C` | Opaque session strip/non-glass headers |
| `BG_DEEP` | `#12151A` | Diagnostics log, preview gutter |
| `BG_TERMINAL` | `#0B0D10` | Terminal canvas/card |
| `BG_ELEVATED` | `#1B1F26` | Secondary buttons, chips, tooltips |
| `BG_HOVER` | `#1D2129` | Row/button hover |
| `BG_SELECTED` | `#222A35` | Selected tab/row |
| `BG_CHROME_GLASS` | `rgba(20,22,27,205)` | Frosted top chrome band |
| `BG_CARD_GLASS` | `rgba(23,26,32,202)` | Frosted explorer card |
| `BG_HEADER_GLASS` | `rgba(20,23,28,208)` | Frosted explorer header |
| `GLASS_HIGHLIGHT` | `rgba(255,255,255,12)` | 1px inner top highlight |
| `GLASS_SHADOW` | `rgba(0,0,0,58)` | Painted soft glass shadow |
| `BORDER` | `#262B34` | Card/band borders; decorative separators |
| `BORDER_HEADER` | `#20252D` | Header dividers inside cards |
| `BORDER_CONTROL` | `#5E697B` | Secondary control boundary |
| `BORDER_CONTROL_HOVER` | `#7A8494` | Hover boundary |
| `TEXT_PRIMARY` | `#E6E8EC` | Titles, active values |
| `TEXT_SECONDARY` | `#BCC2CC` | Body, inactive tab titles |
| `TEXT_TERTIARY` | `#9AA2B0` | Section labels, paths, metadata |
| `TEXT_DISABLED` | `#7A8494` | Disabled only |
| `ACCENT` | `#E9A76A` | Active tab underline, primary action, focus accent |
| `ACCENT_HOVER` | `#F2B87F` | Accent hover |
| `ACCENT_PRESSED` | `#D89355` | Accent pressed |
| `ACCENT_TEXT` | `#14100B` | Text on accent |
| `ACCENT_TINT` | `#302B29` | Accent-tinted inset surfaces |
| `SUCCESS` | `#7CC79A` | Connected, running, read-only |
| `SUCCESS_TINT` | `#232F2F` | Success/read-only pill |
| `WARNING` | `#E9B872` | Degraded, busy, attention, exited |
| `WARNING_TINT` | `#302D2A` | Warning banner/pill |
| `ERROR` | `#E58A8A` | Error, unsupported, failed |
| `ERROR_TINT` | `#30272D` | Error banner/pill |
| `INFO` | `#8FB8E8` | Unread, informational |
| `INFO_TINT` | `#252D38` | Unread badge |
| `FOCUS` | `#F4C48E` | Focus ring |
| `SCROLL_TRACK` | `#14171D` | Terminal scrollbar track |
| `SCROLL_THUMB` | `#626C7D` | Terminal scrollbar thumb |
| `SCROLL_THUMB_HOVER` | `#7A8494` | Scrollbar hover |

Tints are precomposited opaque colors, not alpha fills. The glass band/explorer surfaces are the only alpha fills. `BORDER` is never the sole boundary of an interactive control; use `BORDER_CONTROL` or accent.

### Alpha and glass layering

| Surface | Alpha |
|---|---:|
| Window backdrop/gutter, no native effect | `190/255` |
| Window backdrop/gutter, native effect applied | `150/255` |
| Top chrome band | `205/255` |
| Explorer card | `202/255` |
| Explorer card header | `208/255` |
| Terminal, preview, picker, diagnostics, tooltip, toast | `255` |

`MADE_NATIVE_BACKDROP=auto` (default, including unset/unrecognized) attempts the native effect; `off` skips it. macOS requests HudWindow vibrancy; Windows 11 requests Mica only. Linux, unsupported compositors and failures use the in-app glass. The native macOS/Windows effect is compile-checked but not runtime-validated in this repository; no live-blur claim is made for those targets. If the compositor ignores alpha, the RGB clear renders as a solid dark backdrop and the UI is otherwise unchanged. `TRANSLUCENT_WINDOW` is the kill switch; `false` requests no transparency and uses opaque clear.

### Typography

Bundled egui fonts only: proportional Ubuntu-Light, monospace Hack. Ubuntu-Light has one weight; `.strong()` changes color, not weight.

| Role | Size / line | Family | Color |
|---|---:|---|---|
| Brand wordmark | 14 / 18, tracking 1.8 | monospace | `ACCENT` M + `TEXT_PRIMARY` ADE |
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
| Gap between cards/bands | 8 | 6 |
| Workspace strip/essential controls | 40 | 36 |
| Card radius | 10 | 8 |
| Card header | 34 | 30 |
| Header horizontal padding | 12 | 10 |
| Card content padding | 12 | 10 |
| Optional session strip | 36 | 34 |
| Tab height | 28 | 26 |
| Tree row | 24 | 24 |
| Buttons | 26 ghost / 28 primary-secondary | 24 / 26 |
| Status pill | 20 | 18 |
| Badge | 14 | 14 |
| Right explorer default | 440 | 288, constrained by usable left width |
| Explorer height | full content | full content |
| Tree indent | 14/depth | 14 |
| Preview code line | 18 | 18 |

Spacing scale: `2, 4, 6, 8, 10, 12, 16, 20, 24, 32`. Global spacing: `item_spacing(8,6)`, `button_padding(10,4)`, `interact_size(28,26)`, `window_margin 14`, `menu_margin 6`, `indent 14`, `tooltip_width 320`, `animation_time 0.12`. Scrollbars: solid, 8px, min handle 24.

Shadows: floating overlays plus the glass approximation. Terminal/preview cards have none.

```text
SHADOW_FLOAT  = offset [0,12], blur 32, color black 120/255
SHADOW_POPUP  = offset [0,6],  blur 18, color black 100/255
SHADOW_TOAST  = offset [0,4],  blur 14, color black 90/255
GLASS_SHADOW  = offset [0,2],  expand 1px, color black 58/255
```

### Interaction states

| Element | Rest | Hover | Selected/Active | Focus | Disabled |
|---|---|---|---|---|---|
| Ghost button | transparent, tertiary | `BG_HOVER`, primary | `BG_SELECTED` + `BORDER_CONTROL`, primary | 2px `FOCUS` double ring | `TEXT_DISABLED` |
| Secondary button | `BG_ELEVATED`, 1px `BORDER_CONTROL` | `BG_HOVER` + hover border | `BG_SELECTED` + `BORDER_CONTROL` | 2px `FOCUS` ring | `BG_CARD`, `BORDER`, disabled |
| Primary button | `ACCENT`, `ACCENT_TEXT` | `ACCENT_HOVER` | `ACCENT_PRESSED` | `FOCUS` ring | muted accent |
| Tab | transparent, secondary | `BG_HOVER` | `BG_CARD` + 2px accent underline; no dot/hollow/border | 2px `FOCUS` ring | n/a |
| Tree row | transparent | `BG_HOVER` | `BG_SELECTED` + 2px accent left bar | 1.5px inside ring | n/a |
| Picker row | transparent | `BG_HOVER` | one rounded `BG_SELECTED`; no left accent bar | 1.5px inside ring | n/a |
| Glass band/card | glass fill, 1px border, shadow/highlight | no fill change | n/a | n/a | n/a |
| Card | `BG_CARD`, 1px border | no change | n/a | n/a | n/a |
| Diagnostics pill | tone tint + tone text | no hover | n/a | n/a | n/a |

Focus ring: 2px surface-colored gap at `rect.expand(1.0)`, then 2px `FOCUS` at `rect.expand(3.0)`, `StrokeKind::Outside`. Inside clipped scroll areas use a 1.5px inside ring.

## Layout

```
root Frame::NONE, inner margin = gutter, no fill
├─ Panel::top  workspace tabs + icon actions (40/36)
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
| Top bar | 1260×40 at `(10,10)` |
| Gap | 8 |
| Main content | 1260×732 at `y=58` |
| Right column | 448 total: 8 gap + 440 card at `x=822` |
| Left terminal OR preview | 812×732 at `(10,58)` |
| Full-height explorer | 440×732 at `(830,58)` |
| Explorer hidden | left surface 1260×732 |

No empty session strip, terminal path row, shell-running pill, preview placeholder, or bottom status bar. Preview owns the full left region. Terminal column/row count derives from measured font/grid metrics.

### 720×480

Content after 8px gutter: 704×464.

| Region | Geometry |
|---|---|
| Top bar | 704×36 at `(8,8)` |
| Gap | 6 |
| Main content | 704×422 at `y=50` |
| Right column | 294 total: 6 gap + 288 tree at `x=418` |
| Left terminal OR preview | 410×422 at `(8,50)` |
| Full-height explorer | 288×422 at `(424,50)` |
| Explorer hidden | left surface 704×422 |

Keep open-folder, tab close/switch, Explorer toggle, and Diagnostics reachable without clipping; optional controls may use a labeled overflow. Preview states and Close/return remain usable at compact size.

### Responsive rules

| Width | Behavior |
|---|---|
| ≥1100 | Folder context may show truncated path; explorer defaults to 440 |
| 900–1099 | Truncated folder context; explorer default 35% within usable bounds |
| <900 | Compact labels, gutter 8, top 36, radius 8; explorer default 288 with accessible hide/resize |
| <820 | Optional Sessions may move to labeled overflow; workspace/Explorer/Diagnostics remain reachable |
| Height <600 | Hide preview meta; picker list max 160; diagnostics clamps |

No control may vanish through clipping. Workspace/session overflow lists all of their own tabs; truncate labels with `…` and tooltip full values. Splitter width/visibility and widget ids are per workspace and clamp again when the window shrinks.

## Components

### Workspace strip and header

- `Panel::top("made-header")`, exact 40/36, no fill, no separator.
- Band: `BG_CHROME_GLASS` 205/255, 1px `BORDER`, radius 8/7, painted `GLASS_SHADOW` and 1px top `GLASS_HIGHLIGHT`, padding 12×7 / 10×5, content 26px. This is the in-app glass fill; optional native material may layer behind it.
- Left: cached two-tone monospace `MADE` wordmark (accent M, primary ADE, 14px, 1.8px tracking, tooltip “Minimal Agentic Development Environment”), a 1px divider, then editor-like workspace tabs. Tabs have folder labels, full-path tooltips, reachable close X, full-opacity `BG_CARD` + 2px accent underline when active, no dot/hollow marker or active border, stable workspace-scoped widget ids, and an overflow list when needed.
- Right: icon-only Explorer toggle, Diagnostics, and optional Sessions; no persistent text labels. Each icon is an app-drawn monochrome glyph with a 24×24 minimum target, matching tooltip, accessible name (`Response::widget_info`), and visible focus ring. Optional Sessions is visually distinct from workspace tabs. Compact overflow is labeled and keyboard reachable.
- Ghost icon button: 26/24 high, minimum 24 wide, radius 6; selected `BG_SELECTED` + `BORDER_CONTROL`; hover `BG_HOVER`; tooltip is the action name (dynamic Hide/Show for Explorer).
- No version in the top bar; version is in the picker and diagnostics.

### Optional session map

Initially hidden, scoped to the selected workspace, visually separate from workspace tabs. Opening it reveals session data and focus behavior; normal shell use reserves no strip. It does not create another terminal or convert sessions into workspace tabs.

- Strip 36/34, `BG_HEADER`, top corners only, bottom 1px `BORDER_HEADER`, padding 4/3.
- Tabs 28/26 high, radius 7/6, gap 4, width `clamp(112, measured + 26, 220)` (compact max 184).
- Content: title 12.5 proportional, busy spinner, attention badge, unread badge, `*` for renaming.
- States: inactive transparent/secondary; hover `BG_HOVER`/primary; selected `BG_ELEVATED`; active `BG_CARD` + 2px accent underline inset 8px; no dot/hollow marker or active border. Focus double ring.
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
- Palette: background `#0B0D10`, foreground `#D7DDE7`, cursor `#F2B87F`, selection `#2E3A4E`/`#FFFFFF`; keep ANSI 1–7/9–15; raise ANSI 8 to `#707B8E` (4.55:1). Scrollbar track/thumb/hover as tokens.

### Explorer card

- Fill `BG_CARD_GLASS` 202/255, 1px `BORDER`, radius 10/8, painted `GLASS_SHADOW`; optional native material behind the window.
- Full right content height, no preview beneath. Header 34/30, `BG_HEADER_GLASS` 208/255 with 1px `GLASS_HIGHLIGHT`, padding 12/10, bottom divider: “EXPLORER” and root folder 11 mono tertiary with full-path tooltip. No Wider, Narrower, Reload, or Hide text actions. Watcher/live/starting state lives in Diagnostics.
- Hide/show animates panel width over about 150 ms quadratic ease-out. Lay out the tree at its stored width and clip to the sliding panel; reverse cleanly; settle at stored width; stop tree/splitter input once closed. The left surface receives reclaimed space without losing shell/root/selection/preview state.
- Divider: pointer-drag only, clamped to usable left/tree widths. Transparent at rest: no permanent line/grip. Hover/drag reveals 1px line and 6×32 grip (accent while dragging) over a comfortable hit area; hover alone does not change stored width. Not a keyboard stop; header Explorer toggle remains the keyboard path. Width/visibility/expansion/valid selection survive tab switching and hide/restore, not restart.
- Body: 4px vertical / 8px horizontal padding; vertical scroll.
- Row 24px min, indent 14/depth; hover `BG_HOVER` radius 4; selected `BG_SELECTED` + 2px accent left bar; focus 1.5px inside `FOCUS`.
- Glyphs: `▸/▾` directory, `·` file, `@` symlink, `?` other, `!` error. Name 12.5 proportional; single-line ellipsis; error rows `ERROR` + tooltip.
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
- Toolbar 40/36: Up/Refresh ghost 24; path 10.5 mono secondary, middle-truncated 200 chars, full tooltip; divider.
- List max height `(available_h - 280).clamp(120, 320)` (compact 160); rows 32px; `▸` 10 mono tertiary + name 12.5 secondary; hover `BG_HOVER`; selected uses one rounded `BG_SELECTED` background + primary text, no left accent bar.
- Footer 52/48: right-aligned “Open selected folder” secondary 28, “Use this folder” primary 28 (only accent-filled control).
- Error: inline `ERROR_TINT` strip above footer, 10.5 error, tooltip full.
- Hint below card 10.5 tertiary: “Double-click a folder to enter it. Enter opens the selected folder.”
- Place card at `max(24, (available_h - 600)/3)` top padding; never stretch the list to fill the window.
- No-arg startup opens the home workspace directly; picker is explicit via `--picker`, Open folder, or final close. Explicit path wins over `--picker`. Picker starts no process until confirmation. Escape/Close return to the live workspace when present, otherwise default home/terminal fallback; Back remains for add-workspace; confirmation opens a new tab without stopping the existing shell.

### Shared controls

- Ghost 26/24 high; secondary 28/26 with `BORDER_CONTROL`; primary 28/26 accent; warning action warning tint/border.
- Diagnostics status pill 20/18; optional session unread badge 14 high, min 18 wide; preview read-only label is contextual, not routine status.
- Interactive targets ≥24×24. Card titles are section labels, not headings.

## Transparency and native backdrop policy

- The window clear (`BG_CLEAR` 190/255, or `BG_CLEAR_NATIVE` 150/255 when native) and the top band/explorer card/header carry alpha.
- The band/explorer surfaces are the in-app frosted-glass approximation: translucent tonal fill + painted `GLASS_SHADOW` + 1px `GLASS_HIGHLIGHT`. This is portable and readable when alpha is ignored.
- `made_platform::native_backdrop` optionally requests a native material once on the first UI frame: macOS `NSVisualEffectView` HudWindow (active), Windows 11 Mica only. Linux, unsupported compositors, `MADE_NATIVE_BACKDROP=off`, and failures keep `InAppGlass` with `applied=false` and an honest reason. Windows acrylic/blur are not used.
- Native macOS/Windows rendering is compile-checked only; no runtime native-effect/blur claim is made. Linux Xvfb confirms the fallback.
- Terminal, preview, picker, diagnostics, tooltips, and toast are opaque reading surfaces. Hover/selection/tints are opaque precomposited colors.
- Do not use `ui.set_opacity` above the terminal or place alpha on terminal/preview/picker text.
- Kill switch: `TRANSLUCENT_WINDOW: bool = true`; when false, no transparent request and alpha 1.0. The UI has no OS branches; platform-specific backdrop selection stays in `made-platform`.

## Motion

- Spinners, standard control animation, and the explorer hide/show width transition (~150 ms quadratic ease-out) are the only motion.
- The explorer lays out its content at the stored width and clips to the sliding panel; it does not reflow during the transition, reverses cleanly, and stops tree/splitter input once closed.
- Tabs do not animate width. Motion never carries essential status/selection/error information.
- Native animation smoothness and native pointer feedback remain manual validation gaps; Xvfb/headless checks cover the intermediate frame, clipping, reversal, and closed input behavior.

## Accessibility

Contrast (opaque reading surfaces):

| Pair | Ratio |
|---|---:|
| Primary / card | 14.21:1 |
| Secondary / card | 9.73:1 |
| Tertiary / card | 6.78:1 |
| Tertiary / chrome | 6.90:1 |
| Accent / card | 8.46:1 |
| Accent text / accent | 9.20:1 |
| Success / success tint | 6.91:1 |
| Warning / warning tint | 7.54:1 |
| Error / error tint | 5.73:1 |
| Info / info tint | 6.74:1 |
| Focus / card | 10.90:1 |
| Terminal fg / bg | 14.15:1 |
| Terminal selection | 11.47:1 |
| ANSI red / terminal | 4.76:1 |
| ANSI bright black adjusted / terminal | 4.55:1 |
| Control border / card | 3.14:1 |

- `BORDER` card edges are decorative; never the sole control boundary.
- Glass surfaces composite over the dark backdrop. The ratios cover opaque reading surfaces; native backdrop rendering, native blur, and glass text contrast are not separately certified.
- ANSI 0 is a terminal background slot and cannot be made readable as foreground. Arbitrary ANSI/truecolor terminal content is outside app control; the default terminal pair meets AA.
- Focus: every keyboard-reachable interactive control has a visible focus state; never rely on fill change alone. Custom tabs/ghost buttons use the double ring; tree selected rows use an inside ring. The pointer-only divider is not focusable and uses hover/drag feedback.
- Keyboard: labeled workspace open/switch/close, icon-only Explorer/Diagnostics (tooltip/accessibility names), optional Sessions; workspace/session tabs (Tab in, Left/Right, Enter/Space); terminal as one focus stop; explorer tree (arrows, Enter, Left/Right); preview actions/return; diagnostics normal egui focus. The resize divider is pointer-drag only and not a keyboard stop; the header Explorer toggle is the keyboard hide/restore path. Only the selected visible surface receives input, with visible focus after every transition.
- Colorblind-safe: status = shape + text + color (`●`, `!`, `×`); active tab = full-opacity background + underline (no dot/hollow); busy = spinner; attention = `!`; unread = number; renaming = `*`; shell = running/exited/error text; tree errors = `!`.
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

- `crates/made-app/src/theme.rs`: tokens, typography, glass painting, focus rings, icon primitive, wordmark galley, clear color/kill switch.
- `crates/made-app/src/ui.rs`: workspace strip, header actions, tabs/overflow, picker composition, layout, explorer animation.
- `crates/made-app/src/explorer_view.rs`: full-height tree, hover-only divider, full-left preview.
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
| Motion causes layout jank | UI feels less fluid | 150 ms width transition, clipped content, no reflow, intermediate-frame test; native smoothness pending |
| Visual regression from future UI changes | Contract drift | Token/contrast/layout/component checks in `ui_model`; two-size captures; [`design.md`](design.md) as reference |

## Migration and rollback

There is no data migration and no persisted UI schema. This change is a contract split, not a new product version. Existing checked implementation tasks moved from the core change retain their Linux evidence; native/manual checkpoints remain unchecked. If the UI extraction proves wrong, the visual contract can be re-merged into the core change without changing runtime behavior, user state, or persisted data. No archive or version bump is part of this change.
