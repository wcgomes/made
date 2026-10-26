# Change Spec: made-ui-ux-v1

> **Scope:** This active change owns the UI/UX/visual/presentation contract for the undelivered v1, split from `opencode-session-follow-v1`. Product/runtime/integration/security/performance behavior remains in the core change (`specs/changes/opencode-session-follow-v1/spec.md`). This is not a new version, release, or archive. The UI is implemented on Linux with 602 passing tests and Xvfb/headless evidence; native platform, pointer, keyboard, compositor, and manual visual evidence remain pending.

## ADDED Requirements

### Requirement: Calm Editor Visual System

Made MUST use the implemented Calm Editor visual system for the native shell: dark layered surfaces, one amber accent, bundled Ubuntu-Light proportional and Hack monospace typography, a 4/8-point spacing rhythm, 8–10px card radii and gutters, 1px borders, and opaque terminal, preview, picker, diagnostics, tooltip, toast, and other reading surfaces. The top chrome band and the full-height explorer card/header MAY use the documented translucent in-app glass approximation with a painted soft shadow and 1px top highlight. A native macOS/Windows backdrop MAY layer behind the window only through the platform boundary; when unavailable, disabled, or unsupported, the in-app glass MUST remain and the reason MUST be honest. The UI MUST NOT introduce gradients or glows, multiple accents, accent-colored status, opacity on reading surfaces, or text below 10px. The no-webview runtime constraint is owned by the core Responsiveness and Lightness requirement.

#### Scenario: Calm Editor tokens and typography

- Given the running Made UI
- When its visual tokens are inspected
- Then the documented Calm Editor colors, typography roles, spacing, radii, target sizes, and one-accent rule are used, and reading surfaces remain opaque

#### Scenario: Glass chrome and opaque reading surfaces

- Given a workspace with the top band and explorer visible
- When the UI is rendered
- Then the band, explorer card, and explorer header use the documented glass alphas, shadow, and top highlight, while terminal, preview, picker, diagnostics, tooltips, and toast remain opaque and readable when alpha is ignored

#### Scenario: Native backdrop auto and fallback

- Given `MADE_NATIVE_BACKDROP=auto`, `off`, or an unsupported platform/compositor
- When the first UI frame requests or skips the native material
- Then the UI uses the native clear alpha only when an effect was actually applied, otherwise the in-app glass remains with `applied=false` and an honest reason; no native effect is claimed without runtime evidence

#### Scenario: Reading-surface contrast

- Given primary, secondary, tertiary, accent, status, focus, terminal, and control-boundary text and surfaces
- When contrast is measured against the documented pairs
- Then the documented ratios hold for opaque reading surfaces, decorative borders are not the sole control boundary, and native glass/blur contrast is not claimed without evidence

### Requirement: Minimal Workspace Layout

Made MUST present a terminal-first workspace in one native window: workspace tabs above the content area, a selected left surface that is either the terminal or the full-left read-only preview, and a full-height explorer in the right column. Persistent header actions MUST be icon-only buttons with tooltips and accessible labels: Open folder, Explorer toggle, Diagnostics, and the optional Sessions action when shown. The top header MUST NOT show text labels for these actions; the Made identity and workspace-tab folder labels remain text, and workspace tabs MUST retain a close affordance. Routine integration, shell-running, watcher, version, branch, revision and performance statuses MUST NOT consume persistent banners, pills, path/status rows or empty session strips; technical details belong in Diagnostics. Real shell/input, file/root and explicitly requested action errors MUST remain contextual and non-blocking. The explorer MUST occupy the full right content column with no preview below it, MUST be hideable/restorable through the header Explorer toggle, and MUST have a resizable width constrained to leave a usable left pane. Explorer width MUST be adjusted only by pointer-dragging its divider; the explorer header MUST NOT contain Wider, Narrower, Reload or Hide text actions and MUST NOT offer a routine manual refresh control. At rest the divider MUST NOT paint a persistent line or grip; hover or drag MAY reveal a thin line/grip while retaining a comfortable pointer hit area. Hiding it MUST give the left surface the reclaimed width without killing the shell, resetting the root or losing tree/preview state; restoring MUST reconcile changes that occurred while hidden. Width and visibility need only be retained for that live workspace, not across app restart. Keyboard reachability, accessible names, visible focus, target sizing and the pointer-only divider exception are owned by the Keyboard Focus and Accessibility requirement; essential controls MUST NOT clip or introduce noisy default technical chrome.

#### Scenario: Quiet shell workspace

- Given a newly opened folder and no running OpenCode
- When the default workspace is displayed
- Then a usable terminal and full-height right explorer appear without an empty preview, empty session strip, degraded/waiting banner, technical status bar or routine status pills; the header shows only icon-only Open folder, Explorer and Diagnostics actions with accessible labels, and the explorer header shows no text action buttons

#### Scenario: Icon-only header controls

- Given a live workspace at 1280×800 or 720×480
- When the user inspects or keyboard-navigates the top header
- Then Open folder, Explorer toggle, Diagnostics and the visible optional Sessions action are icon-only with matching tooltips and accessible labels, no persistent action text labels are rendered, workspace tabs retain their close affordance, and each action is reachable and visibly focused by keyboard

#### Scenario: Explorer hide and restore

- Given an expanded tree and a running shell, with or without an open preview
- When the user activates the header Explorer toggle to hide, files change on disk, and the user activates it again to restore
- Then the left surface reclaims/releases its width, shell and preview state survive, the restored tree reflects current files while preserving valid expansion/selection, and the toggle remains reachable and focused by keyboard

#### Scenario: Explorer resize and compact layout

- Given either a 1280×800 or 720×480 window
- When the user pointer-drags the explorer divider or navigates the rest of the layout by keyboard
- Then its full-height width changes within usable bounds, the divider is invisible at rest and reveals a thin line/grip on hover or drag, the active left surface reflows without terminal corruption, no Wider/Narrower/Reload/Hide text action is present, essential controls have visible focus and do not clip, and the pointer-only divider is not a keyboard stop

#### Scenario: Explorer header action removal

- Given a visible explorer header
- When the user inspects or navigates it by pointer or keyboard
- Then no Wider, Narrower, Reload or Hide text action is present, and no routine manual refresh control is offered for normal live updates

#### Scenario: Hover-only explorer divider

- Given a visible explorer at rest
- When the divider is not hovered or dragged
- Then no vertical divider line or grip is painted; when hovered or dragged, a thin line/grip appears while the pointer hit area remains usable, and a mere hover does not change the stored per-workspace width

#### Scenario: No manual refresh control

- Given a visible explorer while files change on disk
- When the user looks for refresh actions
- Then no routine manual Reload control exists and creates, deletes and renames still appear automatically through the watcher

#### Scenario: Quiet integration chrome

- Given OpenCode is absent, inactive, disconnected, degraded, or has exited normally
- When the workspace chrome is inspected
- Then no routine degraded/waiting/unsupported banner or status pill is shown, and integration detail remains available only through Diagnostics

### Requirement: Brand and Workspace Tabs

Made MUST present the two-tone monospace `MADE` wordmark with its accent `M` and tooltip identity, followed by editor-like workspace tabs with folder labels, full-path tooltips, and a reachable close affordance. The active workspace tab MUST use a full-opacity selected background plus accent underline, without a dot/hollow marker or active border. Workspace tabs MUST remain visually distinct from optional OpenCode session tabs and MUST NOT animate their width. When tabs do not fit, a labeled, keyboard-reachable overflow list MUST expose every workspace tab; labels MUST truncate with an ellipsis and retain full-value tooltips.

#### Scenario: MADE wordmark identity

- Given the running UI
- When the header brand is inspected
- Then `MADE` is a cached two-tone monospace wordmark with accent `M`, primary `ADE`, the documented tracking, the “Minimal Agentic Development Environment” tooltip, and a divider before the workspace tabs

#### Scenario: Workspace tab presentation

- Given two or more open workspaces
- When the tab strip is displayed
- Then each tab shows its folder label, full-path tooltip, and close affordance; the active tab uses a full-opacity background plus accent underline with no dot/hollow marker or active border; and session tabs are not presented as workspace tabs

#### Scenario: Workspace tab overflow and close

- Given enough workspace tabs to exceed the available width at 1280×800 or 720×480
- When the header lays out
- Then a labeled keyboard-reachable overflow list exposes every tab, the active tab remains visible, no tab or close affordance is clipped, and closing still targets only its workspace

### Requirement: Optional Session Map Presentation

Made MUST present the OpenCode session map as an optional, initially hidden surface scoped to the selected workspace and visually distinct from workspace tabs. It MUST NOT reserve an empty strip before OpenCode data exists or when the map is closed. When opened, it MUST show each session's title and path, exactly one active session, and busy, attention, unread, and renaming indicators using shape plus text, not color alone. Long labels and overflow MUST remain inspectable; the map MUST NOT hide sessions from its overflow list. Session data, focus behavior, and failures are owned by the core spec.

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

Made MUST present the folder picker as an explicit card-based selection surface with the `MADE` brand, a Close X and Escape return path, a Back action for the add-workspace flow, a single rounded selected-row treatment with no left accent bar, a visible path, and primary/secondary folder actions. The picker MUST remain keyboard operable and MUST preserve a live workspace when opened to add another folder. Picker appearance is owned by this UI requirement; the home/`--picker`/explicit-path resolution and no-process-before-confirmation behavior are owned by the core Workspace Launch requirement.

#### Scenario: Picker selection and close

- Given the picker is shown
- When the user inspects and confirms or closes it
- Then the selected row uses one rounded background with no left accent bar, Close X has a tooltip and ≥24px target, Escape/Close return to the live workspace when present or to the default home/terminal fallback, and Back remains available for the add-workspace flow

#### Scenario: Picker keyboard navigation

- Given the picker is focused
- When the user presses Up/Down, Backspace/Left, Enter, Cmd/Ctrl+Enter, or Escape
- Then selection, parent navigation, folder entry, confirmation, and close/return behave as documented, and the list remains operable with thousands of entries

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

Made MUST present Diagnostics as an opaque floating card with a status pill, reason, copy actions, sectioned connection/session/worktree/version/metrics/log detail, bounded log block, and a path/copy footer. The card MUST remain usable at the documented responsive sizes and MUST NOT turn technical details into routine chrome outside the card. Shell-follow diagnostics MUST expose workspace identity, reporting capability/reason, latest shell cwd, effective root/source, and inactive optional integration without treating shell-only use as degraded. Data fields, status semantics, redaction, and log content are owned by the core spec.

#### Scenario: Shell follow diagnostics without chrome noise

- Given a shell-only workspace with supported or unsupported cwd reporting
- When the user opens Diagnostics
- Then its shell-follow detail is presented inside the diagnostics card, routine technical chrome remains hidden outside Diagnostics, and displayed/copied logs contain no secrets

#### Scenario: Diagnostics panel presentation

- Given Diagnostics is open at 1280×800 or 720×480
- When the card is inspected or resized
- Then it uses the opaque card, documented width/height clamps, section labels, monospace values, bounded log block, copy actions, and no clipped essential detail

### Requirement: Responsive and Motion

Made MUST keep the UI usable at 1280×800 and 720×480 with the documented gutters, compact spacing, explorer defaults, truncation, and labeled overflow behavior; no essential control may vanish through clipping. At 900–1099 width the explorer SHOULD default to 35% of the usable content width, and below 900 width the compact gutter, header height, radii, and 288px explorer default apply. Hiding or showing the explorer SHOULD animate its panel width over about 150 ms with quadratic ease-out while content is clipped rather than reflowed, MUST reverse cleanly, and MUST stop tree/splitter input once closed. Motion is limited to spinners, the explorer width transition, and standard control animation; no essential information may exist only in motion, and tab widths MUST NOT animate.

#### Scenario: Responsive breakpoints preserve panes and controls

- Given a window at 1280×800, 900–1099, or 720×480
- When the UI lays out
- Then the left surface and explorer keep usable widths, compact controls use the documented heights/gutters, labels truncate with tooltips, optional controls move to labeled overflow instead of clipping, and preview/return remains usable

#### Scenario: Explorer hide-show animation

- Given the explorer is visible or hidden and the user toggles it
- When the width transition runs, is interrupted, or is reversed
- Then an intermediate width frame exists, tree content is clipped rather than reflowed, the animation settles at the stored width, and the closed state stops tree/splitter input

#### Scenario: Motion does not carry essential information

- Given spinners, the explorer transition, and standard control animation
- When motion is unavailable or reduced
- Then no essential status, selection, action, or error is communicated only through animation

### Requirement: Keyboard Focus and Accessibility

Made MUST make every persistent interactive control keyboard reachable with a visible focus state, never relying on fill change alone. Icon-only actions MUST expose matching tooltip and accessibility-name metadata; workspace and optional session tabs MUST support keyboard entry, Left/Right movement, and Enter/Space activation; the terminal MUST remain one focus stop; the explorer tree, preview actions/return, picker, and diagnostics MUST remain keyboard operable. The pointer-only explorer divider MUST NOT be a keyboard stop; hide/restore remains the keyboard path. Interactive targets MUST be at least 24×24, text MUST not fall below 10px, and status MUST use shape plus text plus color. Platform screen-reader exposure MUST NOT be claimed while the accesskit integration is absent.

#### Scenario: Visible focus on persistent controls

- Given a keyboard-only walkthrough at 1280×800 and 720×480
- When focus moves through header actions, workspace/session tabs, tree, preview actions, picker, and diagnostics
- Then every persistent keyboard-reachable control shows a visible focus ring, the pointer-only divider is skipped, and focus never lands on a clipped or hidden control

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
- Then each state retains a shape, text, or distinct non-color cue in addition to color, and active tabs do not rely on a dot/hollow marker
