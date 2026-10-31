# Made UI UX Specification

## Purpose

This specification owns the UI/UX/visual/presentation contract for v1. Product/runtime/integration/security/performance behavior is owned by the core specification (`specs/opencode-session-follow.md`). The UI is implemented on Linux with 654 passing tests and Xvfb/headless evidence; macOS arm64 HudWindow vibrancy and UI rendering were manually verified natively on 2026-10-28 (windowed, resized, and fullscreen). Native UX and native contrast certification are not current requirements; Windows, macOS x64, native Linux desktop, native pointer/keyboard, and hardware-GPU evidence remain unverified.

## Requirements

### Requirement: Calm Editor Visual System

Made MUST use the implemented Calm Editor visual system for the native shell: dark layered surfaces, one calm mint-green accent, bundled Ubuntu-Light proportional and Hack monospace typography, a 4/8-point spacing rhythm, 8–10px card radii and gutters, 1px borders, and opaque preview, picker, diagnostics, tooltip, toast, and other non-terminal reading surfaces. The accent MUST be `ACCENT #8BD5A7`, with `ACCENT_HOVER #A3E3BA`, `ACCENT_PRESSED #72BF8E`, `ACCENT_TEXT #08140D`, `ACCENT_TINT #26352C`, `FOCUS #B8F0C9`; it applies to the wordmark `M`, primary accent actions, focus rings, busy spinners, and the explorer splitter while dragging, but MUST NOT be used as a workspace-tab outline. The terminal cursor MUST use the neutral terminal foreground `#D7DDE7` rather than the accent; when the pane is focused and the terminal reports a blinking cursor style it MUST blink with a ~530 ms on/off phase, while a steady reported style or an unfocused pane MUST keep the cursor steady and visible. The no-native/fallback clear color MUST use `BG_CLEAR` alpha 215/255, while an actually applied native backdrop keeps `BG_CLEAR_NATIVE` alpha 135/255. Workspace tabs use opaque `BG_TAB_SELECTED #1B231F` and `BG_TAB_HOVER #151C18` fills over the translucent black `BG_TAB_IDLE`; only the selected tab paints a 1px inside stroke, including when that selected tab is hovered: the dark tab hue darkened by 12 at alpha 128, the same strength as the previous lighter step but not a light edge, and not `BORDER_CONTROL` or another contrasting gray. Inactive tabs paint no stroke at rest and no stroke on hover. That stroke is not `ACCENT`, `FOCUS`, or a selection cue. Card borders remain neutral and the tab close hover remains `ERROR`. The full-height explorer card/header MUST use the documented conditional glass: when a native backdrop was actually applied (macOS HudWindow vibrancy or Windows 11 Mica), they use `BG_CARD_GLASS_NATIVE` 24/255 and `BG_HEADER_GLASS_NATIVE` 32/255 so the native material reads through; when no native backdrop is applied (disabled, unsupported, or failed), they use the darker fallback `BG_CARD_GLASS` 188/255 and `BG_HEADER_GLASS` 194/255 for legibility without blur. Windows MUST use the same applied/fallback selection. The painted soft shadow and 1px top highlight remain. The terminal card fill MUST use `BG_TERMINAL_GLASS` alpha 160/255 over the `BG_TERMINAL` RGB (11/13/16) only while the terminal is on its primary screen (no alternate-screen TUI), and it MUST be the sole glass layer: the terminal body and terminal-owned path-line/file-slot chrome MUST NOT paint another translucent fill. When the terminal is in alternate-screen mode (the VT `ALT_SCREEN` signal used by OpenCode, vim, and less), the terminal card fill, terminal-owned path-line/file-slot chrome, and body MUST use fully opaque `BG_TERMINAL` 255/255 so TUI cells remain on an opaque surface. The optional session strip/header remains opaque `BG_HEADER`. Preview, picker, diagnostics, tooltip, and toast remain opaque. The workspace header MUST float directly over the window clear or native backdrop with no persistent band fill, border, shadow, or top highlight; the wordmark, workspace tabs, and icon actions are painted directly on that backdrop. A native macOS/Windows backdrop MAY layer behind the window only through the platform boundary; when unavailable, disabled, or unsupported, the in-app explorer glass MUST remain and the reason MUST be honest. Except for the documented primary-screen terminal glass, the UI MUST NOT introduce gradients or glows, multiple accents, accent-colored status, opacity on reading surfaces, or text below 10px. The no-webview runtime constraint is owned by the core Responsiveness and Lightness requirement.

#### Scenario: Calm Editor tokens and typography

- Given the running Made UI
- When its visual tokens are inspected
- Then the documented Calm Editor colors including the mint-green accent/focus tokens and the neutral terminal cursor, the fallback clear alpha 215/255 with native 135/255, the explorer card/header fallback glass alphas 188/255 and 194/255 with native tints 24/255 and 32/255 selected by the actual applied state, the `BG_TERMINAL_GLASS` 160/255 primary-screen terminal alpha with opaque `BG_TERMINAL` 255/255 in alternate-screen mode, the opaque dark green-gray `BG_TAB_SELECTED`/`BG_TAB_HOVER` tab fills, typography roles, spacing, radii, target sizes, and one-accent rule are used, semantic success/warning/error/status tones remain visually distinguishable and shape/text-backed, neutral card borders are unchanged, and preview, picker, diagnostics, tooltip, and toast remain opaque

#### Scenario: Terminal cursor color and blink

- Given a focused terminal pane whose terminal reports a blinking cursor style, then a steady style, then a pane that loses focus, and then that same pane regaining focus
- When the cursor is rendered while idle, while the user types, and after focus is regained
- Then the cursor uses the neutral terminal foreground `#D7DDE7` rather than the mint accent; while focused with the reported blinking style it alternates about 530 ms on and off, resets to visible on input or focus gain, and schedules the next repaint at the phase boundary instead of repainting continuously; with a steady reported style or without focus it stays steady and visible; and the existing block, hollow block, underline, and bar shapes are unchanged

#### Scenario: Explorer glass and transparent header

- Given a workspace with the floating header and explorer visible
- When the UI is rendered
- Then the explorer card and explorer header use the native 24/255 and 32/255 tints when a native backdrop is applied and the fallback 188/255 and 194/255 fills otherwise, with the documented shadow and top highlight; the header controls float directly over the clear/native backdrop without a persistent band, the terminal follows the conditional `BG_TERMINAL_GLASS`/opaque `BG_TERMINAL` rule, and preview, picker, diagnostics, tooltips, and toast remain opaque and readable when alpha is ignored

#### Scenario: Conditional Explorer glass

- Given an actually applied native backdrop (macOS HudWindow or Windows 11 Mica), then `MADE_NATIVE_BACKDROP=off`, an unsupported compositor, or a failed request
- When the Explorer card and header render
- Then with the native backdrop applied they use `BG_CARD_GLASS_NATIVE` 24/255 and `BG_HEADER_GLASS_NATIVE` 32/255; otherwise they use `BG_CARD_GLASS` 188/255 and `BG_HEADER_GLASS` 194/255; the choice follows the actual applied status rather than the platform name, Windows uses the same selection, and the border, soft shadow, and 1px top highlight remain unchanged

#### Scenario: Explorer fallback glass readability

- Given no native backdrop (disabled, unsupported, or failed) and a bright desktop behind the translucent window
- When the Explorer card and header render
- Then they use the darker 188/255 and 194/255 fallback glass rather than the native 24/32 tints, primary and secondary Explorer text remain legible without blur, the fallback legibility check covers a bright composited desktop, and native glass text contrast is not claimed

#### Scenario: Conditional terminal glass

- Given a terminal on its primary screen, then a full-screen TUI that enters the alternate screen (for example OpenCode, vim, or less), then returns to the primary screen
- When the terminal surface is rendered
- Then while the primary screen is active the terminal card fill uses `BG_TERMINAL_GLASS` alpha 160/255 over the `BG_TERMINAL` RGB (11/13/16) and is the only glass layer: the terminal body and terminal-owned path-line/file-slot chrome paint no second translucent fill; while the terminal reports the VT `ALT_SCREEN` mode, the terminal card fill, path-line/file-slot chrome, and body use fully opaque `BG_TERMINAL` 255/255; the optional session strip/header remains opaque `BG_HEADER`; the surface follows that mode signal rather than title/content heuristics and returns to the single glass layer when the alternate screen exits; and preview, picker, diagnostics, tooltip, and toast remain opaque

#### Scenario: Floating header controls

- Given a workspace header over the in-app clear or an applied native backdrop
- When the header is rendered at 1280×800 or 720×480
- Then no persistent band fill, border, shadow, or top highlight is painted behind the wordmark, tabs, or icon actions; the 26/24 strip places the card content at 42/36 with one 8/6 pane gap above the strip and one 8/6 gap below it; the header row spans the card left/right edges with no extra horizontal padding; the last right-side icon’s 26/24 target, selected/hover fill, and focus ring all end at the card right edge with no gutter overhang, and its 14px glyph remains centered with normal optical padding on both sides; and header controls share one vertical center with square hit targets and vertically centered glyphs

#### Scenario: Native backdrop auto and fallback

- Given `MADE_NATIVE_BACKDROP=auto`, `off`, or an unsupported platform/compositor
- When the first UI frame requests or skips the native material
- Then the UI uses native clear alpha 135/255 and the native Explorer tints 24/255 and 32/255 only when an effect was actually applied; otherwise it uses the darker fallback clear alpha 215/255 and fallback Explorer glass 188/255 and 194/255 with `applied=false` and an honest reason; Windows uses the same applied/fallback selection; no native effect is claimed without runtime evidence

#### Scenario: Reading-surface contrast

- Given primary, secondary, tertiary, accent, status, focus, terminal, and control-boundary text and surfaces
- When contrast is measured against the documented pairs
- Then the documented ratios hold for opaque reading surfaces, the fallback Explorer glass is checked for legibility over a bright composited desktop, the native Explorer tints and primary-screen terminal glass are not claimed to meet a separate text-contrast ratio without evidence, decorative borders are not the sole control boundary, and native glass/blur contrast is not claimed without evidence

### Requirement: Minimal Workspace Layout

Made MUST present a terminal-first workspace in one native window: workspace tabs above the content area, a selected left surface that is either the terminal or the full-left read-only preview, and a full-height explorer in the right column. Persistent header actions MUST be icon-only buttons with tooltips and accessible labels: Open folder, Explorer toggle, Diagnostics, and the optional Sessions action when shown. The Open folder action MUST live in the workspace tab strip immediately after the last visible tab and before the overflow trigger, use an app-drawn `+` glyph, reserve its width in the tab-fitting budget, and remain reachable with the overflow trigger when tabs overflow. The remaining right icon group MUST order Sessions (when shown), Diagnostics, Explorer left-to-right so Explorer is the rightmost icon. The top header MUST NOT show text labels for these actions; the Made identity and workspace-tab folder labels remain text, and workspace tabs MUST retain a close affordance. The header controls MUST float directly over the window clear/native backdrop and MUST share one vertical center and square hit targets, with vertically centered glyphs and the last icon’s glyph centered in its target. Workspace tabs MUST start at the window gutter/card left edge. The header control row MUST span the window gutter to `width - gutter` with no extra horizontal padding, so the last right-side icon’s 26/24 target, selected/hover fill, and focus ring end at the explorer/terminal card right edge with no gutter overhang. The 14px glyph MUST remain centered inside its target with normal optical padding on both sides, so its visible edge is inset from the card border. The wordmark MUST sit to the right of the tab strip immediately before the right icon group. The tab-fitting budget MUST reserve a 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark; with few tabs the visible flexible space between the last visible control and the wordmark is larger because the boundary is reserved, not measured from the last control. The wordmark-to-first-right-icon gap MUST be exactly 24px wide / 18px compact. The whole right cluster MUST be reserved so tabs, `+`, and overflow never overlap it, and the wordmark MUST NOT paint a tab fill, hover, or selection. The top workspace strip and workspace tabs MUST both be 26/24 high while session-map metrics remain unchanged. Header icon actions, optional session tabs, explorer rows and picker rows share one interaction system: hover uses a `BG_HOVER` filled rounded rect with radius 6 and no border; selection uses a `BG_SELECTED` filled rounded rect with no border or accent bar; inactive icons use reduced opacity that returns to full on hover or selection. Workspace tabs use the dedicated selected/hover/idle-fill treatment in the Brand and Workspace Tabs requirement. Expanded explorer folders MUST be marked by a small app-drawn disclosure triangle and a brighter label only, with no border, accent side bar or extra expansion fill. Explorer rows MUST use 24px height, a 4px horizontal inset, 6px radius, a 16px icon slot inset 4px within the painted row fill (slot starts 8px from the row's left edge) and text starting 28px from the row's left edge, with depth adding 14px; files keep a quiet app-drawn bullet. Text and disclosure/bullet markers MUST be vertically centered on one optical center in the row. Selection and focus in the tree MUST be conveyed by the inset `BG_SELECTED` fill and `TEXT_PRIMARY` label only, with no focus outline around the row or the tree container. When the tree container receives keyboard focus with no row selected, the first visible row MUST become selected with that same fill and primary text, without opening a preview or changing pointer row activation. Every scroll area (explorer tree, picker list, preview, diagnostics) and the terminal scrollback bar MUST use hover-only floating overlay bars: no track or thumb at rest, a thumb-only fade over about 120–160 ms when the area is hovered, expansion on bar hover/drag with a faint track only then, fade-out on leave, zero allocated width and no layout shift, and no always-on track. Routine integration, shell-running, watcher, version, branch, revision and performance statuses MUST NOT consume persistent banners, pills, path/status rows or empty session strips; technical details belong in Diagnostics. Real shell/input, file/root and explicitly requested action errors MUST remain contextual and non-blocking. The explorer MUST occupy the full right content column with no preview below it, MUST be hideable/restorable through the header Explorer toggle, and MUST have a resizable width constrained to leave a usable left pane. Explorer width MUST be adjusted only by pointer-dragging its divider; the explorer header MUST NOT contain Wider, Narrower, Reload or Hide text actions and MUST NOT offer a routine manual refresh control. At rest the divider MUST NOT paint a persistent line or grip; hover or drag MAY fade in a discreet thin line/grip using muted `BORDER_CONTROL` (and `ACCENT` at restrained opacity while dragging) while retaining a comfortable pointer hit area, with a fade-out on leave. Hiding it MUST give the left surface the reclaimed width without killing the shell, resetting the root or losing tree/preview state; restoring MUST reconcile changes that occurred while hidden. Width and visibility need only be retained for that live workspace, not across app restart. Keyboard reachability, accessible names, visible focus, target sizing and the pointer-only divider exception are owned by the Keyboard Focus and Accessibility requirement; essential controls MUST NOT clip or introduce noisy default technical chrome.

#### Scenario: Quiet shell workspace

- Given a newly opened folder and no running OpenCode
- When the default workspace is displayed
- Then a usable terminal and full-height right explorer appear without an empty preview, empty session strip, degraded/waiting banner, technical status bar or routine status pills; the header controls float over the clear/native backdrop without a persistent band and show only icon-only Open folder, Explorer and Diagnostics actions with accessible labels, and the explorer header shows no text action buttons

#### Scenario: Icon-only header controls

- Given a live workspace at 1280×800 or 720×480
- When the user inspects or keyboard-navigates the top header
- Then Open folder, Explorer toggle, Diagnostics and the visible optional Sessions action are icon-only with matching tooltips and accessible labels, no persistent action text labels are rendered, workspace tabs retain their close affordance, and each action is reachable and visibly focused by keyboard

#### Scenario: Header placement and order

- Given a live workspace at 1280×800 or 720×480, with tabs that fit and a case that overflows
- When the header lays out
- Then workspace tabs start at the window gutter/card left edge, the last right-side icon’s 26/24 target, selected/hover fill, and focus ring end at the card right edge (`width - gutter`) with no gutter overhang and the 14px glyph centered inside the target with normal optical padding, the header row has no extra horizontal padding, the top strip and tabs are 26/24 high with the card top at 42/36 and one 8/6 pane gap above and below the strip, the tab-fitting budget reserves the documented 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark (the visible space after the last visible control may be larger), the wordmark-to-first-icon gap is exactly 24px wide / 18px compact, the whole right cluster is reserved so tabs/plus/overflow never overlap it, the app-drawn `+` Open folder action follows the last visible tab and precedes the overflow trigger, the overflow trigger and `+` both remain reachable, and the right icon group orders Sessions (when available), Diagnostics, Explorer with Explorer rightmost while retaining tooltips, accessible names, minimum targets, and keyboard focus

#### Scenario: Unified hover and selection states

- Given header icon actions, optional session tabs, workspace tabs, explorer rows, and picker rows
- When each is hovered, selected, focused, or at rest
- Then header icons, session tabs, explorer rows, and picker rows paint their documented borderless radius-6 `BG_HOVER` hover and `BG_SELECTED` selection fills; workspace tabs use the translucent idle fill and opaque `BG_TAB_HOVER` hover fill with no stroke, and the opaque `BG_TAB_SELECTED` fill with the faint 1px inside stroke; selected icon toggles paint no border or accent bar; tree and picker selection paint no focus outline; inactive tabs/icons use the reduced-opacity treatment; and focus rings, tooltips, accessible names, and minimum targets remain intact on every control except the deliberate tree/picker row exception

#### Scenario: Explorer folder and row presentation

- Given expanded and collapsed folders, files, hover, selection, and keyboard focus in the explorer tree
- When the tree renders
- Then expanded folders use a brighter label, rows use 24px height, a 4px horizontal inset, 6px radius, a 16px icon slot inset 4px within the painted row fill (slot starts 8px from the row's left edge) and text 28px from the row's left edge with 14px per depth, explorer entry icons are owned by the realized `specs/explorer-file-icons.md` specification, which is authoritative for their resolution and rendering, and replace the former app-drawn file bullet and folder disclosure triangle in that 16px slot, entry icons and labels share one vertical optical center, selection/focus uses only the inset `BG_SELECTED` fill plus `TEXT_PRIMARY` text, keyboard focus with no selection selects the first visible row without opening a preview, and no border, focus outline, accent side bar or extra expansion fill is painted

#### Scenario: Hover-only fading scrollbars

- Given overflowing explorer, picker, preview, diagnostics and terminal scroll content
- When the area is at rest, hovered, or its bar is hovered or dragged
- Then no track or thumb is painted at rest, the floating overlay bar fades in over about 120–160 ms as a thumb when the area is hovered, expands and strengthens on bar hover/drag with a faint track only then, fades out on leave, allocates zero width, and never shifts layout

#### Scenario: Explorer hide and restore

- Given an expanded tree and a running shell, with or without an open preview
- When the user activates the header Explorer toggle to hide, files change on disk, and the user activates it again to restore
- Then the left surface reclaims/releases its width, shell and preview state survive, the restored tree reflects current files while preserving valid expansion/selection, and the toggle remains reachable and focused by keyboard

#### Scenario: Explorer resize and compact layout

- Given either a 1280×800 or 720×480 window
- When the user pointer-drags the explorer divider or navigates the rest of the layout by keyboard
- Then its full-height width changes within usable bounds, the divider is invisible at rest and fades in a thin line/grip on hover or drag, the active left surface reflows without terminal corruption, no Wider/Narrower/Reload/Hide text action is present, essential controls have visible focus and do not clip, and the pointer-only divider is not a keyboard stop

#### Scenario: Explorer header action removal

- Given a visible explorer header
- When the user inspects or navigates it by pointer or keyboard
- Then no Wider, Narrower, Reload or Hide text action is present, and no routine manual refresh control is offered for normal live updates

#### Scenario: Hover-only explorer divider

- Given a visible explorer at rest
- When the divider is not hovered or dragged
- Then no vertical divider line or grip is painted; when hovered or dragged, a thin line/grip fades in while the pointer hit area remains usable, and a mere hover does not change the stored per-workspace width

#### Scenario: Explorer splitter fade

- Given a visible explorer splitter with its 8px pointer hit area
- When the pointer hovers, starts dragging, releases, or leaves
- Then the splitter paints nothing at rest, fades in over about 140 ms to a discreet 1px line and 6×32 grip using muted `BORDER_CONTROL` at partial opacity on hover, uses the green `ACCENT` at a restrained opacity while dragging, fades out on leave, keeps the resize cursor and “Drag to resize explorer” tooltip, remains pointer-only (not a keyboard stop), and never paints a bright permanent line

#### Scenario: No manual refresh control

- Given a visible explorer while files change on disk
- When the user looks for refresh actions
- Then no routine manual Reload control exists and creates, deletes and renames still appear automatically through the watcher

#### Scenario: Quiet integration chrome

- Given OpenCode is absent, inactive, disconnected, degraded, or has exited normally
- When the workspace chrome is inspected
- Then no routine degraded/waiting/unsupported banner or status pill is shown, and integration detail remains available only through Diagnostics

### Requirement: Brand and Workspace Tabs

Made MUST present editor-like workspace tabs starting at the window gutter/card left edge, with folder labels, full-path tooltips, and a reachable close affordance. The header row MUST span the card left/right edges with no extra horizontal padding. The last right-side icon’s 26/24 target, selected/hover fill, and focus ring MUST all end at the card right edge with no gutter overhang; the 14px glyph MUST remain centered inside its target with normal optical padding on both sides. The two-tone monospace `MADE` wordmark with its accent `M` and tooltip identity MUST sit to the right of the tab strip, immediately before the right icon group, with a reserved 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark (the visible flexible space after the last visible control is larger when few tabs are shown), the same exact 24px wide / 18px compact gap from the wordmark to the first right icon, and no divider. The wordmark MUST NOT look like a tab and MUST NOT paint a fill, hover, or selection treatment. The top workspace strip and workspace tabs MUST both be 26/24 high; the session-map strip/tabs keep their previous 36/34 and 28/26 metrics unless they directly depend on the reduced top height. The selected workspace tab MUST use the opaque `BG_TAB_SELECTED #1B231F` dark green-gray fill with a `TEXT_PRIMARY` 13.0px label, the faint 1px inside stroke, and no underline, dot/hollow marker, accent bar, or width animation. Selection MUST also carry a subtle non-color cue: the selected label is set at 13.0px versus 12.5px for inactive tabs, vertically centered, with tab width and layout unchanged and no animation; the selected fill is also a luminance step above the idle/hover fills. Inactive workspace tabs MUST use the translucent black `BG_TAB_IDLE` fill, no stroke, and secondary text at reduced opacity. Hovering an inactive workspace tab MUST replace the idle fill with the slightly darker opaque `BG_TAB_HOVER #151C18` fill, paint no stroke, and restore the label to full-opacity `TEXT_PRIMARY`; a selected tab keeps `BG_TAB_SELECTED` and the faint inside stroke on hover. The tab close affordance MUST keep its 24×24 hit area and keyboard focusability reserved at all times, but its `×` glyph MUST be hidden at rest—including on the selected tab—and MUST appear only while the tab is pointer-hovered or while the tab or close action has keyboard focus. The close affordance MUST NOT paint a hover background fill; hovering only the close icon MUST turn the visible glyph `ERROR` and MUST also apply the whole tab's hover fill and label treatment without changing tab width, label truncation, or layout. The glyph MUST use the fixed documented size with no scale or width animation. Workspace tabs MUST remain visually distinct from optional OpenCode session tabs and MUST NOT animate their width. When tabs do not fit, a labeled, keyboard-reachable overflow list MUST expose every workspace tab; labels MUST truncate with an ellipsis and retain full-value tooltips.

#### Scenario: MADE wordmark identity

- Given the running UI
- When the header brand is inspected
- Then workspace tabs start at the window gutter/card left edge, the last right-side icon’s 26/24 target, selected/hover fill, and focus ring end at the card right edge with the 14px glyph centered inside the target with normal optical padding, the top strip and tabs are 26/24 high with one 8/6 pane gap above and below, and `MADE` is a cached two-tone monospace wordmark with accent `M`, primary `ADE`, the documented tracking and mint accent, the “Minimal Agentic Development Environment” tooltip, a reserved 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark (visible space after the last visible control may be larger), an exact 24px wide / 18px compact wordmark-to-first-icon gap, no divider, and no tab fill/hover/selection on the wordmark

#### Scenario: Workspace tab presentation

- Given two or more open workspaces
- When the tab strip is displayed
- Then each tab shows its folder label, full-path tooltip, and close affordance; the selected tab uses the opaque dark green-gray `BG_TAB_SELECTED` fill, a `TEXT_PRIMARY` 13.0px label, the faint 1px inside stroke, and no underline, dot/hollow marker, accent bar, or width animation; inactive tabs use the translucent black `BG_TAB_IDLE` fill, no stroke, secondary text at 12.5px and reduced opacity; and session tabs are not presented as workspace tabs

#### Scenario: Workspace tab hover and close

- Given selected and inactive workspace tabs with their close affordances
- When an inactive tab or only its close icon is hovered
- Then an inactive tab switches to the opaque `BG_TAB_HOVER` fill, paints no stroke, and shows a full-opacity primary label; a selected tab keeps `BG_TAB_SELECTED` and the faint inside stroke; the hidden-at-rest close glyph appears while the tab is hovered, paints no hover background, and, when hovered directly, turns only the glyph `ERROR` while the parent tab also shows its hover fill and primary label and the tab width/label layout stays unchanged

#### Scenario: Workspace tab close visibility

- Given selected and inactive workspace tabs with their 24×24 close affordances
- When the tabs are at rest, then pointer-hovered, then keyboard-focused on the tab or close
- Then the `×` glyph is absent at rest for idle and selected tabs, appears on tab hover or keyboard focus, keeps its 24×24 keyboard-reachable target and focus ring, uses the `ERROR` tone only on direct close hover without an extra fill, and never scales or changes tab width/label layout

#### Scenario: Workspace tab overflow and close

- Given enough workspace tabs to exceed the available width at 1280×800 or 720×480
- When the header lays out
- Then a labeled keyboard-reachable overflow list exposes every tab, the active tab remains visible, no tab or close affordance is clipped, and closing still targets only its workspace

### Requirement: Optional Session Map Presentation

Made MUST present the OpenCode session map as an optional, initially hidden surface scoped to the selected workspace and visually distinct from workspace tabs. It MUST NOT reserve an empty strip before OpenCode data exists or when the map is closed. When opened, it MUST show each session's title and path, exactly one active session, and busy, attention, unread, and renaming indicators using shape plus text, not color alone. The active session tab MUST use a `BG_SELECTED` background with a `TEXT_PRIMARY` 13.0px label and MUST NOT draw an underline, border, or width animation; inactive session-tab labels use 12.5px text at reduced opacity. Long labels and overflow MUST remain inspectable; the map MUST NOT hide sessions from its overflow list. Session data, focus behavior, and failures are owned by the core spec.

#### Scenario: Session map hidden by default

- Given a shell-only workspace or one with OpenCode data but no map request
- When the default workspace is displayed
- Then no session strip or empty session space is reserved, and the optional Sessions action is the explicit way to open the map

#### Scenario: Session activity indicators

- Given an OpenCode session reports busy, attention, or unread state
- When the session map renders
- Then the corresponding indicator is shown and updates when the state changes

#### Scenario: Session map overflow and long labels

- Given more sessions or longer titles/paths than fit
- When the session map lays out
- Then labels truncate with an ellipsis and retain full-value tooltips, a labeled overflow lists every session with its path, and no session is hidden from the map

### Requirement: Explicit Folder Picker

Made MUST present the folder picker as an explicit card-based selection surface with the `MADE` brand, a Close X and Escape return path, a Back action for the add-workspace flow, an inline expandable folder tree rooted at the selected start directory, a single rounded selected-row treatment using only an inset `BG_SELECTED` fill and `TEXT_PRIMARY` text with no border, left accent bar, or focus outline, a visible path rendered as a compact inset `BG_SELECTED` chip with `TEXT_PRIMARY` monospace text and no border, a borderless quiet secondary folder action, and a primary accent folder action. The root directory's child folders MUST appear as depth-0 rows. Expanding a folder MUST keep its parent and all ancestors visible and insert its child folders below it, indented 14px per depth; collapsing MUST remove only that subtree and MUST NOT replace the parent rows. Rows MUST use 32px height, a 4px inset, radius-6 hover/selected fills, primary selected text, app-drawn disclosure triangles, and no row/container focus outline. The picker card and its buttons MUST keep their focus rings; the deliberate row-focus exception is owned by the Keyboard Focus and Accessibility requirement. Clicking a folder MUST select it and toggle its expansion: an expanded folder collapses and stays selected; a collapsed folder expands; child folders expand/select the same way. The path chip MUST show the selected tree row path, or the tree root when nothing is selected, and MUST be the target for “Use current folder”. Selection MUST be one flattened visible-row index; there MUST be no separate hidden child index. Keyboard navigation MUST move Up/Down through visible rows (first/last from no selection), Right MUST expand the selected folder or select its first child if already expanded, Left MUST collapse the selected folder or select its parent row when collapsed (root children have no parent row), Enter MUST toggle expansion, Cmd/Ctrl+Enter MUST confirm the selected/current target, and Backspace or the Up toolbar action MUST re-root to the parent directory and clear expansion/selection. “Use current folder” MUST open the selected path target or the root when nothing is selected; “Open selected folder” MUST open the selected row when one exists and otherwise be disabled. The picker MUST keep both actions. Re-rooting MUST start the new list at the top; expanding MUST reveal the selected row with minimal scrolling that keeps the visible ancestor chain when it fits and otherwise keeps the nearest ancestors plus the selected row visible; Refresh MUST reload the root and every expanded directory while preserving expansion and valid selection by path; each directory read MUST retain the 1000-entry cap; no picker state persists across app restart. The first click of a double-click MUST toggle once and the second MUST NOT toggle again or start a workspace. An unreadable child directory MUST leave the row unexpanded and show the existing non-blocking picker error; invalid selection MUST remain safe. The picker MUST remain keyboard operable and MUST preserve a live workspace when opened to add another folder. Picker appearance is owned by this UI requirement; the home/`--picker`/explicit-path resolution and no-process-before-confirmation behavior are owned by the core Workspace Launch requirement.

#### Scenario: Picker selection and close

- Given the picker is shown
- When the user inspects and confirms or closes it
- Then the selected tree row uses only one rounded inset `BG_SELECTED` fill with primary text and no border, focus outline, or left accent bar; the path chip uses an inset `BG_SELECTED` fill, primary monospace text, and no border and shows the selected row path or root when nothing is selected; the “Open selected folder” action is borderless with `BG_ELEVATED` rest, `BG_HOVER` hover and `BG_SELECTED` pressed while the primary action stays accent-filled and opens the selected path target or root; Close X has a tooltip and ≥24px target, Escape/Close return to the live workspace when present or to the default home/terminal fallback, and Back remains available for the add-workspace flow

#### Scenario: Picker tree expansion and ancestors

- Given the picker shows a folder with child and grandchild folders
- When the user clicks a folder, clicks an expanded folder, or double-clicks a row
- Then the root rows remain in place, the clicked folder is selected and expands to insert its children below it at 14px per depth with disclosure triangles, ancestors stay visible, the path chip carries the clicked/selected row as the current target, collapsing removes only that subtree and keeps the folder selected, pointer hover still shows only a `BG_HOVER` row under the cursor, a double-click toggles only once, and no workspace process starts before confirmation

#### Scenario: Picker keyboard tree navigation

- Given the picker is focused with visible rows
- When the user presses Up/Down, Right, Left, Enter, Cmd/Ctrl+Enter, Backspace, or Escape
- Then Up/Down move the visible-row selection (first/last from no selection), Right expands the selected folder or selects its first child when expanded, Left collapses the selected folder or selects its parent row when collapsed, Enter toggles expansion without starting a workspace, Cmd/Ctrl+Enter confirms the selected/current target, Backspace re-roots to the parent and clears expansion/selection, Escape/Close behave as documented, and the tree remains operable with thousands of rows

#### Scenario: Picker refresh and re-root state

- Given an expanded picker tree with a selected descendant row
- When the user refreshes or re-roots with Up/Backspace
- Then Refresh re-reads the root and every expanded directory, preserves valid expansion and selection by path, and prunes removed paths; re-rooting starts the new list at the top with no selection or expansion; each directory remains capped at 1000 entries; an unreadable child stays collapsed with a non-blocking error; and no state persists across app restart

#### Scenario: Picker preserves live workspace

- Given a live workspace tab
- When the user opens the picker to add a folder, cancels, or confirms a new folder
- Then the existing shell and its state remain untouched; cancellation returns to it, and confirmation opens a new workspace tab without stopping the existing shell

### Requirement: Full-Left Preview Presentation

Made MUST present an opened file as a read-only preview occupying the entire left content pane in place of the terminal visually, never underneath the explorer or in a split with the terminal. No preview pane or placeholder MUST occupy space before a file is opened. The preview MUST include a compact header with filename, full-path tooltip, an understated read-only label, Copy path, Refresh, Close, and a clearly labeled return-to-terminal control. Close or return MUST restore the same live terminal and its focus. Preview content MUST use the documented gutter, code typography, horizontal scrolling, and safe loading/binary/size/encoding/unavailable states. Syntax highlighting SHOULD be applied where available and MUST fall back to plain text. Read-only safety, process continuity, input isolation, refresh, and root clearing are owned by the core spec.

#### Scenario: Text file preview

- Given a text file inside the active root
- When the user selects it in the explorer
- Then Made shows its content read-only across the entire left content pane, the full-height right explorer remains intact, and no editing or saving controls exist

#### Scenario: Preview terminal roundtrip

- Given a running terminal command in the selected workspace
- When a file is opened and the user closes the preview or chooses return to terminal
- Then preview used the entire left content area and the left surface returns to the live terminal with terminal focus

#### Scenario: Syntax highlighting

- Given a supported text file is open in the preview
- When the file is rendered
- Then syntax highlighting is applied where available and plain text is shown when highlighting is unavailable

### Requirement: Diagnostics Presentation

Made MUST present Diagnostics as an opaque floating card with a status pill, reason, copy actions, sectioned connection/session/worktree/version/metrics/log detail, bounded log block, and a path/copy footer. Under `LOG`, the card MUST show a compact Previous log path row. When the previous log exists, the row MUST offer a "Copy previous log path" action, show the full path in a tooltip, and remain truncation-safe at 720×480; when it does not exist, the row MUST show a quiet "not yet rotated" state with that action disabled. The existing active-log path and copy row MUST remain. A retention warning or error present while the card is rendering (for example the startup legacy-size warning) MUST use the warning or error tone inside the card. Final-exit write/rotation failures are stderr-only and MUST NOT be claimed as in-card notices. The card MUST remain usable at the documented responsive sizes and MUST NOT turn technical details into routine chrome outside the card. Shell-follow diagnostics MUST expose workspace identity, reporting capability/reason, latest shell cwd, effective root/source, and inactive optional integration without treating shell-only use as degraded. Data fields, status semantics, redaction, retention, and log content are owned by the core spec.

#### Scenario: Shell follow diagnostics without chrome noise

- Given a shell-only workspace with supported or unsupported cwd reporting
- When the user opens Diagnostics
- Then its shell-follow detail is presented inside the diagnostics card, routine technical chrome remains hidden outside Diagnostics, and displayed/copied logs contain no secrets

#### Scenario: Diagnostics panel presentation

- Given Diagnostics is open at 1280×800 or 720×480
- When the card is inspected or resized
- Then it uses the opaque card, documented width/height clamps, section labels, monospace values, bounded log block, copy actions, and no clipped essential detail

#### Scenario: Previous log path and retention notice

- Given Diagnostics is open and the previous diagnostics log exists, or has not yet been rotated
- When the user inspects or copies the Previous log row
- Then the full previous-log path is available (truncation-safe with a tooltip), copying it works only when the file exists, an absent file shows a quiet "not yet rotated" state with the action disabled, a retention warning or error present while the card is rendering uses the warning or error tone inside the card, and final-exit failures are stderr-only rather than claimed in-card

### Requirement: Responsive and Motion

Made MUST keep the UI usable at 1280×800 and 720×480 with the documented gutters, compact spacing, explorer defaults, truncation, and labeled overflow behavior; no essential control may vanish through clipping. At 1100+ width the explorer SHOULD default to 360px. At 900–1099 width it SHOULD default to 35% of the usable content width, clamped to 288–360px. Below 900 width the compact gutter, header height, radii, and 288px explorer default apply. Hiding or showing the explorer SHOULD animate its panel width over about 150 ms with quadratic ease-out while content is clipped rather than reflowed, MUST reverse cleanly, and MUST stop tree/splitter input once closed. Motion is limited to spinners, the terminal cursor blink (~530 ms on/off only while focused and reported as blinking), the explorer width transition, hover-triggered scrollbar fades over about 120–160 ms, the explorer splitter fade over about 140 ms, and standard control animation; no essential information may exist only in motion, tab widths MUST NOT animate, splitter fades MUST NOT change layout or allocated width, scrollbar fades MUST NOT allocate width or shift layout, and no scroll track or thumb may remain always visible.

#### Scenario: Responsive breakpoints preserve panes and controls

- Given a window at 1280×800, 900–1099, or 720×480
- When the UI lays out
- Then the left surface and explorer keep usable widths, compact controls use the documented heights/gutters, labels truncate with tooltips, optional controls move to labeled overflow instead of clipping, and preview/return remains usable

#### Scenario: Explorer hide-show animation

- Given the explorer is visible or hidden and the user toggles it
- When the width transition runs, is interrupted, or is reversed
- Then an intermediate width frame exists, tree content is clipped rather than reflowed, the animation settles at the stored width, and the closed state stops tree/splitter input

#### Scenario: Hover-triggered scrollbar fades preserve layout

- Given an overflowing scroll area
- When the hover-triggered scrollbar fade runs, is interrupted, or is unavailable
- Then the scrollbar stays hidden at rest, the rest/hover/selected states remain readable without motion, the bar allocates no width, and no content reflows or shifts

#### Scenario: Motion does not carry essential information

- Given spinners, the explorer transition, hover-triggered scrollbar fades, the splitter fade, the terminal cursor blink, and standard control animation
- When motion is unavailable or reduced
- Then no essential status, selection, action, or error is communicated only through animation

### Requirement: Keyboard Focus and Accessibility

Made MUST make every persistent interactive control keyboard reachable with a visible focus state, never relying on fill change alone. Icon-only actions MUST expose matching tooltip and accessibility-name metadata; workspace and optional session tabs MUST support keyboard entry, Left/Right movement, and Enter/Space activation; the terminal MUST remain one focus stop; the explorer tree, preview actions/return, picker, and diagnostics MUST remain keyboard operable. Focusing the tree before any row is selected MUST select the first visible row (fill plus primary text only, no preview). The explorer tree row/container and picker rows are a deliberate focus-outline exception: while focus moves there, it MUST be conveyed by the inset `BG_SELECTED` fill plus `TEXT_PRIMARY` text (luminance and text, not hue alone) with no outline; every other persistent control, including the picker card and picker buttons, MUST keep its visible focus ring. The pointer-only explorer divider MUST NOT be a keyboard stop; hide/restore remains the keyboard path. Interactive targets MUST be at least 24×24, text MUST not fall below 10px, and status MUST use shape plus text plus color. Platform screen-reader exposure MUST NOT be claimed while the accesskit integration is absent.

#### Scenario: Visible focus on persistent controls

- Given a keyboard-only walkthrough at 1280×800 and 720×480
- When focus moves through header actions, workspace/session tabs, tree, preview actions, picker, and diagnostics
- Then every persistent keyboard-reachable control shows a visible focus ring except tree rows/container and picker rows, which show only the inset `BG_SELECTED` fill and `TEXT_PRIMARY` text; a tree container focused before any selection selects its first visible row without opening a preview; the pointer-only divider is skipped, and focus never lands on a clipped or hidden control

#### Scenario: Icon actions expose accessible names

- Given the icon-only Open folder, Explorer, Diagnostics, and optional Sessions actions
- When each is inspected by keyboard or accessibility metadata
- Then it has a matching tooltip, accessible name, minimum target, and visible focus, and the icon alone is not the only label

#### Scenario: Keyboard workspace and session navigation

- Given multiple workspace tabs and an open optional session map
- When the user enters the tab strip, moves Left/Right, and presses Enter/Space
- Then the intended workspace or session activates without leaking focus or input to an inactive surface, and the terminal remains a single focus stop

#### Scenario: Status cues do not rely on color

- Given busy, attention, unread, renaming, connected, degraded, shell-exit, tree-error, and selected states
- When the UI is inspected without color or through a colorblind simulation
- Then each state retains a shape, text, or distinct non-color cue in addition to color, selected/active tabs use the 13.0px label cue versus 12.5px inactive labels, and active tabs do not rely on a dot/hollow marker

### Requirement: Explorer and source-control icons

When the explorer root is a known git worktree, the explorer header MUST show two icon-only controls: a files mark of two offset rounded sheets, the back sheet up-left and the front sheet down-right, whose tooltip and accessible name are "Explorer", and the existing source-control icon whose accessible name is "Source Control". That mark MUST NOT be a folder and MUST NOT be a tree. That accessible name MUST remain "Source Control". Its tooltip MUST always include the text "Source Control". When no changes are open, the tooltip MUST be exactly "Source Control". When changes are open, the tooltip MUST be "Source Control - " followed by each visible section as "{title} ({count})", joined by " · ", in section order. Empty sections MUST be omitted from the tooltip. The accessible name MUST stay exactly "Source Control". The files mark's tooltip and accessible name MUST stay "Explorer". Those names MUST NOT swap. The source-control icon MUST stay the rightmost, and the files mark MUST sit immediately to its left with a 4px gap. The head line and the root name MUST clip before both icons. The active view's icon MUST use the existing selected fill, and the other icon MUST NOT. Activating the active icon MUST NOT switch away. Activating the other icon MUST switch only the explorer body. While the file tree is showing, the header title MUST be `EXPLORER` and the header MUST show the root name. While the git view is showing, the header title MUST be `REPOS` in the same section-label treatment as `EXPLORER`, the header MUST show the head line in place of the root name, and the root name MUST NOT be shown. Each icon MUST NOT hide the explorer, change the left surface, or replace the window Explorer toggle, which remains the hide and restore control. Both icons MUST remain icon-only actions with a visible focus state and MUST NOT be text buttons. When the folder is not a known git worktree, the header MUST show neither icon. Ring colors MUST still apply to the source-control icon in both views. The files mark MUST NOT be recolored by those rings.

#### Scenario: Known worktree shows both icons

- Given the explorer root is a known git worktree and the file tree is showing
- When the explorer header is inspected
- Then a files mark named "Explorer" sits immediately left of the source-control icon named "Source Control" with a 4px gap, the source-control icon is rightmost, the files mark uses the selected fill, the source-control icon does not, the names do not swap, the root name clips before both icons, the header title is `EXPLORER`, and no text action button is shown

#### Scenario: Explorer header icon is a files mark

- Given a known git worktree
- When the explorer header is shown
- Then the left icon is two offset rounded sheets, the back sheet up-left and the front sheet down-right, not a folder and not a tree; its tooltip and accessible name are "Explorer"; and it sits immediately left of the source-control icon with a 4px gap

#### Scenario: The other icon switches only the explorer body

- Given both icons are visible and the file tree is showing
- When the user activates the files mark, then activates the source-control icon, then activates the source-control icon again
- Then the first activation does not switch away; the second makes the body the git view, gives the source-control icon the selected fill, and leaves the files mark without that fill, with the title `REPOS` and the head line in place of the root name, and that head line clips before both icons; the third activation does not switch away; the names stay "Explorer" and "Source Control"; the explorer stays visible; the left surface is unchanged; and the window Explorer toggle still hides and restores the explorer

#### Scenario: Unknown folder has neither icon

- Given the explorer root is not a known git worktree
- When the explorer is shown
- Then neither the files mark nor the source-control icon is present, the body is the file tree, and the header title remains `EXPLORER`

#### Scenario: Ring colors stay on the source-control icon

- Given a known git worktree with a new change, first while the file tree is showing and then while the git view is showing
- When the header icons are shown
- Then only the source-control icon uses the ring colors in both views, and the files mark is not recolored

### Requirement: Source-control ring colors

The source-control toggle is the three-ring branch glyph, not the linked-worktree icon. Its highest ring MUST use `SUCCESS` only when at least one new change is open. A new change is an untracked path or a staged add. Its vertically middle ring MUST use `WARNING` only when at least one modified change is open. Its lowest ring MUST use `ERROR` only when at least one deleted change is open. A ring with no open change of its type MUST keep the icon's original color. The three rings MUST be colored independently. A clean repository MUST leave all three rings in the original color. The stem and curve MUST stay the original color. A renamed change MUST NOT light a ring. A conflict MUST light the lowest ring in `ERROR`, because that kind already uses `ERROR` and the glyph has no fourth ring. These ring colors MUST remain visible when the icon is selected or at rest, and the selected treatment MUST NOT replace them. A ring that uses `SUCCESS`, `WARNING`, or `ERROR` MUST fill its center while the source-control icon is hovered or while it has the selected fill. A ring that kept the original color MUST stay hollow in both states. When the icon is neither hovered nor selected, every ring MUST be hollow. The stem and curve MUST stay unfilled. The linked-worktree icon and the files mark MUST NOT be recolored by this rule. Ring colors MUST still apply to the source-control icon while either view is showing.

#### Scenario: Source-control rings follow open changes

- Given a known worktree with a new change, a modified change, and a deleted change, then the same worktree with only a modified change, then a clean repository
- When the source-control icon is shown
- Then the highest ring is `SUCCESS` only while a new change is open, the vertically middle ring is `WARNING` only while a modified change is open, and the lowest ring is `ERROR` only while a deleted change is open; each other ring keeps the original color; the clean repository leaves all three rings in the original color; and the stem and curve stay the original color

#### Scenario: Rename and conflict use the existing rings

- Given only a renamed change, then only a conflict
- When the source-control icon is shown
- Then a rename leaves all three rings in the original color, and a conflict lights only the lowest ring in `ERROR`

#### Scenario: Linked-worktree icon stays uncolored

- Given a linked worktree with open changes
- When the linked-worktree icon and the source-control icon are shown
- Then only the source-control rings use the status colors, and the linked-worktree icon keeps its original color

#### Scenario: Hover fills a colored ring

- Given a source-control icon whose highest ring uses `SUCCESS`, whose vertically middle ring kept the original color, and whose lowest ring uses `ERROR`
- When the icon is hovered, then the pointer leaves it while the icon stays selected
- Then only the `SUCCESS` and `ERROR` rings fill their centers during the hover and while the selected fill remains after the pointer leaves; the original-color ring stays hollow; and the stem and curve stay unfilled

#### Scenario: Selected fill keeps a colored ring filled

- Given a source-control icon with the selected fill, a `SUCCESS` ring, and a ring that kept the original color, and that icon is not hovered
- When the icon is shown, then the selected fill is removed
- Then the `SUCCESS` ring stays filled while the selected fill is active, the original-color ring stays hollow, every ring is hollow once the icon is neither hovered nor selected, and the stem and curve stay unfilled

#### Scenario: Selected treatment keeps the ring colors

- Given a known worktree with a new change, a modified change, and a deleted change, first with the source-control icon at rest and then with that icon selected while the git view is showing
- When the source-control icon is shown in each state
- Then both states keep the highest ring `SUCCESS`, the vertically middle ring `WARNING`, and the lowest ring `ERROR`, and the selected treatment does not replace those colors

### Requirement: Git view head label

While the git view is showing, the head line MUST appear in the explorer header beside the `REPOS` title, and the git-view body MUST NOT repeat it. When the checkout is not a linked worktree, the head line MUST show the current branch name when HEAD is attached, and an 8-character commit id when HEAD is detached. It MUST NOT show the words "Detached HEAD". Hovering a shown commit id MUST show the commit subject. If the subject is empty, the hover MUST still show the id. The commit subject MUST remain available on hover when that id line is truncated. A linked worktree MUST show a small monochrome icon. When its folder name differs from the branch or 8-character id, the order MUST be that head text, then the icon, then the folder name. When the folder name equals the branch or the 8-character id, the header MUST show the icon and that folder name only. It MUST NOT also show the branch or id, and it MUST NOT use a separate icon-before-head layout. The shown name MUST be the linked-worktree folder name, not a second copy of the branch. The " · " separator MUST NOT appear before the name. Hovering the icon or the name MUST say that it is a linked worktree and MUST include the folder name. That hover MUST NOT replace the commit-subject hover on a shown commit id. A worktree that is not linked MUST show the branch or id and MUST NOT show a worktree icon or a worktree name. When the head line does not fit, it MUST truncate and MUST keep the full visible line available.

#### Scenario: Attached branch without a linked label

- Given a known git worktree that is not a linked worktree and whose HEAD is attached to a branch
- When the git view is shown
- Then the header shows that branch name, does not replace it with a commit id, does not show the words "Detached HEAD", and shows no worktree icon and no worktree name; the title is `REPOS`, the root name is not shown, and the git-view body does not repeat the head line

#### Scenario: Detached HEAD shows a short commit id

- Given a known git worktree whose HEAD is detached, whose 8-character commit id is `fb16d0ee`, and whose commit subject is "Fix the header"
- When the git view is shown and the head line is hovered, including when that line is truncated
- Then the header shows `fb16d0ee`, does not show the words "Detached HEAD", the hover shows "Fix the header", the full visible line remains available, the title is `REPOS`, the root name is not shown, and the git-view body does not repeat the head line

#### Scenario: Distinct linked-worktree label

- Given a linked worktree whose branch is "feature" and whose distinct linked-worktree label is "linked"
- When the git view is shown
- Then the header shows the branch name `feature`, then a small monochrome icon, then the worktree name `linked`; the name is not a branch; there is no " · " before the name; hovering the icon or the name says it is a linked worktree and includes `linked`; the title is `REPOS`, the root name is not shown, and the git-view body does not repeat the head line

#### Scenario: Repeated linked label is not duplicated

- Given a linked worktree whose linked-worktree label is the same as its branch name
- When the git view is shown
- Then the header shows the small monochrome icon and that folder name only, does not also show the branch, and does not use a separate icon-before-head layout; hovering the icon or the name says it is a linked worktree and includes that folder name; the title is `REPOS`, the root name is not shown, and the git-view body does not repeat the head line

#### Scenario: Detached linked worktree

- Given a linked worktree whose HEAD is detached at `fb16d0ee` and whose distinct linked-worktree label is "linked"
- When the git view is shown
- Then the header shows `fb16d0ee`, then a small monochrome icon, then the worktree name `linked`; the name is not a branch; there is no " · " before the name; the words "Detached HEAD" are not the head text; the title is `REPOS`, the root name is not shown, and the git-view body does not repeat the head line

#### Scenario: Empty commit subject still shows the id

- Given a detached HEAD whose 8-character commit id is `fb16d0ee` and whose commit subject is empty
- When the head line is hovered
- Then the hover shows `fb16d0ee` and the header does not show the words "Detached HEAD"

#### Scenario: Repeated linked label matches the visible id

- Given a linked worktree whose HEAD is detached at `fb16d0ee` and whose linked-worktree label is `fb16d0ee`
- When the git view is shown
- Then the header shows the small monochrome icon and the folder name `fb16d0ee` only, does not also show the commit id as a separate head, and does not use a separate icon-before-head layout; hovering the icon or the name says it is a linked worktree and includes `fb16d0ee`

#### Scenario: Repeated worktree name still shows the icon

- Given a linked worktree whose folder name equals its branch name, a linked worktree whose folder name equals its 8-character commit id `fb16d0ee`, and a checkout that is not a linked worktree
- When each git-view header is shown and each linked icon is hovered
- Then each linked header shows the small monochrome icon and that folder name only, does not also show the branch or id, and does not use a separate icon-before-head layout; the hover says it is a linked worktree and includes the folder name; the non-linked header shows the branch or id and no worktree icon

#### Scenario: Matching folder name replaces the head text

- Given a linked worktree whose folder name equals its branch, and a linked worktree whose folder name equals its 8-character commit id
- When each header is shown and the icon or the name is hovered
- Then the header shows the icon and that folder name only, does not also show the branch or id, and does not use a separate icon-before-head layout; the hover says it is a linked worktree and includes the folder name

#### Scenario: Linked worktree name has an icon

- Given a linked worktree named `feat-git-repos` whose HEAD is detached at `fb16d0ee`, and then a non-linked checkout whose branch is `main`
- When each git-view header is shown, and the linked icon or name is hovered
- Then the linked header shows `fb16d0ee`, a small monochrome icon immediately before the worktree name `feat-git-repos`, and no " · " before that name; the name is not a branch; the hover says it is a linked worktree and includes `feat-git-repos`; the commit id and its subject hover are otherwise unchanged; the non-linked header shows `main` with no worktree icon and no worktree name

#### Scenario: Git view header shows REPOS and the head line

- Given the file tree is showing for a known git worktree whose HEAD is attached to a branch
- When the user opens the git view, then returns to the file tree
- Then the git-view header title is `REPOS` in the same section-label treatment as `EXPLORER`, the header shows the branch head line and does not show the root name, the git-view body does not repeat the head line, and the file tree restores the `EXPLORER` title and the root name

### Requirement: Grouped changes

The git view MUST present changes in the sections "Merge Changes", "Staged Changes", "Changes", and "Untracked", in that order. A section with no rows MUST be omitted and MUST NOT show a zero. Each visible section title MUST be "{title} ({count})", for example "Changes (5)". The view MUST NOT present the changes as one flat list and MUST NOT show a commit box. A repository with no listed changes MUST show "No changes". Ignored paths MUST NOT be listed and MUST NOT mark a folder or a file row. A path that is both staged and unstaged MUST appear in both "Staged Changes" and "Changes", each with that side's status dot. A conflicted path MUST appear in "Merge Changes" with a Conflict dot and MUST NOT also appear as an ordinary staged or unstaged row. Each row MUST show its path relative to the explorer root. The git view and the file-tree change dots MUST include only changes under the explorer root. A change outside that root MUST NOT be listed or marked.

#### Scenario: Non-empty sections stay in order

- Given a known git worktree with a conflicted path, a staged path, an unstaged path, and an untracked path
- When the git view is shown
- Then the visible sections are "Merge Changes", "Staged Changes", "Changes", and "Untracked", in that order, and there is no commit box and no single flat list

#### Scenario: Empty sections are omitted

- Given a known git worktree whose only listed change is untracked
- When the git view is shown
- Then "Untracked" is shown and "Merge Changes", "Staged Changes", and "Changes" are omitted

#### Scenario: Clean repository

- Given a known git worktree with no listed changes
- When the git view is shown
- Then the view shows "No changes" and shows no change section, and the source-control tooltip is exactly "Source Control"

#### Scenario: Visible sections show their row counts

- Given the git view has two staged rows and one untracked row, and no other changes
- When the git view and the source-control tooltip are shown
- Then the visible titles are "Staged Changes (2)" and "Untracked (1)", empty sections are omitted from the view and the tooltip, the tooltip is exactly "Source Control - Staged Changes (2) · Untracked (1)", and the accessible name is exactly "Source Control"

#### Scenario: Tooltip lists visible section counts

- Given five rows in "Changes" and two rows in "Untracked", and no other changes
- When the git view and the source-control tooltip are shown
- Then the titles are "Changes (5)" and "Untracked (2)", and the tooltip is exactly "Source Control - Changes (5) · Untracked (2)"

#### Scenario: Tooltip uses parentheses and a middle dot

- Given one visible section with five rows, then that section plus an untracked section with two rows
- When each source-control tooltip is shown
- Then the first tooltip is exactly "Source Control - Changes (5)", the second is exactly "Source Control - Changes (5) · Untracked (2)", and the accessible name stays exactly "Source Control"

#### Scenario: Partially staged file appears twice

- Given a file that has a staged change and a separate unstaged change
- When the git view is shown
- Then that file appears in "Staged Changes" and in "Changes", each with that side's status dot and kind tooltip

#### Scenario: Ignored path is omitted

- Given an ignored path inside the explorer root
- When the git view and the file tree are shown
- Then the ignored path is not listed in any section and does not mark a folder or a file row

#### Scenario: Changes outside the explorer root are omitted

- Given the explorer root is a subdirectory of a git worktree, with one change inside that root and one change outside it
- When the git view and the file tree are shown
- Then only the change under the explorer root is listed or marked

### Requirement: Git-view row icon and status dot

Each git-view row MUST show the same file or folder icon used in the file tree, immediately before the path. It MUST NOT show a status letter. It MUST show one right-side status dot. A staged add and an untracked path MUST use New in `SUCCESS`. A modified path, including a type change, MUST use Modified in `WARNING`. A deleted path MUST use Deleted in `ERROR`. A conflicted path MUST use Conflict in `ERROR`. A renamed path, including a copied path, MUST use Renamed in `INFO`. A copied path MUST NOT use Conflict. The section title already separates a staged add from an untracked path. The dot tooltip MUST name that kind with "New", "Modified", "Deleted", "Renamed", or "Conflict", and that tooltip MUST be the non-color cue. The tooltip MUST be anchored at the dot, not at the far side of the row. Activating a file row MUST still open the existing preview. Activating a directory row MUST still not open a preview.

#### Scenario: Git-view row shows an icon and a status dot

- Given a git-view file whose kind is Modified, and a git-view directory
- When the rows are shown and the file's status dot is hovered
- Then each row shows the same file or folder icon used in the file tree immediately before the path, shows no status letter, and shows one right-side dot; the file dot is `WARNING` and its tooltip, anchored at the dot, says "Modified"; activating the file opens the existing preview and activating the directory does not

#### Scenario: Kind tooltip replaces the painted letter

- Given an untracked path, a staged add, a type change, a deleted path, a conflicted path, and a copied path
- When the git view shows them
- Then the untracked path and the staged add use a New dot, the type change uses Modified, the deleted path uses Deleted, the conflicted path uses Conflict, and the copied path uses Renamed, not Conflict; none of the rows shows a status letter

#### Scenario: Status-dot tooltip is anchored at the dot

- Given a file-tree row and a git-view row that each show a status dot
- When each dot is hovered
- Then each tooltip is anchored at that dot, not at the far side of the row

### Requirement: File-tree change dot

A folder row in the file tree MUST show one right-side dot when that folder contains a change under the explorer root, including when the folder is collapsed. A file row that is already in the file tree and has a change under the explorer root MUST show one right-side dot with the same treatment as a folder. When the changes for that dot are one kind, it MUST use that kind's semantic color: New uses `SUCCESS`, Modified uses `WARNING`, Deleted and Conflict use `ERROR`, and Renamed uses `INFO`. When they are more than one kind, it MUST use `ACCENT`. The dot MUST expose a tooltip that names the present kinds using "New", "Modified", "Deleted", "Renamed", and "Conflict". A mixed tooltip MUST list only the present kinds, separated by ", ", in that order. The tooltip MUST be available so the dot is not the only cue, and it MUST be anchored at the dot, not at the far side of the row. An unchanged file row MUST show no dot. A deleted path that is not already a file-tree row MUST NOT get a fabricated row. A file-tree row MUST NOT show a status letter.

#### Scenario: Single-kind folder dot

- Given a folder whose only contained change is a deletion, including when that folder is collapsed
- When the file tree shows the folder
- Then the folder shows a dot in `ERROR`, the dot tooltip says "Deleted", and file-tree rows show no status letter

#### Scenario: Mixed-kind folder dot

- Given a folder that contains both a new path and a modified path
- When the file tree shows the folder
- Then the folder shows one dot in `ACCENT`, and the dot tooltip says "New, Modified"

#### Scenario: Folder without changes has no dot

- Given a folder that contains no change under the explorer root
- When the file tree shows the folder
- Then the folder shows no change dot

#### Scenario: Changed file row shows one dot

- Given the file tree already shows a file whose only change kind is Modified, and an unchanged file
- When the file tree is shown
- Then the changed file shows one right-side dot in `WARNING`, its tooltip says "Modified", the unchanged file shows no dot, and neither row shows a status letter

#### Scenario: Deleted path not in the tree has no row

- Given a deleted path under the explorer root that is not already a file-tree row
- When the file tree is shown
- Then no fabricated row is added for that path

### Requirement: Git view uses the existing preview

Activating a file row in the git view MUST open that file in the existing full-left read-only preview. The explorer body MUST remain the git view. Activating a directory row MUST NOT open a preview and MUST NOT replace the left surface. The git view MUST NOT add an editor, a diff view, or a save control.

#### Scenario: File activation opens the existing preview

- Given the git view lists a file
- When the user activates that file
- Then the existing full-left read-only preview shows that file, the explorer remains visible on the git view, and no editing, saving, or git-write control is added

#### Scenario: Directory activation does not open a preview

- Given the git view lists a directory
- When the user activates that directory
- Then no preview opens and the left surface is unchanged

### Requirement: Status failure does not block the file tree

A failed, missing, or unfinished status read MUST NOT block, replace, or disable the file tree. The file tree MUST remain usable, and the user MUST be able to return to it. When the git view is showing and no status snapshot is available yet, the view MUST show "Reading changes…". When the git view is showing and no status snapshot is available because the read failed, the view MUST show "Git status unavailable". Those messages MUST appear in the git view, not as a blocker over the file tree. When the folder is not a known git worktree, the header MUST show neither icon and the file tree MUST be shown.

#### Scenario: Pending read leaves the file tree available

- Given a known git worktree whose status snapshot is not yet available
- When the git view is showing
- Then the git view shows "Reading changes…", and the user can return to a usable file tree

#### Scenario: Failed read leaves the file tree available

- Given a known git worktree whose status read failed and no status snapshot is available
- When the git view is showing, and the user then returns to the file tree
- Then the git view shows "Git status unavailable", and the file tree is usable after the return

#### Scenario: Missing git does not block the file tree

- Given the status cannot be read because the folder is not a known git worktree
- When the explorer is shown
- Then neither icon is present, the file tree is shown, and no status failure blocks the tree

### Requirement: Read-only source control

The git view MUST be read-only. It MUST NOT offer or perform stage, unstage, discard, commit, or checkout, and it MUST NOT otherwise write to git. It MUST NOT show a commit box. It MUST NOT add Wider, Narrower, Reload, Hide, or any other text action. It MUST NOT add a routine manual refresh control. The existing bans on those controls remain in force.

#### Scenario: No git write is offered

- Given the git view is showing a changed file
- When the user inspects the explorer header and the git view
- Then no stage, unstage, discard, commit, or checkout action is available, no commit box is shown, and no git write is offered

#### Scenario: Banned explorer actions stay absent

- Given the git view is showing
- When the user inspects or navigates the explorer header
- Then Wider, Narrower, Reload, and Hide are absent, and no routine manual refresh control is offered

### Requirement: Per-workspace view choice

The choice between the file tree and the git view MUST be remembered separately for each open workspace. Switching to another workspace and back, or hiding and restoring the explorer, MUST keep that workspace's choice. The choice MUST NOT be restored after that workspace is closed or after the application restarts. If the explorer root stops being a known git worktree, the body MUST return to the file tree and the header MUST show neither icon.

#### Scenario: Each workspace keeps its own choice

- Given two open workspaces, one showing the git view and one showing the file tree
- When the user switches from one workspace to the other and back
- Then each workspace shows the view it had before the switch

#### Scenario: Hide and restore keeps the choice

- Given a workspace showing the git view
- When the user hides the explorer and then restores it
- Then that workspace still shows the git view

#### Scenario: Restart does not restore the choice

- Given a workspace was showing the git view
- When the application is started again and that folder is opened
- Then the file tree is shown rather than the previous git-view choice

#### Scenario: Leaving a worktree returns to the file tree

- Given the git view is showing and the explorer root then stops being a known git worktree
- When the explorer updates
- Then neither icon is present and the body is the file tree
