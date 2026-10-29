# Change Spec: made-ui-ux-v1

> **Scope:** This active change owns the UI/UX/visual/presentation contract for the undelivered v1, split from `opencode-session-follow-v1`. Product/runtime/integration/security/performance behavior remains in the core change (`specs/changes/opencode-session-follow-v1/spec.md`). This is not a new version, release, or archive. The UI is implemented on Linux with 654 passing tests and Xvfb/headless evidence; macOS arm64 HudWindow vibrancy and UI rendering were manually verified natively on 2026-10-28 (windowed, resized, and fullscreen), while full native UX and native contrast certification remain pending, as do Windows, macOS x64, native Linux desktop, native pointer/keyboard, and hardware-GPU evidence.

## ADDED Requirements

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

Made MUST present a terminal-first workspace in one native window: workspace tabs above the content area, a selected left surface that is either the terminal or the full-left read-only preview, and a full-height explorer in the right column. Persistent header actions MUST be icon-only buttons with tooltips and accessible labels: Open folder, Explorer toggle, Diagnostics, and the optional Sessions action when shown. The Open folder action MUST live in the workspace tab strip immediately after the last visible tab and before the overflow trigger, use an app-drawn `+` glyph, reserve its width in the tab-fitting budget, and remain reachable with the overflow trigger when tabs overflow. The remaining right icon group MUST order Sessions (when shown), Diagnostics, Explorer left-to-right so Explorer is the rightmost icon. The top header MUST NOT show text labels for these actions; the Made identity and workspace-tab folder labels remain text, and workspace tabs MUST retain a close affordance. The header controls MUST float directly over the window clear/native backdrop and MUST share one vertical center and square hit targets, with vertically centered glyphs and the last icon’s glyph centered in its target. Workspace tabs MUST start at the window gutter/card left edge. The header control row MUST span the window gutter to `width - gutter` with no extra horizontal padding, so the last right-side icon’s 26/24 target, selected/hover fill, and focus ring end at the explorer/terminal card right edge with no gutter overhang. The 14px glyph MUST remain centered inside its target with normal optical padding on both sides, so its visible edge is inset from the card border. The wordmark MUST sit to the right of the tab strip immediately before the right icon group. The tab-fitting budget MUST reserve a 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark; with few tabs the visible flexible space between the last visible control and the wordmark is larger because the boundary is reserved, not measured from the last control. The wordmark-to-first-right-icon gap MUST be exactly 24px wide / 18px compact. The whole right cluster MUST be reserved so tabs, `+`, and overflow never overlap it, and the wordmark MUST NOT paint a tab fill, hover, or selection. The top workspace strip and workspace tabs MUST both be 26/24 high while session-map metrics remain unchanged. Header icon actions, optional session tabs, explorer rows and picker rows share one interaction system: hover uses a `BG_HOVER` filled rounded rect with radius 6 and no border; selection uses a `BG_SELECTED` filled rounded rect with no border or accent bar; inactive icons use reduced opacity that returns to full on hover or selection. Workspace tabs use the dedicated selected/hover/idle-fill treatment in the Brand and Workspace Tabs requirement. Expanded explorer folders MUST be marked by a small app-drawn disclosure triangle and a brighter label only, with no border, accent side bar or extra expansion fill. Explorer rows MUST use 24px height, a 4px horizontal inset, 6px radius, a 16px icon slot and text starting 24px from the row's left edge, with depth adding 14px; files keep a quiet app-drawn bullet. Text and disclosure/bullet markers MUST be vertically centered on one optical center in the row. Selection and focus in the tree MUST be conveyed by the inset `BG_SELECTED` fill and `TEXT_PRIMARY` label only, with no focus outline around the row or the tree container. When the tree container receives keyboard focus with no row selected, the first visible row MUST become selected with that same fill and primary text, without opening a preview or changing pointer row activation. Every scroll area (explorer tree, picker list, preview, diagnostics) and the terminal scrollback bar MUST use hover-only floating overlay bars: no track or thumb at rest, a thumb-only fade over about 120–160 ms when the area is hovered, expansion on bar hover/drag with a faint track only then, fade-out on leave, zero allocated width and no layout shift, and no always-on track. Routine integration, shell-running, watcher, version, branch, revision and performance statuses MUST NOT consume persistent banners, pills, path/status rows or empty session strips; technical details belong in Diagnostics. Real shell/input, file/root and explicitly requested action errors MUST remain contextual and non-blocking. The explorer MUST occupy the full right content column with no preview below it, MUST be hideable/restorable through the header Explorer toggle, and MUST have a resizable width constrained to leave a usable left pane. Explorer width MUST be adjusted only by pointer-dragging its divider; the explorer header MUST NOT contain Wider, Narrower, Reload or Hide text actions and MUST NOT offer a routine manual refresh control. At rest the divider MUST NOT paint a persistent line or grip; hover or drag MAY fade in a discreet thin line/grip using muted `BORDER_CONTROL` (and `ACCENT` at restrained opacity while dragging) while retaining a comfortable pointer hit area, with a fade-out on leave. Hiding it MUST give the left surface the reclaimed width without killing the shell, resetting the root or losing tree/preview state; restoring MUST reconcile changes that occurred while hidden. Width and visibility need only be retained for that live workspace, not across app restart. Keyboard reachability, accessible names, visible focus, target sizing and the pointer-only divider exception are owned by the Keyboard Focus and Accessibility requirement; essential controls MUST NOT clip or introduce noisy default technical chrome.

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
- Then expanded folders use a brighter label, rows use 24px height, a 4px horizontal inset, 6px radius, a 16px icon slot and text 24px from the row's left edge with 14px per depth, explorer entry icons are owned by the realized `specs/explorer-file-icons.md` specification, which is authoritative for their resolution and rendering, and replace the former app-drawn file bullet and folder disclosure triangle in that 16px slot, entry icons and labels share one vertical optical center, selection/focus uses only the inset `BG_SELECTED` fill plus `TEXT_PRIMARY` text, keyboard focus with no selection selects the first visible row without opening a preview, and no border, focus outline, accent side bar or extra expansion fill is painted

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
