# Change Spec: wordmark-diagnostics

## MODIFIED Requirements

### Requirement: Minimal Workspace Layout

Made MUST present a terminal-first workspace in one native window: workspace tabs above the content area, a selected left surface that is either the terminal or the full-left read-only preview, and a full-height explorer in the right column. Persistent header icon actions MUST be icon-only buttons with tooltips and accessible labels: Open folder, Explorer toggle, and the optional Sessions action when shown. The Open folder action MUST live in the workspace tab strip immediately after the last visible tab and before the overflow trigger, use an app-drawn `+` glyph, reserve its width in the tab-fitting budget, and remain reachable with the overflow trigger when tabs overflow. The remaining right icon group MUST order Sessions (when shown) then Explorer left-to-right so Explorer is the rightmost icon. The top header MUST NOT show text labels for those icon actions; the Made identity and workspace-tab folder labels remain text, and workspace tabs MUST retain a close affordance. The header controls MUST float directly over the window clear/native backdrop and MUST share one vertical center and square hit targets, with vertically centered glyphs and the last icon’s glyph centered in its target. Workspace tabs MUST start at the window gutter/card left edge. The header control row MUST span the window gutter to `width - gutter` with no extra horizontal padding, so the last right-side icon’s 26/24 target, selected/hover fill, and focus ring end at the explorer/terminal card right edge with no gutter overhang. The 14px glyph MUST remain centered inside its target with normal optical padding on both sides, so its visible edge is inset from the card border. The wordmark MUST sit to the right of the tab strip immediately before the right icon group. The tab-fitting budget MUST reserve a 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark; with few tabs the visible flexible space between the last visible control and the wordmark is larger because the boundary is reserved, not measured from the last control. The wordmark-to-first-right-icon gap MUST be exactly 24px wide / 18px compact. The whole right cluster MUST be reserved so tabs, `+`, and overflow never overlap it, and the wordmark MUST NOT paint a tab fill, hover fill, or selection treatment. The MADE wordmark MUST be the persistent About trigger. While a workspace is active, activating the wordmark by pointer or keyboard MUST open and close the About card; when no workspace is active, activation MUST NOT open or toggle the About card. The wordmark MUST keep its two-tone MADE brand identity, MUST show a pointer cursor over its target, MUST expose the tooltip and accessibility name `About`, MUST provide a target of at least 24×24, MUST show a visible keyboard focus ring, and MUST activate on Enter or Space. The top workspace strip and workspace tabs MUST both be 26/24 high while session-map metrics remain unchanged. Header icon actions, optional session tabs, explorer rows and picker rows share one interaction system: hover uses a `BG_HOVER` filled rounded rect with radius 6 and no border; selection uses a `BG_SELECTED` filled rounded rect with no border or accent bar; inactive icons use reduced opacity that returns to full on hover or selection. The wordmark trigger is the deliberate exception: it paints no hover or selection fill and conveys focus with its focus ring. Workspace tabs use the dedicated selected/hover/idle-fill treatment in the Brand and Workspace Tabs requirement. Expanded explorer folders MUST be marked by a small app-drawn disclosure triangle and a brighter label only, with no border, accent side bar or extra expansion fill. Explorer rows MUST use 24px height, a 4px horizontal inset, 6px radius, a 16px icon slot inset 4px within the painted row fill (slot starts 8px from the row's left edge) and text starting 28px from the row's left edge, with depth adding 14px; files keep a quiet app-drawn bullet. Text and disclosure/bullet markers MUST be vertically centered on one optical center in the row. Selection and focus in the tree MUST be conveyed by the inset `BG_SELECTED` fill and `TEXT_PRIMARY` label only, with no focus outline around the row or the tree container. When the tree container receives keyboard focus with no row selected, the first visible row MUST become selected with that same fill and primary text, without opening a preview or changing pointer row activation. Every scroll area (explorer tree, picker list, preview, About) and the terminal scrollback bar MUST use hover-only floating overlay bars: no track or thumb at rest, a thumb-only fade over about 120–160 ms when the area is hovered, expansion on bar hover/drag with a faint track only then, fade-out on leave, zero allocated width and no layout shift, and no always-on track. Routine integration, shell-running, watcher, version, branch, revision and performance statuses MUST NOT consume persistent banners, pills, path/status rows or empty session strips; technical details belong in the About card's on-demand diagnostics detail. Real shell/input, file/root and explicitly requested action errors MUST remain contextual and non-blocking. The explorer MUST occupy the full right content column with no preview below it, MUST be hideable/restorable through the header Explorer toggle, and MUST have a resizable width constrained to leave a usable left pane. Explorer width MUST be adjusted only by pointer-dragging its divider; the explorer header MUST NOT contain Wider, Narrower, Reload or Hide text actions and MUST NOT offer a routine manual refresh control. At rest the divider MUST NOT paint a persistent line or grip; hover or drag MAY fade in a discreet thin line/grip using muted `BORDER_CONTROL` (and `ACCENT` at restrained opacity while dragging) while retaining a comfortable pointer hit area, with a fade-out on leave. Hiding it MUST give the left surface the reclaimed width without killing the shell, resetting the root or losing tree/preview state; restoring MUST reconcile changes that occurred while hidden. Width and visibility need only be retained for that live workspace, not across app restart. Keyboard reachability, accessible names, visible focus, target sizing and the pointer-only divider exception are owned by the Keyboard Focus and Accessibility requirement; the wordmark trigger behavior is owned here; essential controls MUST NOT clip or introduce noisy default technical chrome.

#### Scenario: Quiet shell workspace

- Given a newly opened folder and no running OpenCode
- When the default workspace is displayed
- Then a usable terminal and full-height right explorer appear without an empty preview, empty session strip, degraded/waiting banner, technical status bar or routine status pills; the header controls float over the clear/native backdrop without a persistent band and show only icon-only Open folder and Explorer actions with accessible labels, no Diagnostics icon, and the MADE wordmark as the About trigger; and the explorer header shows no text action buttons

#### Scenario: Icon-only header controls

- Given a live workspace at 1280×800 or 720×480
- When the user inspects or keyboard-navigates the top header
- Then Open folder, Explorer toggle and the visible optional Sessions action are icon-only with matching tooltips and accessible labels; the MADE brand text remains as the About trigger with the matching tooltip and accessibility name `About`; no persistent action text labels are rendered; workspace tabs retain their close affordance; and every one of these actions is reachable and visibly focused by keyboard

#### Scenario: Header placement and order

- Given a live workspace at 1280×800 or 720×480, with tabs that fit and a case that overflows
- When the header lays out
- Then workspace tabs start at the window gutter/card left edge, the last right-side icon’s 26/24 target, selected/hover fill, and focus ring end at the card right edge (`width - gutter`) with no gutter overhang and the 14px glyph centered inside the target with normal optical padding, the header row has no extra horizontal padding, the top strip and tabs are 26/24 high with the card top at 42/36 and one 8/6 pane gap above and below the strip, the tab-fitting budget reserves the documented 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark (the visible space after the last visible control may be larger), the wordmark-to-first-icon gap is exactly 24px wide / 18px compact, the whole right cluster is reserved so tabs/plus/overflow never overlap it, the app-drawn `+` Open folder action follows the last visible tab and precedes the overflow trigger, the overflow trigger and `+` both remain reachable, the right icon group orders Sessions (when available) then Explorer with Explorer rightmost and no Diagnostics icon while retaining tooltips, accessible names, minimum targets, and keyboard focus, and the MADE wordmark stays immediately before that group with a target of at least 24×24 and the `About` tooltip and accessibility name

#### Scenario: Wordmark About trigger

- Given a live workspace at 1280×800 or 720×480, then the shell with no active workspace
- When the user activates the MADE wordmark by pointer, then by keyboard focus plus Enter, then by keyboard focus plus Space, then without an active workspace
- Then each activation while a workspace is active opens and closes the About card between closed and open, and activation without an active workspace does not open or close the card

#### Scenario: Unified hover and selection states

- Given header icon actions, the MADE wordmark About trigger, optional session tabs, workspace tabs, explorer rows, and picker rows
- When each is hovered, selected, focused, or at rest
- Then header icons, session tabs, explorer rows, and picker rows paint their documented borderless radius-6 `BG_HOVER` hover and `BG_SELECTED` selection fills; the MADE wordmark paints no `BG_HOVER`, `BG_SELECTED`, `BG_TAB_HOVER`, or `BG_TAB_SELECTED` fill and conveys keyboard focus with its visible focus ring; workspace tabs use the translucent idle fill and opaque `BG_TAB_HOVER` hover fill with no stroke, and the opaque `BG_TAB_SELECTED` fill with the faint 1px inside stroke; selected icon toggles paint no border or accent bar; tree and picker selection paint no focus outline; inactive tabs/icons use the reduced-opacity treatment; and focus rings, tooltips, accessible names, and minimum targets remain intact on every control except the deliberate tree/picker row exception

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
- Then no routine degraded/waiting/unsupported banner or status pill is shown, and integration detail remains available only through the About card's on-demand diagnostics detail

### Requirement: Brand and Workspace Tabs

Made MUST present editor-like workspace tabs starting at the window gutter/card left edge, with folder labels, full-path tooltips, and a reachable close affordance. The header row MUST span the card left/right edges with no extra horizontal padding. The last right-side icon’s 26/24 target, selected/hover fill, and focus ring MUST all end at the card right edge with no gutter overhang; the 14px glyph MUST remain centered inside its target with normal optical padding on both sides. The two-tone monospace `MADE` wordmark with its accent `M` and its `About` trigger identity MUST sit to the right of the tab strip, immediately before the right icon group, with a reserved 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark (the visible flexible space after the last visible control is larger when few tabs are shown), the same exact 24px wide / 18px compact gap from the wordmark to the first right icon, and no divider. The wordmark MUST NOT look like a tab and MUST NOT paint a fill, hover, or selection treatment. It MUST be the persistent About trigger defined by the Minimal Workspace Layout requirement, which opens the About card and its on-demand diagnostics detail, with the `About` tooltip and accessibility name, a pointer cursor, a target of at least 24×24, a visible keyboard focus ring, and Enter/Space activation. The top workspace strip and workspace tabs MUST both be 26/24 high; the session-map strip/tabs keep their previous 36/34 and 28/26 metrics unless they directly depend on the reduced top height. The selected workspace tab MUST use the opaque `BG_TAB_SELECTED #1B231F` dark green-gray fill with a `TEXT_PRIMARY` 13.0px label, the faint 1px inside stroke, and no underline, dot/hollow marker, accent bar, or width animation. Selection MUST also carry a subtle non-color cue: the selected label is set at 13.0px versus 12.5px for inactive tabs, vertically centered, with tab width and layout unchanged and no animation; the selected fill is also a luminance step above the idle/hover fills. Inactive workspace tabs MUST use the translucent black `BG_TAB_IDLE` fill, no stroke, and secondary text at reduced opacity. Hovering an inactive workspace tab MUST replace the idle fill with the slightly darker opaque `BG_TAB_HOVER #151C18` fill, paint no stroke, and restore the label to full-opacity `TEXT_PRIMARY`; a selected tab keeps `BG_TAB_SELECTED` and the faint inside stroke on hover. The tab close affordance MUST keep its 24×24 hit area and keyboard focusability reserved at all times, but its `×` glyph MUST be hidden at rest—including on the selected tab—and MUST appear only while the tab is pointer-hovered or while the tab or close action has keyboard focus. The close affordance MUST NOT paint a hover background fill; hovering only the close icon MUST turn the visible glyph `ERROR` and MUST also apply the whole tab's hover fill and label treatment without changing tab width, label truncation, or layout. The glyph MUST use the fixed documented size with no scale or width animation. Workspace tabs MUST remain visually distinct from optional OpenCode session tabs and MUST NOT animate their width. When tabs do not fit, a labeled, keyboard-reachable overflow list MUST expose every workspace tab; labels MUST truncate with an ellipsis and retain full-value tooltips.

#### Scenario: MADE wordmark identity

- Given the running UI
- When the header brand is inspected or activated by pointer or keyboard
- Then workspace tabs start at the window gutter/card left edge, the last right-side icon’s 26/24 target, selected/hover fill, and focus ring end at the card right edge with the 14px glyph centered inside the target with normal optical padding, the top strip and tabs are 26/24 high with one 8/6 pane gap above and below, and `MADE` is a cached two-tone monospace wordmark with accent `M`, primary `ADE`, the documented tracking and mint accent, the `About` tooltip and accessibility name, a pointer cursor, a target of at least 24×24, a visible keyboard focus ring, Enter/Space activation that opens the About card, a reserved 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark (visible space after the last visible control may be larger), an exact 24px wide / 18px compact wordmark-to-first-icon gap, no divider, no tab fill/hover/selection, and no `BG_HOVER`, `BG_SELECTED`, `BG_TAB_HOVER`, or `BG_TAB_SELECTED` fill

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

### Requirement: Keyboard Focus and Accessibility

Made MUST make every persistent interactive control keyboard reachable with a visible focus state, never relying on fill change alone. Icon-only actions MUST expose matching tooltip and accessibility-name metadata; the MADE wordmark About trigger MUST expose the matching tooltip and accessibility name `About`, show a visible focus ring, and open or close the About card on Enter or Space while a workspace is active. Workspace and optional session tabs MUST support keyboard entry, Left/Right movement, and Enter/Space activation; the terminal MUST remain one focus stop; the explorer tree, preview actions/return, picker, and the About card's diagnostics detail MUST remain keyboard operable. Focusing the tree before any row is selected MUST select the first visible row (fill plus primary text only, no preview). The explorer tree row/container and picker rows are a deliberate focus-outline exception: while focus moves there, it MUST be conveyed by the inset `BG_SELECTED` fill plus `TEXT_PRIMARY` text (luminance and text, not hue alone) with no outline; every other persistent control, including the wordmark trigger, the About diagnostics toggle, the picker card, and picker buttons, MUST keep its visible focus ring. The wordmark's focus cue MUST be its visible focus ring, not a fill change. The pointer-only explorer divider MUST NOT be a keyboard stop; hide/restore remains the keyboard path. Interactive targets MUST be at least 24×24, text MUST not fall below 10px, and status MUST use shape plus text plus color. Platform screen-reader exposure MUST NOT be claimed while the accesskit integration is absent.

#### Scenario: Visible focus on persistent controls

- Given a keyboard-only walkthrough at 1280×800 and 720×480
- When focus moves through the MADE wordmark About trigger, header actions, workspace/session tabs, tree, preview actions, picker, and the About card's diagnostics detail
- Then every persistent keyboard-reachable control shows a visible focus ring except tree rows/container and picker rows, which show only the inset `BG_SELECTED` fill and `TEXT_PRIMARY` text; the wordmark shows its focus ring while painting no fill and opens or closes About on Enter or Space while a workspace is active; a tree container focused before any selection selects its first visible row without opening a preview; the pointer-only divider is skipped, and focus never lands on a clipped or hidden control

#### Scenario: Icon actions expose accessible names

- Given the icon-only Open folder, Explorer, and optional Sessions actions plus the MADE wordmark About trigger and the About card's diagnostics toggle
- When each is inspected by keyboard or accessibility metadata
- Then each icon-only action has a matching tooltip, accessible name, minimum target, and visible focus; the wordmark trigger has the matching tooltip and accessibility name `About`, a target of at least 24×24, and a visible focus ring; the diagnostics toggle has a matching accessible name and visible focus; and no action relies on its icon or brand text alone as its label

#### Scenario: Keyboard workspace and session navigation

- Given multiple workspace tabs and an open optional session map
- When the user enters the tab strip, moves Left/Right, and presses Enter/Space
- Then the intended workspace or session activates without leaking focus or input to an inactive surface, and the terminal remains a single focus stop

#### Scenario: Status cues do not rely on color

- Given busy, attention, unread, renaming, connected, degraded, shell-exit, tree-error, and selected states
- When the UI is inspected without color or through a colorblind simulation
- Then each state retains a shape, text, or distinct non-color cue in addition to color, selected/active tabs use the 13.0px label cue versus 12.5px inactive labels, and active tabs do not rely on a dot/hollow marker

### Requirement: Diagnostics Presentation

When revealed from About, Diagnostics MUST be presented as the existing opaque Diagnostics card with a status pill, reason, copy actions, sectioned connection/session/worktree/version/metrics/log detail, bounded log block, and a path/copy footer. Under `LOG`, the card MUST show a compact Previous log path row. When the previous log exists, the row MUST offer a "Copy previous log path" action, show the full path in a tooltip, and remain truncation-safe at 720×480; when it does not exist, the row MUST show a quiet "not yet rotated" state with that action disabled. The existing active-log path and copy row MUST remain. A retention warning or error present while the diagnostics detail is rendering (for example the startup legacy-size warning) MUST use the warning or error tone inside the card. Final-exit write/rotation failures are stderr-only and MUST NOT be claimed as in-card notices. The card MUST remain usable at the documented responsive sizes and MUST NOT turn technical details into routine chrome outside the card. Shell-follow diagnostics MUST expose workspace identity, reporting capability/reason, latest shell cwd, effective root/source, and inactive optional integration without treating shell-only use as degraded. Data fields, status semantics, redaction, retention, and log content are owned by the core spec.

#### Scenario: Shell follow diagnostics without chrome noise

- Given a shell-only workspace with supported or unsupported cwd reporting
- When the user opens About and reveals its diagnostics detail
- Then its shell-follow detail is presented inside the About card, routine technical chrome remains hidden outside About, and displayed/copied logs contain no secrets

#### Scenario: Diagnostics panel presentation

- Given About is open with diagnostics revealed at 1280×800 or 720×480
- When the card is inspected or resized
- Then it uses the opaque card, documented width/height clamps, section labels, monospace values, bounded log block, copy actions, and no clipped essential detail

#### Scenario: Previous log path and retention notice

- Given About is open with diagnostics revealed and the previous diagnostics log exists, or has not yet been rotated
- When the user inspects or copies the Previous log row
- Then the full previous-log path is available (truncation-safe with a tooltip), copying it works only when the file exists, an absent file shows a quiet "not yet rotated" state with the action disabled, a retention warning or error present while the card is rendering uses the warning or error tone inside the card, and final-exit failures are stderr-only rather than claimed in-card

## ADDED Requirements

### Requirement: About Presentation

Made MUST present About as an opaque floating card opened by the header MADE wordmark. About MUST show the MADE wordmark, the Made application name and version, and a short product description, with a labeled `Show diagnostics` button. The status pill, reason, copy diagnostics actions, notices, sectioned connection/session/worktree/version/metrics/log detail, bounded log block, previous-log row, active-log path/copy row, and other technical detail MUST remain hidden until the user activates `Show diagnostics`; activating it MUST reveal the existing Diagnostics content in the same card and change the button label to `Hide diagnostics`, which hides that detail again. While diagnostics are revealed, the toggle MUST use the active accent treatment; while hidden, it MUST use the secondary treatment. The diagnostics detail MUST reveal with a smooth fade and height growth over about 220 ms and reverse when hidden. Closing and reopening About MUST return it to the default About-only state with diagnostics hidden. The About card MUST be a fixed, centered, non-resizable, non-movable card with no resize or move affordance. While diagnostics are hidden it MUST use a compact height; when diagnostics are revealed it MUST grow to use the available application height, with the diagnostics body taking the remaining space, and return to the compact height when hidden or reopened. The About card MUST remain usable at the documented responsive sizes, and the diagnostics toggle MUST be keyboard reachable with a visible focus ring and matching accessible name.

#### Scenario: About default content

- Given a live workspace at 1280×800 or 720×480
- When the user opens About from the MADE wordmark
- Then the card shows the MADE wordmark, the Made application name and version, the short product description, and a `Show diagnostics` button in the secondary treatment, while the status pill, diagnostics sections, bounded log block, previous-log row, active-log row, and copy diagnostics actions remain hidden

#### Scenario: About diagnostics reveal and reset

- Given About is open with diagnostics hidden
- When the user activates `Show diagnostics`, then `Hide diagnostics`, then closes and reopens About
- Then the first activation reveals the existing Diagnostics detail in the same card, changes the button to `Hide diagnostics`, and switches that button to the active accent treatment; the second hides that detail, returns the button to the secondary treatment, and restores the compact card; and reopening About shows only the default About content with diagnostics hidden

#### Scenario: About diagnostics reveal animation

- Given About is open with diagnostics hidden at 1280×800 or 720×480
- When the user activates `Show diagnostics`, then `Hide diagnostics`
- Then the detail fades in while the card grows smoothly over about 220 ms, the reveal passes through intermediate heights rather than jumping, and hiding reverses the animation back to the compact About card

#### Scenario: About height and resize behavior

- Given About is open at 1280×800 or 720×480
- When the user inspects the card with diagnostics hidden, then reveals diagnostics, then hides them again
- Then the card has no resize or move affordance, stays centered, uses a compact height while diagnostics are hidden, grows to use the available application height when they are revealed, and returns to the compact height when they are hidden or About is reopened
