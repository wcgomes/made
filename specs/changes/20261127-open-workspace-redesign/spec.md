# Change Spec: open-workspace-redesign

This delta adds eight requirements to `specs/made-ui-ux.md` (`Open Workspace Screen`, `Open Workspace Host Bar`, `Open Workspace Folder Navigation`, `Open Workspace Launch Bar`, `Open Workspace Git Indicator and Worktrees`, `Open Workspace Change Pills`, `Hardened read-only git reads`, `Program-neutral wording`) and one requirement to `specs/ssh-remote-workspaces.md` (`Remote Open workspace browsing and Git parity (SSH-12)`). It modifies `specs/made-ui-ux.md` (requirements `Calm Editor Visual System`, `Minimal Workspace Layout`, `Grouped changes`, `Optional Session Map Presentation`, `Diagnostics Presentation`, `Brand and Workspace Tabs` and `Full-Left Preview Presentation`), `specs/opencode-session-follow.md` (requirement `Degraded Integration`), `specs/grok-session-follow.md` (requirements `Catalog-driven new-tab menu`, `OpenCode diagnostics wording` and `Grok tab dots`), `specs/claude-code-session-follow.md` (requirements `Claude Code launch`, `Claude Code tab presentation` and `Claude Code channel fallback`) and `specs/ssh-remote-workspaces.md` (requirements `SSH server entry and launch choices (SSH-01)`, `Remote OpenCode session and cwd follow (SSH-06)`, `Local preservation and truthful parity evidence (SSH-09)`, `Remote Grok Build session follow and tab parity (SSH-10)` and `Remote Claude Code launch and status (SSH-11)`), and `specs/agent-integration-architecture.md` (requirement `Declared precedence (AIA-05)`). It removes `Explicit Folder Picker` from `specs/made-ui-ux.md`; its tree, keyboard, refresh and close guarantees are carried into `Open Workspace Screen` and `Open Workspace Folder Navigation`. Each MODIFIED requirement below replaces the realized requirement of the same name in full, scenarios included; text not named in the proposal's change list is verbatim. SSH-04, SSH-07 and SSH-08 are not modified: their guarantees apply unchanged, and the terminology rule of `Open Workspace Screen` and of SSH-01 maps their references to rows, the `+` menu and remote launch choices onto the new screen.

## ADDED Requirements

### Requirement: Open Workspace Screen

Made MUST present one Open workspace screen as the single surface for choosing a host, a folder and a launch program for a new workspace tab. It MUST be shown when the user activates the app-drawn `+` control, at startup with `--picker` (an explicit workspace path still takes precedence under the core `Workspace Launch` requirement), after the last workspace tab closes, and as the test-only smoke surface without a folder. Activating `+` MUST open this screen directly and MUST NOT show an intermediate menu, whether or not any integrated TUI is detected or any SSH server is saved. This screen is the surface the live contract calls the folder picker: every requirement this change does not modify that refers to the folder picker, picker, picker card, picker surface, picker list, picker rows, picker wordmark or add-workspace flow, or to the Open folder action, MUST be read as referring to this screen, its folder rows and the `+` control; a reference to choosing a row or item of the `+` menu or `+` new-tab menu MUST be read as activating the corresponding launch button of this screen.

The screen MUST be a card that grows with the window in both dimensions, inset from the window edges by 48px horizontally and 32px vertically in the wide layout and 16px horizontally and 12px vertically in the compact layout (below 900px width), so the window backdrop shows around it; the card uses the conditional glass defined for it by `Calm Editor Visual System`. It carries the brand-only `MADE` wordmark and version, an icon-only Close X with tooltip, and Escape; it MUST NOT show a Back action. Escape (when no text field is being edited) and Close MUST return to the live workspace when one exists and otherwise open the default home/terminal fallback. Opening the screen, cancelling it and launching from it MUST leave every existing tab's shell, processes and state untouched. From top to bottom it MUST present the host bar, a notice line, the folder navigation (toolbar with address bar, then folder tree), the git and worktree area when applicable, and the launch bar. The launch bar MUST remain visible without scrolling the screen at 1280×800 and 720×480, and at 720×480 the folder tree MUST keep at least four visible rows while the worktree list is shown. No local shell, explorer or TUI process and no remote terminal session MUST start before the user activates a launch button; selecting an SSH host MAY start that server's SSH connection and companion helper for browsing only, as SSH-12 defines. When a launch starts, the screen MUST close and the new tab MUST be selected once its runtime is ready; a launch that fails validation MUST keep the screen open with a contextual, non-blocking error.

The notice line MUST show, as text in `ERROR` color with an error glyph so it does not rely on color alone, every unresolved failure of the selected SSH host: its connection or authentication, host-key trust, staging consent, companion staging, opening the browse grant, and any browse operation (listing, address check or git read), for example `A packaged companion file is missing. Made did not start a local shell.`, as well as a failed connection test (SSH-01) and a launch that fails validation. While such a failure is unresolved the notice line MUST NOT be blank. Each notice MUST be dismissible through a keyboard-reachable close control and by Escape while the notice is focused, and MUST clear when another host is selected or when a later operation of the same kind succeeds. The host button's state badge, tooltip and accessible name MUST still reflect the failure after the notice is dismissed. With no failure, the notice line shows nothing and a successful operation adds no text.

When the screen opens, the selected host MUST be the host of the active workspace tab, or Local when no workspace is active or the active tab's server record no longer exists. Screen state (selected host, folder, worktree, address text) MUST NOT persist across app restart. Keyboard: Tab MUST move host bar → address bar → folder tree → worktree list (when shown) → launch bar → Close; Cmd/Ctrl+Enter MUST activate the Terminal launch button for the current effective launch path. Folder rows and worktree rows convey focus and selection only by the inset `BG_SELECTED` fill plus `TEXT_PRIMARY` text with no outline, the same deliberate exception as picker rows; every other control on the screen MUST show its focus ring, expose a tooltip and accessible name when it is icon-only, and have a target of at least 24×24. No text on the screen MUST be below 10px.

#### Scenario: Plus opens the screen directly

- Given OpenCode, grok and claude are on the local PATH and two SSH servers are saved, then a variant with no TUI detected and no server saved
- When the user activates `+` by pointer and by keyboard (Enter or Space)
- Then the Open workspace screen opens in both variants, no menu is shown, the `+` tooltip and accessible name are `Open workspace`, and the live workspace keeps running

#### Scenario: One surface for startup, last-tab close and add-workspace

- Given Made started with `--picker`, then a window whose last tab was just closed, then a live workspace where the user activates `+`
- When the screen is shown in each case
- Then it is the same Open workspace screen with host bar, folder navigation and launch bar; no Back action is shown in any case; and no shell, explorer or TUI process has started

#### Scenario: Cancel preserves the workspace

- Given a live local workspace with a running shell and a remote workspace tab
- When the user opens the screen, changes host and folder, then presses Escape, and separately opens it and activates Close X, and separately activates Close X with no workspace live
- Then no Back action is shown, Escape and Close X return to the workspace with its shell, scrollback, root and remote session untouched and nothing launched, and Close with no live workspace opens the default home/terminal fallback

#### Scenario: Compact layout keeps the launch bar visible

- Given a 720×480 window, a selected repository with five linked worktrees and three detected TUIs, then the same at 1280×800
- When the screen lays out
- Then the host bar, address bar, at least four folder rows, the worktree list (scrolling within its own area) and the full two-row launch bar are visible without scrolling the screen, nothing clips or overlaps, and every control is reachable by keyboard in the documented Tab order

#### Scenario: Browse failure shows a dismissible error notice

- Given server A whose staged companion set is missing a packaged file, and Local
- When the user selects A, inspects the screen without color, dismisses the notice with its close control by keyboard, selects A again, then selects Local, and in a variant fixes the companion and retries
- Then the notice line shows `A packaged companion file is missing. Made did not start a local shell.` in `ERROR` color with an error glyph, A's badge, tooltip and accessible name show the failure, the notice closes by its control and by Escape while focused while A's badge still shows the failure, it reappears on reselecting A while the failure is unresolved, it clears when Local is selected and after the successful retry, and no notice text appears for any success

#### Scenario: Card grows with the window and keeps margins

- Given the screen at 720×480, 1280×800 and 1920×1200
- When the window is resized between those sizes
- Then the card grows and shrinks in both dimensions, keeps 16px/12px margins in the compact layout and 48px/32px in the wide layout with the backdrop visible around it, and at 720×480 at least four folder rows and the whole launch bar stay visible

#### Scenario: Default host follows the active tab

- Given an active remote tab on server A, then an active local tab, then no active workspace, then an active tab whose server record was removed
- When the screen opens in each case
- Then the selected host is A, then Local, then Local, then Local

### Requirement: Open Workspace Host Bar

The top of the Open workspace screen MUST show one row of host buttons: Local first, always present and never removable, then one button per saved SSH server in saved-record order, then a `+ Add host` button last. Exactly one host MUST be selected; the selected host button uses the inset `BG_SELECTED` fill and `TEXT_PRIMARY` text and the others the shared hover treatment. Each host button MUST show a leading 16px host-kind icon, the host name, and the host address on a second line in monospace secondary text. The Local icon MUST be an app-drawn computer glyph and the SSH icon an app-drawn server glyph whose shapes differ, so the host kind never relies on color alone. Local MUST show the name `Local` and the local machine's host name as address, or `This computer` when the host name cannot be resolved. An SSH host MUST show its saved display name and the address `[user@]host[:port]` from its record, with the user only when set and the port only when set. Names and addresses that do not fit MUST be truncated with an ellipsis, and the tooltip and accessible name MUST give the kind, full name and full address.

Each SSH host button MUST show its connection state as a small status glyph plus text in its tooltip and accessible name: `Not connected`, `Connecting`, `Connected`, `Host key approval needed`, `Authentication cancelled` or `Connection failed: <reason>`; Local shows no connection state. Each host button, Local included, MUST show the marks of the catalog TUIs detected on that host, in catalog order (OpenCode, Grok Build, Claude Code), at 12px after the name, omitting undetected programs; while that host's detection has not completed it shows no marks and its tooltip says detection is pending. Detection results of one host MUST NOT be shown on another.

When the buttons exceed the screen width the row MUST scroll horizontally with the floating overlay scrollbar rule (hidden at rest, zero allocated width), MUST NOT wrap, MUST keep the selected and the focused host in view, and `+ Add host` MUST remain reachable. Left/Right MUST move focus within the row and Enter/Space MUST select the focused host or activate `+ Add host`. Selecting a host MUST reset the folder navigation to that host's start folder (Local: the start directory the folder picker uses; SSH: the record's initial remote directory, else the remote user's home directory as reported by the server), clear the worktree selection, and show that host's launch buttons. SSH host management actions are owned by SSH-01.

#### Scenario: Local first and distinguishable kinds

- Given two saved SSH servers, `prod` with user `deploy`, host `prod.example.org` and port 2222, and `lab` with host alias `lab` only
- When the screen opens
- Then the host bar shows Local first with the computer glyph, `Local` and the machine host name, then `prod` with the server glyph and `deploy@prod.example.org:2222`, then `lab` with `lab`, then `+ Add host`; Local and SSH glyphs differ in shape, and each tooltip and accessible name states the kind, name and address

#### Scenario: Per-host state and detected TUIs

- Given Local has OpenCode and claude detected, server A is connected with grok detected, and server B failed authentication
- When the host bar is inspected without color
- Then Local shows the OpenCode and Claude Code marks, A shows the Grok Build mark and a connected state glyph with `Connected` in its tooltip, B shows no marks and a failure glyph with `Connection failed:` and the reason in its tooltip, and no host shows another host's marks

#### Scenario: Many hosts scroll instead of wrapping

- Given twelve saved SSH servers at 720×480
- When the user moves focus with Left/Right through the host bar to `+ Add host` and back
- Then the row never wraps, the focused host scrolls into view, no scrollbar is painted at rest, and `+ Add host` is reachable

#### Scenario: Selecting a host resets navigation

- Given Local is selected with a nested folder and a worktree selected, and server A has initial remote directory `/srv/app`
- When the user selects A, then server B with no initial directory
- Then the tree re-roots at `/srv/app` on A with no selection and no worktree selection, then at B's remote home directory, and the launch bar shows each server's own buttons

### Requirement: Open Workspace Folder Navigation

Below the host bar the screen MUST show a toolbar with Up and Refresh icon actions and an editable address bar, and below it an inline expandable folder tree of the selected host rooted at the current root. The tree MUST keep the realized picker tree behavior except row height and disclosure mark: the root's child folders appear as depth-0 rows; expanding inserts child folders below their parent indented 14px per depth and keeps every ancestor visible; collapsing removes only that subtree; rows are 24px high, the same height as the main Explorer tree rows (the realized picker used 32px), with a 4px inset, radius-6 `BG_HOVER` hover and inset `BG_SELECTED` selected fills, primary selected text, an app-drawn thin stroked disclosure chevron (an open polyline with no fill, about 8px tall and 4px deep, about 1.25px stroke with round caps, `TEXT_TERTIARY` at rest and `TEXT_SECONDARY` on hovered or selected rows, pointing right when collapsed and down when expanded, optically centered on the label) in place of the realized picker's filled triangle, and no row or container focus outline; selection is one flattened visible-row index; clicking a folder selects it and toggles its expansion, and a double-click toggles once and never launches; Up/Down move the visible-row selection (first/last from no selection), Right expands the selected folder or selects its first child when expanded, Left collapses it or selects its parent row when collapsed, Enter toggles expansion without launching, and Backspace or the Up action re-roots to the parent directory and clears expansion and selection; re-rooting starts the list at the top; expanding or moving selection reveals the row with minimal scrolling that keeps the visible ancestor chain when it fits; Refresh re-reads the root and every expanded directory, preserving valid expansion and selection by path and pruning removed paths; each directory read keeps the 1000-entry cap with the existing truncation notice; an unreadable child stays collapsed with a non-blocking error; the tree uses the floating overlay scrollbar and stays operable with thousands of rows.

The launch target MUST be the selected folder row, or the current root when nothing is selected. The address bar MUST show the launch target's full path in `TEXT_PRIMARY` monospace on an inset `BG_SELECTED` field with no border and a full-path tooltip, and MUST follow the launch target. The address bar MUST be editable: clicking it or focusing it and typing enters edit mode with the text selected, and the user can type or paste a path. Enter MUST navigate: surrounding whitespace and trailing separators are ignored, and when the text is an absolute path to an existing readable directory on the selected host the tree MUST re-root at that directory with no selection or expansion, the list at the top, and that directory as the launch target. A relative, nonexistent, unreadable or non-directory path MUST leave the root, selection and launch target unchanged, keep the typed text for correction, and show an inline non-blocking error naming the reason (`Path must be absolute`, `Folder not found`, `Folder is not readable` or `Not a folder`). Escape while editing MUST restore the launch target's path and leave edit mode without closing the screen. For an SSH host the path MUST be checked on that server under SSH-12 and MUST NOT be resolved against the host filesystem. The former `Use current folder` and `Open selected folder` actions are replaced by the launch bar.

#### Scenario: Tree behavior carried from the picker

- Given a selected host folder with child and grandchild folders and more than 1000 entries in one directory
- When the user clicks, double-clicks, uses Up/Down/Right/Left/Enter/Backspace, refreshes and re-roots
- Then rows are 24px high (not the former 32px), each folder shows the thin stroked chevron (no fill) pointing right when collapsed and down when expanded, `TEXT_TERTIARY` at rest and `TEXT_SECONDARY` when hovered or selected, optically centered on the label, and indentation, fills, single flattened selection, minimal reveal scrolling, refresh preservation, the 1000-entry cap notice and the unreadable-child error behave as the realized picker tree did, and no workspace is launched by any of these actions

#### Scenario: Pasting a full path jumps to it

- Given Local is selected with the tree rooted at the home directory
- When the user pastes `/Users/me/Repos/made/` with a trailing space into the address bar and presses Enter
- Then the tree re-roots at `/Users/me/Repos/made` with no selection, the list starts at the top, the address bar shows that path, and the launch target is that folder

#### Scenario: Invalid address keeps state

- Given a tree with a selected folder
- When the user enters `Repos/made`, then a path that does not exist, then a path to a regular file, then an unreadable directory, then presses Escape
- Then each attempt shows its inline reason (`Path must be absolute`, `Folder not found`, `Not a folder`, `Folder is not readable`), the root, selection and launch target never change, the typed text stays editable, and Escape restores the launch target path without closing the screen

#### Scenario: Launch target follows selection

- Given a tree rooted at `/work` with no selection
- When the user selects `/work/api`, then clears the selection by re-rooting with Up
- Then the address bar and launch target show `/work/api`, then the new root `/`

### Requirement: Open Workspace Launch Bar

Below the folder navigation and the git and worktree area the screen MUST show a launch bar in its footer, of two rows. Row 1 MUST be the launch target display, left-aligned at the footer's content inset: the label `Launch in`, the git indicator when applicable, the effective launch path in monospace, middle-ellipsized when it does not fit, with the full path in its tooltip and accessible name, and the change pills when they are shown beside the indicator. Row 2 MUST hold the launch buttons, horizontally centered in the footer, each with a 16px leading icon and a text label. The rows MUST never overlap, row 1 MUST never exceed the footer width, and the buttons MUST NOT move when the path, git indicator or pills change: when space is short the path MUST shorten first, and pills that still do not fit MUST be hidden from the end of row 1, with the git indicator's tooltip and accessible name then listing the hidden counts. `Terminal`, with the app-drawn terminal glyph, MUST be first and always present; then one button per integrated TUI detected on the selected host, in catalog order OpenCode (`opencode.svg`), Grok Build (`grok.svg`), Claude Code (`claude-code.svg`), omitting programs not detected on that host. The effective launch path MUST be the selected worktree when the worktree list is shown, otherwise the launch target. Launch buttons MUST be enabled only when there is an effective launch path and, for an SSH host, while that server is connected; a disabled button MUST say why in its tooltip. Launch buttons MUST be reachable by Tab and Left/Right and activate on Enter or Space. Every launch button, Terminal and each TUI alike, MUST share one identical style: radius 6, height 26px (24px in the compact layout), no border or stroke; at rest a `BG_SELECTED` fill with a `TEXT_SECONDARY` label; on hover an `ACCENT_TINT` fill with a `TEXT_PRIMARY` label; while pressed a `BG_HOVER` fill with a `TEXT_PRIMARY` label; and for keyboard focus the existing focus treatment of a 2px gap and a 2px `FOCUS` ring. Hover and pressed MUST change only the fill and label color. No launch button MUST receive accent, primary or other emphasis beyond that shared `ACCENT_TINT` hover, including an `ACCENT` fill, accent text, a different size or a distinct rest fill; Terminal is distinguished only by being first and by being the Cmd/Ctrl+Enter target. A disabled button keeps the same shape with reduced opacity and no hover change.

Activating `Terminal` for Local MUST open a new local workspace tab whose shell starts in, and whose explorer is rooted at, the effective launch path, with the existing confirmation semantics and process boundaries of the core `Workspace Launch` requirement. Activating a TUI button for Local MUST start that program as the catalog requirement for it defines, in a new tab whose launch folder and explorer root are the effective launch path. Activating a button for an SSH host MUST start the corresponding SSH launch of SSH-01 bound to that server and the effective launch path. In every case the current tab MUST keep running and no further selection surface is shown. Launching MUST NOT wait for pending git or worktree reads.

#### Scenario: Terminal first, then detected TUIs in catalog order

- Given Local with OpenCode, grok and claude on PATH, then Local with only claude, then server A with only grok detected
- When each host is selected
- Then the launch bar is Terminal, OpenCode, Grok Build, Claude Code; then Terminal, Claude Code; then Terminal, Grok Build; each button shows its 16px mark and label

#### Scenario: Local terminal launch in the chosen folder

- Given a live tab and Local selected with launch target `/work/api` and no worktree list
- When the user activates Terminal, or presses Cmd/Ctrl+Enter
- Then a new tab opens whose shell cwd and explorer root are `/work/api`, the previous tab keeps running, and the screen closes

#### Scenario: Local TUI launch in the chosen folder

- Given Local selected with launch target `/work/api` and claude on PATH
- When the user activates Claude Code
- Then Made starts claude's absolute path with no arguments directly as the child of a new tab whose launch folder and explorer root are `/work/api`, with no shell first, and the current tab keeps running

#### Scenario: Launch buttons share one quiet style

- Given Local selected with OpenCode, grok and claude detected
- When the launch buttons are inspected at rest, hovered, pressed and keyboard-focused
- Then Terminal, OpenCode, Grok Build and Claude Code have the same radius 6 and 26px height (24px compact) with no border or stroke, rest is `BG_SELECTED` with a `TEXT_SECONDARY` label, hover is `ACCENT_TINT` with a `TEXT_PRIMARY` label, pressed is `BG_HOVER` with a `TEXT_PRIMARY` label, keyboard focus shows the 2px gap and 2px `FOCUS` ring, no button uses an accent or primary treatment, Terminal is first, and Cmd/Ctrl+Enter activates Terminal

#### Scenario: Target row left-aligned and buttons centered

- Given a 720×480 window and then 1280×800, Local selected with a short path and then a 200-character effective launch path in a repository without linked worktrees that has conflicted, staged, modified and untracked changes, and three detected TUIs
- When the launch bar lays out and the path changes between the short and the long one
- Then row 1 starts at the footer's content inset with `Launch in`, the git indicator, the path in monospace middle-ellipsized with the full path in its tooltip and accessible name, and the pills that fit; row 2 shows Terminal, OpenCode, Grok Build and Claude Code centered in the footer at their normal size; the buttons do not move when the path changes, nothing overlaps or exceeds the footer, and at 720×480 at least four folder rows and the whole launch bar stay visible

#### Scenario: Remote launch is disabled until connected

- Given server A is selected and still connecting
- When the user inspects the launch bar
- Then Terminal and any shown TUI buttons are disabled with a tooltip saying the server is not connected, and Local remains selectable

### Requirement: Open Workspace Git Indicator and Worktrees

When the launch target is inside a git working tree on the selected host, the screen MUST show an app-drawn git repository indicator beside the effective launch path, with tooltip and accessible name `Git repository`. When the launch target is the top level of any working tree of a repository that has at least one linked worktree, whether that is the main worktree's root or a linked worktree's root, and however it was reached (tree selection, Up, or a pasted address), the screen MUST show a Worktrees list. Its content and order MUST be the same in every such case: the main worktree first, then each linked worktree in the order the repository reports them. Worktrees rows MUST be 24px high with the 4px inset and the radius-6 `BG_HOVER` hover and inset `BG_SELECTED` selected fills of the folder rows. Each item MUST show its path and, when known, its branch name or short detached commit id using the existing head-label rules, plus its change pills. The preselected item MUST be the one whose path equals the launch target: the main worktree when the target is the repository root, and that linked worktree when the target is a linked worktree's root (never the main root in that case). Exactly one item is selected; selecting an item MUST make its path the effective launch path; changing the host, the launch target or the root MUST reset the selection to the default. A worktree whose directory is missing or unreadable MUST be shown with the text `missing`, MUST NOT be selectable, and MUST NOT be launched. When the launch target is a subdirectory of a working tree, the indicator and that working tree's change pills MUST be shown with no Worktrees list, and the effective launch path stays the launch target. When the repository has no linked worktree, the indicator MUST be shown with no list and the working tree's change pills beside it. When the target is not in a repository, git is missing or a read fails, no indicator, list or pills are shown and launching is unaffected.

Git reads for the screen MUST run off the UI thread, MUST be bounded, and MUST discard results for a previous host, connection, root or launch target. They MUST use only read-only git operations (top level, git and common directory, branch, commit id and subject, worktree list, status), each hardened as `Hardened read-only git reads` requires. Made MUST NOT create, remove, prune, lock, move, merge or check out worktrees, MUST NOT otherwise write to git, and MUST offer no worktree action other than selection. The list MUST scroll within its own area with the floating overlay scrollbar when it exceeds its bounded height (at least two visible items at 720×480), and MUST support Up/Down to move and Enter or Space to select.

#### Scenario: Repository root with worktrees preselects the root

- Given Local and launch target `/work/made`, a repository with linked worktrees `/work/made-wt/feature-a` (branch `feature-a`) and `/work/made-wt/fix` (detached)
- When the git reads complete
- Then the git indicator is shown, the Worktrees list is `/work/made` (its branch), `/work/made-wt/feature-a` (`feature-a`), `/work/made-wt/fix` (short commit id), the first item is selected, and the effective launch path is `/work/made`

#### Scenario: Navigating directly into a linked worktree preselects it

- Given Local, the repository of the previous scenario, and claude detected
- When the user pastes `/work/made-wt/feature-a` into the address bar and presses Enter, and separately selects that folder in the tree after re-rooting at `/work/made-wt`, and in each case activates Terminal, then Claude Code
- Then the git indicator and the Worktrees list are shown with `/work/made` first, then `/work/made-wt/feature-a`, then `/work/made-wt/fix`, `/work/made-wt/feature-a` is preselected (not the root), the effective launch path is `/work/made-wt/feature-a`, and both launches start there

#### Scenario: Selecting a worktree retargets every launch button

- Given the previous repository with the first item selected and OpenCode and claude detected
- When the user selects `/work/made-wt/feature-a` and activates Terminal, and separately selects it again and activates OpenCode
- Then the Terminal tab's shell and explorer start in `/work/made-wt/feature-a`, the OpenCode tab starts there too, and nothing in the repository or its worktrees is created, removed or modified

#### Scenario: Subdirectory, single worktree and non-repository targets

- Given a launch target `/work/made/crates` inside the repository, then a repository with no linked worktree, then a plain folder, then a host with git missing
- When the screen updates
- Then the subdirectory shows the indicator and its working tree's pills and no list and launches in `/work/made/crates`; the single-worktree repository shows the indicator and its pills and no list; the plain folder and the git-missing host show no indicator, list or pills; and every launch button stays usable

#### Scenario: Missing worktree and stale reads

- Given a repository whose linked worktree directory was deleted, and a slow status read for a previous launch target
- When the list renders and the user changes the launch target before the slow read completes
- Then the deleted worktree shows `missing` and cannot be selected or launched, and the late result is discarded instead of updating the new target's list or pills

### Requirement: Open Workspace Change Pills

For every working tree the screen shows (each Worktrees item, or the single working tree beside the git indicator), it MUST show one pill per change kind whose count is nonzero, in the order conflicted, staged, modified, untracked, labeled `N conflicted`, `N staged`, `N modified` and `N untracked`. Counts MUST cover the whole working tree, not only the selected subfolder, and MUST use the same classification as the `Grouped changes` requirement: a conflicted path counts only as conflicted; a partially staged path counts once as staged and once as modified; modified counts every unstaged change to a tracked path (the `Changes` group); untracked counts the `Untracked` group; ignored paths are not counted; a submodule counts only when its recorded commit changed, not for uncommitted changes inside it. A zero count MUST NOT show a pill, a clean working tree MUST show no pill and no `0`, and a working tree whose status is pending or failed MUST show no pill rather than a zero. Each pill MUST be a fully rounded `BG_ELEVATED` pill with a leading dot in the existing status color of its group and secondary text of at least 10px, so the kind is conveyed by text and not by color alone. The pills of one working tree MUST expose one tooltip and accessible name listing its nonzero counts, for example `Changes: 2 staged · 3 modified`.

#### Scenario: Pills split by type and omit zeros

- Given the repository root has 1 conflicted, 2 staged, 3 modified and no untracked paths, and a linked worktree has only 4 untracked paths
- When the Worktrees list renders
- Then the root item shows `1 conflicted`, `2 staged`, `3 modified` in that order and no untracked pill, the worktree item shows only `4 untracked`, and the root's tooltip is `Changes: 1 conflicted · 2 staged · 3 modified`

#### Scenario: Classification matches the git view

- Given a partially staged file, an ignored file, and a change outside the selected subfolder of a repository without linked worktrees
- When the pills beside the indicator render
- Then the partially staged file counts once in `staged` and once in `modified`, the ignored file is not counted, and the outside change is counted because counts cover the whole working tree

#### Scenario: Clean, pending or failed status shows no pill

- Given a clean worktree, a worktree whose status read is still pending, and one whose status read failed
- When the list renders
- Then none of the three shows a pill or a zero, and each remains selectable and launchable

### Requirement: Hardened read-only git reads

Every read-only git invocation Made runs, locally or through the remote helper, for the explorer git view, file-tree change dots, the git-view head label, worktree metadata and the Open workspace screen, MUST be hardened as follows. It MUST pass `-c core.fsmonitor=false`, `-c log.showSignature=false`, `-c protocol.allow=never` and `-c protocol.<name>.allow=never` for each of `file`, `git`, `ssh`, `http`, `https`, `ftp`, `ftps`, `ext` and `fd`. Its child environment MUST set `GIT_NO_LAZY_FETCH=1` and `GIT_OPTIONAL_LOCKS=0`, and MUST NOT inherit `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`, `GIT_COMMON_DIR`, `GIT_NAMESPACE`, any `GIT_CONFIG*` variable (so a user-set `GIT_CONFIG_GLOBAL` is not honored by these reads), `GIT_EXTERNAL_DIFF`, `GIT_DIFF_OPTS`, `GIT_EXEC_PATH` or `GIT_ATTR_SOURCE`. Status MUST be read with `--ignore-submodules=dirty`.

Filter drivers: before a status read, Made MUST read the scope of every `filter.<driver>.*` configuration key. A driver whose keys come only from the global or system scope MUST run normally (for example git-lfs). A driver with any key from a repository-controlled scope (local, worktree, or any other non-global, non-system scope) MUST be neutralized for that read: its `clean` and `process` set to empty and `required` set to false. This includes repository-local LFS setups (`git lfs install --local`, the older `git lfs init`, or any repository-local `filter.lfs.*` key), so files those drivers track whose stat information is dirty MAY appear as modified; this fails safe rather than running repository-defined code. When the installed git does not support `--show-scope` (git older than 2.26, detected by git rejecting that option), Made MUST instead neutralize every configured filter driver for that read, whatever its scope (`clean` and `process` empty, `required` false), and MUST still run status; on such git, files that a trusted global filter (for example git-lfs) would normally clean MAY show as modified. Any other failure to read driver scopes or names, malformed scope output, or a driver name that cannot be expressed as a `-c` override MUST fail closed: the status read MUST NOT run, and the read MUST be reported as failed, so the git view shows "Git status unavailable" and the Open workspace screen shows no pills for that working tree. These guarantees add to, and do not relax, `Read-only source control`, SSH-04 and SSH-08: the driver-scope read is a read-only configuration query run by the git runner only inside its status operation; it is not a wire operation and the remote `GitRead` set is unchanged, so SSH-04 and SSH-08 need no change, so the reads stay within the closed read-only operation set, and no git write, fetch or repository-defined command is performed.

#### Scenario: Repository-controlled hooks never execute on selection

- Given a repository whose local config sets `core.fsmonitor` to a script, defines a local-scope filter driver whose `clean` and `process` commands write a marker file, is a partial clone with a promisor remote and missing blobs, and Made was started with `GIT_DIR`, `GIT_WORK_TREE` and `GIT_CONFIG_GLOBAL` set in its environment
- When the user selects that repository on the Open workspace screen, then opens it as a tab and shows the git view, locally and on an SSH host
- Then the fsmonitor script and the filter commands never run (no marker is written), no fetch or network transport is attempted, none of the inherited `GIT_*` variables reaches the git child, and the counts and git view either reflect the working tree or report the read as failed

#### Scenario: Global LFS filter keeps correct counts

- Given git-lfs configured only in the global scope and a repository with LFS-tracked files, one of them modified, and no repository-scope filter keys
- When the Open workspace pills and the git view are read
- Then the LFS filter runs normally, the modified LFS file counts once as modified, and unchanged LFS files are not counted

#### Scenario: Old git without scope support neutralizes all drivers

- Given git older than 2.26 that rejects `--show-scope`, a global-only git-lfs filter, a local-scope filter whose `clean` command writes a marker file, and an unchanged LFS-tracked file
- When the Open workspace pills and the git view are read
- Then every filter driver is neutralized for the read, status still runs, no marker is written, counts and the git view are shown rather than failed, and the unchanged LFS-tracked file MAY appear as modified

#### Scenario: Unreadable driver scopes fail closed

- Given a git that supports `--show-scope`, and in turn: the driver-scope read fails for another reason, its output is malformed, or a driver name cannot be expressed as a `-c` override
- When a status read is due
- Then status is not run, the git view shows "Git status unavailable", the Open workspace screen shows no pills for that working tree, and the file tree and every launch button stay usable

### Requirement: Program-neutral wording

User-visible text in flows that apply to any program MUST NOT name a specific integrated TUI. These flows include SSH connection, connection test, staging, browse and launch-validation notices; generic launch errors; restart and update confirmations; and remote session transport states. Text that names a program MUST use that program's catalog label (`OpenCode`, `Grok Build`, `Claude Code`, never a shorter or different name such as `Grok`) and MUST appear only where the context is that program, such as its own launch button, tab, diagnostics row or reason. When a remote launch is attempted for a program not detected on the selected server, the error MUST be `{catalog label} was not detected on that server. Nothing was started locally.` Remote SSH transport reasons are defined by SSH-09.

#### Scenario: Generic flows do not name a program

- Given a failed SSH connection test, a staging failure, a browse failure, a restart confirmation and an update confirmation, with OpenCode, Grok Build and Claude Code all detected
- When each text is shown
- Then none of them names OpenCode, Grok Build, Claude Code or `Grok`

#### Scenario: Not-detected remote launch names the catalog label

- Given server A where grok was detected and then vanished, and a request to launch Grok Build on A
- When the launch is attempted
- Then the error reads `Grok Build was not detected on that server. Nothing was started locally.`, nothing starts locally, and no other server is used

### Requirement: Remote Open workspace browsing and Git parity (SSH-12)

When the user selects an SSH host on the Open workspace screen, Made MUST connect to that server under the SSH-01 authentication and host-trust rules: an unknown host MUST show the OpenSSH fingerprint and require explicit approval, a changed trusted key MUST be refused, interactive prompts MUST work, and cancelling MUST stop only that attempt while the screen, Local and every tab stay usable. Browsing MUST use the server's companion helper under the existing SSH-06 per-server/version staging consent; when consent is needed it MUST be requested with the existing consent flow, and refusal MUST write nothing and show an actionable non-blocking reason in the screen's notice line while the host stays listed and selectable. Every connection, consent, staging, browse-open and browse-operation failure MUST be shown in that notice line as `Open Workspace Screen` defines.

Remote browsing MUST be an explicit grant scoped to one screen instance and one server user, separate from every tab's root grant. It MUST permit only: listing a directory's entries with their kinds (the screen shows folders) under the 1000-entry cap, checking that an absolute path is an existing readable directory for the address bar and for launch, and the existing read-only Git operations (top level, branch, git and common directory, worktree list, commit id and subject, status) for the launch target and for the worktree paths the repository reports, each hardened as `Hardened read-only git reads` requires, with at most one git read of the grant in flight at a time. It MUST NOT read or preview file contents, write anything outside app-owned remote storage, start a terminal session or a TUI, or apply results to any tab. The grant MUST end when the screen closes, another host is selected, or the server record is removed; ending it MUST NOT affect any tab on that server. Every path MUST be validated on the remote authority and MUST NOT be canonicalized, opened or resolved against the host filesystem; server-supplied names and paths MUST be displayed sanitized and never executed or used as options. Requests and replies MUST be bounded; malformed, oversized or stale replies (previous host, connection epoch, root or launch target) MUST be discarded without blocking the UI.

Remote Open workspace work MUST be asynchronous and bounded under SSH-09: a slow, stalled or failed server MUST NOT block selecting Local or another host, keyboard navigation, Escape/Close, or input, tab switching and Explorer controls in existing workspaces. The git indicator, Worktrees list, preselection, change pills, missing-worktree handling and read-only rules of the UI spec MUST apply to SSH hosts using metadata read on the remote authority, with no remote write and no worktree management; missing remote git or a non-repository MUST degrade as locally. A remote launch from the screen MUST bind the new tab to that server and the effective launch path, which MUST be revalidated on the server at launch; a directory that vanished or became unreadable MUST cause a contextual failure with no local execution and no other server substituted.

#### Scenario: Selecting an unknown host requires approval

- Given a saved server never trusted before, then a server whose trusted key changed
- When the user selects each host on the screen
- Then the first shows the OpenSSH fingerprint and connects only after explicit approval, the second is refused with actionable feedback and no browsing, and in both cases Local stays selectable and no tab is affected

#### Scenario: Remote browsing is a bounded listing grant

- Given a connected server whose filesystem has nested folders, a 2000-entry directory, a symlinked directory, names with shell metacharacters and option-like prefixes, and a hostile reply variant
- When the user browses, pastes absolute and relative paths, and the hostile reply arrives
- Then folders list with the 1000-entry cap and truncation notice, paths are validated on the server and never on the host, names display sanitized and nothing is executed, no file content is read or previewed, the hostile reply is rejected within bounded work, and no tab state changes

#### Scenario: Remote git parity

- Given a remote repository root with two linked worktrees and staged, modified, untracked, conflicted and ignored paths
- When the user selects that folder on the SSH host
- Then the git indicator, the Worktrees list with the root first and preselected, and the change pills appear with the same rules as locally, and no git write or worktree change happens on the server

#### Scenario: Stalled server does not block the screen

- Given a server that stops answering mid-listing and a busy local workspace
- When the user selects Local, then presses Escape
- Then Local's tree appears immediately, Escape returns to the workspace, the existing tabs keep accepting input, and the stalled grant's late replies are discarded

#### Scenario: Vanished directory at launch

- Given server A selected with an effective launch path that is deleted on the server after it was listed
- When the user activates Terminal
- Then Made reports a contextual failure for that path, starts nothing locally, does not use another server, and the screen stays open for another choice

#### Scenario: Consent refusal on the screen

- Given a server without staged companion artifacts
- When the user selects it and refuses the staging consent
- Then no app-owned remote file is written, the notice line shows an actionable reason in `ERROR` color with an error glyph, the host stays listed, and Local remains usable

## MODIFIED Requirements

### Requirement: Minimal Workspace Layout

Made MUST present a terminal-first workspace in one native window: workspace tabs above the content area, a selected left surface that is either the terminal or the full-left read-only preview, and a full-height explorer in the right column. Persistent header icon actions MUST be icon-only buttons with tooltips and accessible labels: Open workspace (`+`), Explorer toggle, and the optional Sessions action when shown; Made MUST NOT show a `Servidores SSH` header action, because SSH host management lives in the Open workspace screen under SSH-01. The Open workspace action MUST live in the workspace tab strip immediately after the last visible tab and before the overflow trigger, use an app-drawn `+` glyph, reserve its width in the tab-fitting budget, and remain reachable with the overflow trigger when tabs overflow. The app-drawn `+` MUST retain its tab-strip placement, MUST use the tooltip and accessible name `Open workspace`, and activating it MUST open the Open workspace screen directly with no menu, whether or not any integrated TUI is detected or any SSH server is saved; local and remote choices are offered by that screen (`Open Workspace Screen`). Existing confirmation semantics remain unchanged for local launches. The remaining right icon group MUST order Sessions (when shown), Explorer left-to-right, keeping Explorer rightmost. The top header MUST NOT show text labels for those icon actions; the Made identity and workspace-tab labels remain text, and workspace tabs MUST retain a close affordance. The header controls MUST float directly over the window clear/native backdrop and MUST share one vertical center and square hit targets, with vertically centered glyphs and the last icon’s glyph centered in its target. Workspace tabs MUST start at the window gutter/card left edge. The header control row MUST span the window gutter to `width - gutter` with no extra horizontal padding, so the last right-side icon’s 26/24 target, selected/hover fill, and focus ring end at the explorer/terminal card right edge with no gutter overhang. The 14px glyph MUST remain centered inside its target with normal optical padding on both sides, so its visible edge is inset from the card border. The wordmark MUST sit to the right of the tab strip immediately before the right icon group. The tab-fitting budget MUST reserve a 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark; with few tabs the visible flexible space between the last visible control and the wordmark is larger because the boundary is reserved, not measured from the last control. The wordmark-to-first-right-icon gap MUST be exactly 24px wide / 18px compact. The whole right cluster MUST be reserved so tabs, `+`, and overflow never overlap it, and the wordmark MUST NOT paint a tab fill, hover fill, or selection treatment. The MADE wordmark MUST be the persistent About trigger. While a workspace is active, activating the wordmark by pointer or keyboard MUST open and close the About card; when no workspace is active, activation MUST NOT open or toggle the About card. The wordmark MUST keep its two-tone MADE brand identity, MUST show a pointer cursor over its target, MUST expose the tooltip and accessibility name `About`, or `About, update ready` while the update indicator is shown, MUST provide a target of at least 24×24, MUST show a visible keyboard focus ring, and MUST activate on Enter or Space. While an installed build has a verified update ready to install (REL-07), the wordmark MUST show one discreet app-drawn accent dot as its update indicator; the indicator MUST NOT add a text label, banner, pill, tab fill, hover fill or selection treatment, MUST NOT change the wordmark target, the documented gaps or the reserved right-cluster width, and MUST be absent in every other state. The indicator appears only on the header wordmark while a workspace is active; the picker wordmark (the brand-only MADE wordmark on the folder picker shown when no workspace is active) never shows it, and a ready update is indicated again once a workspace becomes active. The top workspace strip and workspace tabs MUST both be 26/24 high while session-map metrics remain unchanged. Header icon actions, optional session tabs, explorer rows and picker rows share one interaction system: hover uses a `BG_HOVER` filled rounded rect with radius 6 and no border; selection uses a `BG_SELECTED` filled rounded rect with no border or accent bar; inactive icons use reduced opacity that returns to full on hover or selection. The wordmark trigger is the deliberate exception: it paints no hover or selection fill and conveys focus with its focus ring. Workspace tabs use the dedicated selected/hover/idle-fill treatment and the OpenCode icon and session status dots defined by the Brand and Workspace Tabs requirement. Expanded explorer folders MUST be marked by a small app-drawn disclosure triangle and a brighter label only, with no border, accent side bar or extra expansion fill. Explorer rows MUST use 24px height, a 4px horizontal inset, 6px radius, a 16px icon slot inset 4px within the painted row fill (slot starts 8px from the row's left edge) and text starting 28px from the row's left edge, with depth adding 14px; files keep a quiet app-drawn bullet. Text and disclosure/bullet markers MUST be vertically centered on one optical center in the row. Selection and focus in the tree MUST be conveyed by the inset `BG_SELECTED` fill and `TEXT_PRIMARY` label only, with no focus outline around the row or the tree container. When the tree container receives keyboard focus with no row selected, the first visible row MUST become selected with that same fill and primary text, without opening a preview or changing pointer row activation. Every scroll area (explorer tree, picker list, preview, About) and the terminal scrollback bar MUST use hover-only floating overlay bars: no track or thumb at rest, a thumb-only fade over about 120–160 ms when the area is hovered, expansion on bar hover/drag with a faint track only then, fade-out on leave, zero allocated width and no layout shift, and no always-on track. Routine integration, shell-running, watcher, version, branch, revision and performance statuses MUST NOT consume persistent banners, pills, path/status rows or empty session strips; technical details belong in the About card's on-demand diagnostics detail. Update checking, downloading and check failures are routine statuses under this rule; the only persistent update cue is the wordmark update indicator, and update detail belongs in the About Updates section. Real shell/input, file/root and explicitly requested action errors MUST remain contextual and non-blocking. The explorer MUST occupy the full right content column with no preview below it, MUST be hideable/restorable through the header Explorer toggle, and MUST have a resizable width constrained to leave a usable left pane. Explorer width MUST be adjusted only by pointer-dragging its divider; the explorer header MUST NOT contain Wider, Narrower, Reload or Hide text actions and MUST NOT offer a routine manual refresh control. The left surface and the explorer card MUST be separated by a 4px pane gap in both layouts. At rest the divider MUST NOT paint a persistent line or grip; hover or drag MAY fade in only a small filled, fully rounded 2×32 capsule centered in that gap, using muted `BORDER_CONTROL` (and `ACCENT` at restrained opacity while dragging), with no full-height line and no boxed grip, while retaining a comfortable pointer hit area, with a fade-out on leave. Hiding it MUST give the left surface the reclaimed width without killing the shell, resetting the root or losing tree/preview state; restoring MUST reconcile changes that occurred while hidden. Width and visibility need only be retained for that live workspace, not across app restart. Keyboard reachability, accessible names, visible focus, target sizing and the pointer-only divider exception are owned by the Keyboard Focus and Accessibility requirement; the wordmark trigger behavior is owned here; essential controls MUST NOT clip or introduce noisy default technical chrome.

#### Scenario: Quiet shell workspace

- Given a newly opened folder and no running OpenCode
- When the default workspace is displayed
- Then a usable terminal and full-height right explorer appear without an empty preview, empty session strip, degraded/waiting banner, technical status bar or routine status pills; the header controls float over the clear/native backdrop without a persistent band and show only icon-only Open workspace (`+`) and Explorer actions with accessible labels and no `Servidores SSH` action, no Diagnostics icon, and the MADE wordmark as the About trigger; and the explorer header shows no text action buttons

#### Scenario: Icon-only header controls

- Given a live workspace at 1280×800 or 720×480
- When the user inspects or keyboard-navigates the top header
- Then Open workspace (`+`), Explorer toggle and the visible optional Sessions action are icon-only with matching tooltips and accessible labels; the MADE brand text remains as the About trigger with the matching tooltip and accessibility name `About`; no persistent action text labels are rendered; workspace tabs retain their close affordance; and every one of these actions is reachable and visibly focused by keyboard

#### Scenario: Header placement and order

- Given a live workspace at 1280×800 or 720×480, with tabs that fit and a case that overflows
- When the header lays out
- Then workspace tabs start at the window gutter/card left edge, the last right-side icon’s 26/24 target, selected/hover fill, and focus ring end at the card right edge (`width - gutter`) with no gutter overhang and the 14px glyph centered inside the target with normal optical padding, the header row has no extra horizontal padding, the top strip and tabs are 26/24 high with the card top at 38/34 and the strip vertically centered in that header band with 6/5 above and below it, the tab-fitting budget reserves the documented 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark (the visible space after the last visible control may be larger), the wordmark-to-first-icon gap is exactly 24px wide / 18px compact, the whole right cluster is reserved so tabs/plus/overflow never overlap it, the app-drawn `+` Open workspace action follows the last visible tab and precedes the overflow trigger, the overflow trigger and `+` both remain reachable, the right icon group orders Sessions (when available), Explorer with Explorer rightmost and no `Servidores SSH` action and no Diagnostics icon while retaining tooltips, accessible names, minimum targets, and keyboard focus, and the MADE wordmark stays immediately before that group with a target of at least 24×24 and the `About` tooltip and accessibility name

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
- Then its full-height width changes within usable bounds, the divider is invisible at rest and fades in a small capsule on hover or drag, the active left surface reflows without terminal corruption, no Wider/Narrower/Reload/Hide text action is present, essential controls have visible focus and do not clip, and the pointer-only divider is not a keyboard stop

#### Scenario: Explorer header action removal

- Given a visible explorer header
- When the user inspects or navigates it by pointer or keyboard
- Then no Wider, Narrower, Reload or Hide text action is present, and no routine manual refresh control is offered for normal live updates

#### Scenario: Hover-only explorer divider

- Given a visible explorer at rest
- When the divider is not hovered or dragged
- Then no vertical divider line or grip is painted; when hovered or dragged, a small capsule fades in within the 4px pane gap while the pointer hit area remains usable, and a mere hover does not change the stored per-workspace width

#### Scenario: Explorer splitter fade

- Given a visible explorer splitter with its 6px pointer hit area centered on the 4px pane gap
- When the pointer hovers, starts dragging, releases, or leaves
- Then the splitter paints nothing at rest, fades in over about 140 ms to a discreet filled, fully rounded 2×32 capsule inside the pane gap, touching neither card, using muted `BORDER_CONTROL` at partial opacity on hover, uses the green `ACCENT` at a restrained opacity while dragging, fades out on leave, keeps the resize cursor and “Drag to resize explorer” tooltip, remains pointer-only (not a keyboard stop), and never paints a bright permanent line

#### Scenario: No manual refresh control

- Given a visible explorer while files change on disk
- When the user looks for refresh actions
- Then no routine manual Reload control exists and creates, deletes and renames still appear automatically through the watcher

#### Scenario: Quiet integration chrome

- Given OpenCode is absent, inactive, disconnected, degraded, or has exited normally
- When the workspace chrome is inspected
- Then no routine degraded/waiting/unsupported banner or status pill is shown, and integration detail remains available only through the About card's on-demand diagnostics detail

#### Scenario: Open workspace replaces the plus menu and SSH header action

- Given a shell-only workspace and then a workspace with Sessions available at both responsive sizes, with tab overflow, no local OpenCode installed and saved SSH servers
- When the header is inspected and `+` is activated by pointer and keyboard
- Then no `Servidores SSH` header action is present, Explorer stays rightmost, `+` keeps its tab-strip placement with the tooltip and accessible name `Open workspace` and opens the Open workspace screen directly with no menu, local and SSH choices are reachable from that screen, the right cluster is reserved in the fitting budget, and no routine technical banner or empty session strip is added

#### Scenario: Update-ready indicator

- Given an installed build at 1280×800 and 720×480, first with no update ready, then while an update is checking or downloading, then with a verified update ready, with tabs that fit and a case that overflows
- When the header lays out and the user hovers and keyboard-focuses the MADE wordmark
- Then no indicator is shown until the update is ready; once ready, one discreet accent dot appears on the wordmark with no text label, banner, pill or fill, the wordmark target, gaps, reserved right-cluster width and tab fitting are unchanged, and the tooltip and accessibility name read `About, update ready`; with no active workspace the picker wordmark shows no indicator, and reopening a workspace shows it again; a non-installed build never shows the indicator

### Requirement: Degraded Integration

OpenCode integration is optional. If it is unavailable, disabled, incompatible, disconnected, or OpenCode is missing or older than v2, Made MUST keep the terminal and explorer usable with shell-directory fallback as defined in Active Session Follow. Normal shell use and normal OpenCode exit MUST NOT be treated as an app-level degraded state. Attempted integration failures MUST remain inspectable with actionable reasons in Diagnostics; actual shell, filesystem, or user-action failures MUST retain contextual non-blocking feedback. Made MUST NOT crash or block terminal input. Made SHOULD reconnect automatically when integration becomes available, but MUST NOT launch OpenCode automatically. Made MUST launch OpenCode only when the user explicitly activates the OpenCode launch button of the Open workspace screen, which starts the detected executable directly as a new tab whose launch folder and explorer root are the effective launch path chosen on that screen. Routine chrome suppression is owned by the UI spec.

#### Scenario: Plugin unavailable

- Given the OpenCode integration is not present
- When the user uses OpenCode in the Made terminal
- Then the terminal remains fully usable, the explorer follows shell-directory fallback, and Diagnostics records integration unavailability

#### Scenario: OpenCode missing or older than v2

- Given OpenCode is not installed or the installed version is older than v2
- When the user tries to use the integration
- Then the terminal remains usable and Diagnostics provides an unavailable or unsupported integration state with an actionable reason

#### Scenario: OpenCode started with extension loading disabled

- Given OpenCode is started with extension loading disabled
- When Made cannot connect
- Then Diagnostics names disabled integration, shell follow continues, and Made does not crash

#### Scenario: Bridge disconnects mid-session

- Given integration was connected
- When the connection drops while OpenCode is still running
- Then Made resumes the latest known shell cwd (launch folder only if unknown), keeps the terminal usable, and records the disconnected reason in Diagnostics

#### Scenario: Incompatible protocol version

- Given the integration reports a protocol version Made does not support
- When Made receives it
- Then Made rejects the state, records unsupported integration in Diagnostics, and keeps the terminal and shell-following explorer usable

#### Scenario: OpenCode exits normally

- Given OpenCode exits normally
- When the integration disconnects
- Then Made resumes shell-directory fallback without treating the workspace as degraded and leaves the same shell usable for a new user-initiated launch

#### Scenario: Reconnect

- Given integration returns after a disconnect
- When a valid connection with session tabs enabled is established
- Then Diagnostics reports connected and Made resumes following the active session or, when no session is active and session tabs are enabled, the connected view directory in that workspace without applying stale connection state

#### Scenario: Session tabs unavailable

- Given OpenCode reports that session tabs are disabled
- When Made receives the state
- Then Diagnostics identifies the v1 tabs requirement, Made keeps the terminal and shell-following explorer usable, and Made does not follow the connected view directory

#### Scenario: Configuration composition fails

- Given Made cannot compose the per-launch OpenCode configuration without altering user files
- When OpenCode starts
- Then OpenCode can run without Made integration, Diagnostics reports the composition failure, shell use remains normal, and user files stay unchanged

### Requirement: Catalog-driven new-tab menu

The `+` control MUST keep its tab-strip placement with the tooltip and accessible name `Open workspace` and MUST open the Open workspace screen directly with no menu (`Open Workspace Screen` in the UI spec). The screen's launch bar MUST be driven by the integrated-TUI catalog rather than by a special case for one program. Detecting a catalog entry on the local host MUST be a metadata-only PATH lookup that never spawns the program, never checks a version, and never installs a plugin, and that lookup MUST NOT delay shell or explorer startup. Empty and relative PATH segments MUST be ignored. The launch bar MUST show Terminal first, using the existing terminal glyph and opening a plain workspace tab in the effective launch path, then one button for each integrated TUI detected on the selected host in catalog order; with no TUI detected it MUST show Terminal alone. Catalog order MUST be OpenCode, then Grok, then Claude Code, and a program that is not detected on the selected host MUST be omitted. Choosing a local TUI button MUST start that executable's absolute path directly as the child of a new tab whose launch folder and explorer root are the effective launch path chosen on the screen, with no shell first and no further selection surface, and the current tab MUST keep running. Made MUST NOT launch an integrated TUI unless the user chooses its button. The OpenCode button MUST keep the vendored 16px opencode.svg and MUST keep its existing launch, including its per-launch plugin configuration injection, follow, and dots. The Grok button label MUST be Grok Build, and its 16px leading slot MUST paint the official mark in `grok.svg` (the two paths from https://grok.com/images/favicon.svg, fill #FCFCFC, viewBox "56 56 400 400") rather than the terminal glyph. Choosing Grok MUST launch grok's absolute path with the single argument `dashboard`. That launch MUST NOT inject plugin configuration, including OpenCode's plugin environment and Made's Claude Code plugin entry, and MUST NOT write the user's Grok config. The Claude Code button's label, mark and launch are owned by `specs/claude-code-session-follow.md`. Codex and any other agent TUI MUST stay out of the catalog. Typing grok in a shell MUST remain valid, MUST NOT be replaced by the launch bar, and MUST NOT be given the `dashboard` argument.

#### Scenario: No detected TUI shows Terminal alone

- Given no integrated TUI is on PATH and Local is selected
- When the user activates `+`
- Then the Open workspace screen opens with no menu and its launch bar shows Terminal alone

#### Scenario: Catalog launch bar lists detected TUIs in order

- Given OpenCode, Grok and Claude Code are all on PATH and Local is selected
- When the user activates `+`
- Then the launch bar is Terminal, then OpenCode, then Grok Build, then Claude Code, and the `+` tooltip and accessible name are Open workspace

#### Scenario: Only Grok is installed

- Given Grok is on PATH and neither OpenCode nor Claude Code is, and Local is selected
- When the user activates `+`
- Then the launch bar is Terminal, then Grok Build, and OpenCode and Claude Code are omitted

#### Scenario: Grok button uses the official mark

- Given the launch bar includes Grok
- When the Grok button is shown
- Then its label is Grok Build and its 16px leading slot paints the official mark from `grok.svg`

#### Scenario: OpenCode button keeps its icon and launch

- Given OpenCode is on PATH and Local is selected with effective launch path `/work/api`
- When the user chooses the OpenCode button
- Then the button shows the vendored 16px opencode.svg, Made starts OpenCode's absolute path directly in a new tab whose launch folder and explorer root are `/work/api` with no shell first, and OpenCode's existing launch injection, follow, and dots stay in place

#### Scenario: Choosing Grok starts grok in the chosen folder

- Given a tab is running, Grok is on PATH, and Local is selected with effective launch path `/work/api`
- When the user chooses Grok Build
- Then Made starts grok's absolute path plus the argument `dashboard` directly as the child of a new tab whose launch folder and explorer root are `/work/api`, with no shell first and no further selection surface, the current tab keeps running, that launch does not inject plugin configuration, OpenCode's plugin environment or Made's Claude Code plugin entry, and does not write the user's Grok config, and typing grok in a shell is not given that argument

#### Scenario: Terminal button opens a shell workspace

- Given at least one integrated TUI is detected
- When the user chooses Terminal
- Then a plain shell workspace opens in the effective launch path and no integrated TUI is launched

#### Scenario: Detection does not launch or block startup

- Given an integrated TUI executable is on PATH
- When Made starts the shell and explorer
- Then the lookup does not spawn that program, does not check its version, ignores empty and relative PATH segments, and startup does not wait on the lookup

#### Scenario: PATH lookup does not install

- Given grok is on PATH
- When Made runs the metadata-only PATH lookup
- Then that lookup does not spawn grok and does not install the plugin

#### Scenario: Launch bar does not replace a shell launch

- Given Grok is on PATH
- When the user types grok in a shell tab and does not choose the Grok button
- Then Made does not launch Grok on its own, and that shell session remains a valid way to start Grok

#### Scenario: Other agents stay out

- Given Codex or another agent TUI is on PATH
- When the user opens the Open workspace screen
- Then no launch button is shown for it

### Requirement: Claude Code launch

Claude Code MUST be an integrated-TUI catalog entry after Grok. Detecting it MUST be the catalog's metadata-only PATH lookup for the executable `claude`, which never spawns it, never checks its version, never loads or installs a plugin, ignores empty and relative PATH segments, and does not delay shell or explorer startup. When `claude` is detected on the selected host, the Open workspace launch bar MUST show a button labeled `Claude Code` after every other detected catalog button, whose 16px leading slot paints the Claude Code mark defined by `Claude Code tab presentation`. Choosing that button for Local MUST start the detected `claude` absolute path with no arguments directly as the child of a new tab whose launch folder and explorer root are the effective launch path chosen on the screen, with no shell first and no further selection surface, and the current tab MUST keep running. That launch MUST carry the session-only plugin loading of `Session-only Claude Code plugin`, MUST NOT carry OpenCode's plugin environment, and MUST NOT add any command-line argument. Made MUST NOT launch Claude Code unless the user chooses its button; the read-only `claude agents --json` listing of `Claude Code explorer follow and precedence` starts no Claude Code session and is the only other way Made runs `claude`. Typing `claude` in a local shell tab MUST remain valid, MUST NOT be replaced by the launch bar, and MUST receive the same session-only plugin loading.

#### Scenario: Catalog lists Claude Code last

- Given OpenCode, grok and claude are all on PATH and Local is selected
- When the user activates `+`
- Then the launch bar is Terminal, OpenCode, Grok Build, Claude Code, and the `+` tooltip and accessible name are Open workspace

#### Scenario: Only Claude Code is installed

- Given claude is on PATH and neither OpenCode nor grok is, and Local is selected
- When the user activates `+`
- Then the launch bar is Terminal, then Claude Code, and the Claude Code button paints the Claude Code mark in its 16px leading slot

#### Scenario: Choosing Claude Code starts claude in the chosen folder

- Given a tab is running, claude is on PATH, and Local is selected with effective launch path `/work/api`
- When the user chooses the Claude Code button
- Then Made starts claude's absolute path with no arguments directly as the child of a new tab whose launch folder and explorer root are `/work/api`, with no shell first and no further selection surface, the current tab keeps running, and the launched Claude Code loads Made's session-only plugin and no OpenCode plugin environment

#### Scenario: Detection does not run claude

- Given claude is on PATH
- When Made starts the shell and explorer and runs PATH detection
- Then the lookup does not spawn claude, does not check its version, does not load or install a plugin, ignores empty and relative PATH segments, and startup does not wait on it

#### Scenario: Shell-typed claude stays valid

- Given a local shell tab and claude on PATH
- When the user types `claude` in that shell and does not choose the Claude Code button
- Then Made does not launch Claude Code on its own, the typed claude starts normally with Made's session-only plugin loaded, and the shell remains a valid way to start Claude Code

### Requirement: SSH server entry and launch choices (SSH-01)

SSH server management MUST live in the Open workspace screen's host bar (`Open Workspace Host Bar` in the UI spec); Made MUST NOT show a separate `Servidores SSH` header action. The host bar's `+ Add host` button MUST open an inline host form within the screen with fields for display name, SSH host/alias, optional user, optional port, optional key-file reference and optional initial remote directory, validated before saving with an error beside each invalid field, and with Save and Cancel; saving MUST add the record as the last SSH host button and select it, and Cancel MUST discard the draft. Each SSH host button MUST offer Edit, Test connection and Remove through both a context menu (secondary click, or the keyboard context-menu key or Shift+F10 while the host button is focused) and a hover- and focus-revealed `⋯` button with the tooltip and accessible name `Host actions` and at least a 24×24 target that remains reachable by keyboard. Edit MUST open the same form prefilled with the record; Remove MUST ask for confirmation naming the host and stating that open tabs stay open; Test connection MUST run the explicit connection test and remote OpenCode, grok and claude detection, and its result MUST be reported on that host: a success MUST show no text notice and is conveyed only by the host button's connected state badge, its detected-program marks, and its tooltip and accessible name; a failure MUST show a contextual, actionable error notice for that host in the screen's notice line under the rules of `Open Workspace Screen` (`ERROR` color with an error glyph, dismissible through a keyboard-reachable close control and by Escape while focused, cleared on a later successful test of that host or when another host is selected). Each SSH host button MUST show that server's connection state and the catalog programs detected there. Local MUST NOT offer Edit, Test connection or Remove. While an SSH host is selected, the launch bar MUST offer `Terminal` for that server and, when that program has been detected on the server, OpenCode, Grok Build and Claude Code buttons. In this and every other SSH requirement, `Terminal SSH`, `OpenCode SSH`, `Grok SSH` and `Claude Code SSH` name those launch buttons while an SSH host is selected; a server's remote launch choices, menu or row for a program mean that host's launch bar and its button; the selected remote directory means the effective launch path chosen on the screen under SSH-12; and no separate folder picker means no further selection surface after the launch button is activated. `OpenCode SSH` MUST be offered only when OpenCode has been detected on that server, `Grok SSH` only when grok has been detected there, and `Claude Code SSH` only when claude has been detected there. Detection MUST NOT start the OpenCode TUI, start grok, start claude, or install any program's plugin, and it MUST use the same bounded metadata-only remote PATH rules as the existing probe, ignoring empty and relative PATH entries. Failure or absence MUST NOT prevent a supported remote terminal/explorer workspace or unrelated local workspaces from operating. A previously detected executable that disappears before launch MUST cause contextual failure, not local execution or substitution of a different server. Choosing `Grok SSH` MUST start the detected remote grok executable with the single argument `dashboard` directly as a new tab bound to that server and its selected remote directory, with no shell first and no separate folder picker, while the current tab keeps running and no plugin configuration is injected into that launch; typing `grok` manually in a supported remote shell MUST remain valid, MUST NOT be replaced by the launch bar, and MUST NOT be given the `dashboard` argument. Choosing `Claude Code SSH` MUST start the detected remote claude executable as SSH-11 defines.

Server records MUST persist across client restart in private app-owned storage and contain only id/display name, SSH host/alias, optional user/port/key-file reference and optional initial remote directory, never private-key bytes, credentials, tokens or prompt answers. Add/edit/remove MUST be supported; record changes MUST NOT retarget/terminate existing tabs. Corrupt records MUST produce a recoverable error without overwriting the original or blocking local startup; schema-version mismatch MUST NOT be silently rewritten. Live remote tabs/secrets MUST NOT be restored across application restart.

Authentication MUST use installed host OpenSSH, existing aliases/config/ProxyJump, available keys/agent and interactive encrypted-key/password/keyboard-interactive/MFA prompts. No credentials MUST be persisted or logged; cancellation MUST stop the owning attempt only. Unknown host identity MUST display the OpenSSH fingerprint and require explicit approval before connection; additions MUST be app-owned, without rewriting personal SSH config/known_hosts. A changed trusted key MUST be refused, not accepted through the first-use approval flow. Existing user-supplied SSH config/proxy commands remain user-trusted local configuration, not data supplied by the server. Agent forwarding and configured network forwards MUST be disabled for Made connections. Missing OpenSSH or unavailable prompt support MUST fail actionably rather than falling back to insecure authentication.

#### Scenario: SSH host management is reachable from Open workspace

- Given a window at 1280×800 and 720×480 with overflowing tabs, with Sessions both visible and hidden, and two saved servers
- When the user uses pointer and keyboard to open the Open workspace screen, add a host, and reach Edit, Test connection and Remove of a saved host through the context menu and through the `Host actions` button
- Then no `Servidores SSH` header action exists, `+` does not clip or overlap tabs, the add form validates each field before saving and adds and selects the new host last, every host action is reachable by keyboard with focus and accessible names, Remove confirms before deleting and leaves that server's open tabs running, and local creation remains available

#### Scenario: Connection test and per-server detection

- Given server A has supported OpenCode installed and server B does not, and both are otherwise supported reachable servers
- When the user adds and tests them from their host buttons and selects each host
- Then each test reports its own result on its own host button (a success only through its connected badge, marks, tooltip and accessible name, with no text notice), `Terminal SSH` is available for either server and `OpenCode SSH` only for A, no OpenCode TUI is started by the test, and neither local detection nor A's result is reused for B

#### Scenario: Test feedback is quiet on success and dismissible on failure

- Given server A that tests successfully with OpenCode detected, and server B whose test fails authentication
- When the user tests A, then tests B, dismisses B's notice with its close control by keyboard, tests B again with the failure fixed, and in a variant selects A while B's notice is shown
- Then A shows no text notice and only its connected badge, OpenCode mark, tooltip and accessible name change; B shows a contextual error notice with an actionable reason that closes from its keyboard-reachable close control and by Escape while focused, clears on B's later successful test, and clears when A is selected

#### Scenario: Failed probe or vanished executable

- Given a test fails due to authentication, network or host-trust failure, or a detected OpenCode executable disappears before launch
- When the user inspects the result or attempts that launch
- Then Made reports an actionable scoped error, does not silently install, bypass host verification, run locally or affect another tab, and does not falsely claim a successful remote OpenCode launch

#### Scenario: Grok SSH button and direct dashboard launch

- Given server A has grok on its remote PATH and server B does not, and both are otherwise supported reachable servers
- When the user selects each host on the Open workspace screen, chooses `Grok SSH` on A with a selected remote folder, and separately types `grok` manually in A's remote shell
- Then only A offers `Grok SSH`, choosing it starts A's detected grok executable with the single argument `dashboard` directly in a new tab bound to A and that selected remote directory with no shell first or separate picker while the current tab keeps running and no plugin configuration is injected, the manual shell `grok` is not given the `dashboard` argument and remains a valid way to start Grok, and B's launch bar omits Grok without affecting its terminal

#### Scenario: Persistent metadata and interactive authentication

- Given two saved records, a corrupted-record variant, an existing alias/ProxyJump and isolated agent/key, encrypted-key, password and MFA server fixtures
- When the client restarts, records are edited/removed, and authentication is approved or cancelled through each supported method
- Then valid metadata survives without credentials, corruption preserves the original and local startup, live tabs retain their original authority, all supported prompts work without logging/storing answers, and cancelling one attempt leaves sibling tabs intact

### Requirement: Grouped changes

The git view MUST present changes in the sections "Merge Changes", "Staged Changes", "Changes", and "Untracked", in that order. A section with no rows MUST be omitted and MUST NOT show a zero. Each visible section title MUST be "{title} ({count})", for example "Changes (5)". The view MUST NOT present the changes as one flat list and MUST NOT show a commit box. A repository with no listed changes MUST show "No changes". Ignored paths MUST NOT be listed and MUST NOT mark a folder or a file row. A path that is both staged and unstaged MUST appear in both "Staged Changes" and "Changes", each with that side's status dot. A conflicted path MUST appear in "Merge Changes" with a Conflict dot and MUST NOT also appear as an ordinary staged or unstaged row. Each row MUST show its path relative to the explorer root. The git view and the file-tree change dots MUST include only changes under the explorer root. A change outside that root MUST NOT be listed or marked. Status MUST be read with submodule content changes ignored (`--ignore-submodules=dirty`) under `Hardened read-only git reads`: a submodule whose recorded commit changed MUST be listed like any changed path, but a submodule whose only differences are uncommitted or untracked changes inside it MUST NOT be listed, counted or marked. This is a deliberate security trade-off, not a defect.

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

#### Scenario: Dirty submodule content is not listed

- Given a git worktree with submodule `libs/a` whose working tree has uncommitted and untracked changes but whose checked-out commit equals the recorded one, and submodule `libs/b` whose checked-out commit differs from the recorded one
- When the git view, the file tree and the Open workspace pills are shown
- Then `libs/b` is listed under "Changes" and counted as modified, and `libs/a` is not listed, not counted and does not mark its folder

### Requirement: Calm Editor Visual System

Made MUST use the implemented Calm Editor visual system for the native shell: dark layered surfaces, one calm mint-green accent, bundled Ubuntu-Light proportional and Hack monospace UI typography with bundled DejaVu Sans Mono and a bundled Iosevka symbols fallback for the terminal grid, a 4/8-point spacing rhythm, 8–10px card radii, 4px window gutters, 1px borders, and opaque preview, diagnostics, tooltip, toast, and other non-terminal reading surfaces, with the Open workspace card as the documented conditional-glass exception. The accent MUST be `ACCENT #8BD5A7`, with `ACCENT_HOVER #A3E3BA`, `ACCENT_PRESSED #72BF8E`, `ACCENT_TEXT #08140D`, `ACCENT_TINT #26352C`, `FOCUS #B8F0C9`; it applies to the wordmark `M`, primary accent actions, focus rings, busy spinners, and the explorer splitter while dragging, but MUST NOT be used as a workspace-tab outline. The terminal cursor MUST use the neutral terminal foreground `#D7DDE7` rather than the accent; when the pane is focused and the terminal reports a blinking cursor style it MUST blink with a ~530 ms on/off phase, while a steady reported style or an unfocused pane MUST keep the cursor steady and visible. The no-native/fallback clear color MUST use `BG_CLEAR` alpha 215/255, while an actually applied native backdrop keeps `BG_CLEAR_NATIVE` alpha 135/255. Workspace tabs use opaque `BG_TAB_SELECTED #1B231F` and `BG_TAB_HOVER #151C18` fills over the translucent black `BG_TAB_IDLE`; only the selected tab paints a 1px inside stroke, including when that selected tab is hovered: the dark tab hue darkened by 12 at alpha 128, the same strength as the previous lighter step but not a light edge, and not `BORDER_CONTROL` or another contrasting gray. Inactive tabs paint no stroke at rest and no stroke on hover. That stroke is not `ACCENT`, `FOCUS`, or a selection cue. Card borders remain neutral and the tab close hover remains `ERROR`. The full-height explorer card/header MUST use the documented conditional glass: when a native backdrop was actually applied (macOS HudWindow vibrancy or Windows 11 Acrylic), they use `BG_CARD_GLASS_NATIVE` 24/255 and `BG_HEADER_GLASS_NATIVE` 32/255 so the native material reads through; when no native backdrop is applied (disabled, unsupported, or failed), they use the darker fallback `BG_CARD_GLASS` 188/255 and `BG_HEADER_GLASS` 194/255 for legibility without blur. Windows MUST use the same applied/fallback selection. The painted soft shadow and 1px top highlight remain. The terminal card fill MUST use `BG_TERMINAL_GLASS` alpha 160/255 over the `BG_TERMINAL` RGB (11/13/16) only while the terminal is on its primary screen (no alternate-screen TUI), and it MUST be the sole glass layer: the terminal body and terminal-owned path-line chrome MUST NOT paint another translucent fill. When the terminal is in alternate-screen mode (the VT `ALT_SCREEN` signal used by OpenCode, vim, and less), the terminal card fill, terminal-owned path-line chrome, and body MUST use fully opaque `BG_TERMINAL` 255/255 so TUI cells remain on an opaque surface. The optional session strip/header remains opaque `BG_HEADER`. Preview, diagnostics, tooltip, and toast remain opaque. The Open workspace card MUST use the same conditional glass as the explorer card: `BG_CARD_GLASS_NATIVE` 24/255 when a native backdrop was actually applied and the fallback `BG_CARD_GLASS` 188/255 otherwise, with the painted soft shadow and 1px top highlight; its text fields, folder and worktree rows, pills, notices and buttons keep their own opaque fills and the reading-surface contrast of `Reading-surface contrast`. The workspace header MUST float directly over the window clear or native backdrop with no persistent band fill, border, shadow, or top highlight; the wordmark, workspace tabs, and icon actions are painted directly on that backdrop. A native macOS/Windows backdrop MAY layer behind the window only through the platform boundary; when unavailable, disabled, or unsupported, the in-app explorer glass MUST remain and the reason MUST be honest. Except for the documented primary-screen terminal glass and the Open workspace card glass, the UI MUST NOT introduce gradients or glows, multiple accents, accent-colored status, opacity on reading surfaces, or text below 10px. The no-webview runtime constraint is owned by the core Responsiveness and Lightness requirement.

#### Scenario: Calm Editor tokens and typography

- Given the running Made UI
- When its visual tokens are inspected
- Then the documented Calm Editor colors including the mint-green accent/focus tokens and the neutral terminal cursor, the fallback clear alpha 215/255 with native 135/255, the explorer card/header fallback glass alphas 188/255 and 194/255 with native tints 24/255 and 32/255 selected by the actual applied state, the `BG_TERMINAL_GLASS` 160/255 primary-screen terminal alpha with opaque `BG_TERMINAL` 255/255 in alternate-screen mode, the opaque dark green-gray `BG_TAB_SELECTED`/`BG_TAB_HOVER` tab fills, typography roles, spacing, radii, target sizes, and one-accent rule are used, semantic success/warning/error/status tones remain visually distinguishable and shape/text-backed, neutral card borders are unchanged, and preview, diagnostics, tooltip, and toast remain opaque while the Open workspace card follows its conditional glass

#### Scenario: Terminal cursor color and blink

- Given a focused terminal pane whose terminal reports a blinking cursor style, then a steady style, then a pane that loses focus, and then that same pane regaining focus
- When the cursor is rendered while idle, while the user types, and after focus is regained
- Then the cursor uses the neutral terminal foreground `#D7DDE7` rather than the mint accent; while focused with the reported blinking style it alternates about 530 ms on and off, resets to visible on input or focus gain, and schedules the next repaint at the phase boundary instead of repainting continuously; with a steady reported style or without focus it stays steady and visible; and the existing block, hollow block, underline, and bar shapes are unchanged

#### Scenario: Explorer glass and transparent header

- Given a workspace with the floating header and explorer visible
- When the UI is rendered
- Then the explorer card and explorer header use the native 24/255 and 32/255 tints when a native backdrop is applied and the fallback 188/255 and 194/255 fills otherwise, with the documented shadow and top highlight; the header controls float directly over the clear/native backdrop without a persistent band, the terminal follows the conditional `BG_TERMINAL_GLASS`/opaque `BG_TERMINAL` rule, and preview, diagnostics, tooltips, and toast remain opaque and readable, and the Open workspace card's text stays readable on its conditional glass, when alpha is ignored

#### Scenario: Conditional Explorer glass

- Given an actually applied native backdrop (macOS HudWindow or Windows 11 Acrylic), then `MADE_NATIVE_BACKDROP=off`, an unsupported compositor, or a failed request
- When the Explorer card and header render
- Then with the native backdrop applied they use `BG_CARD_GLASS_NATIVE` 24/255 and `BG_HEADER_GLASS_NATIVE` 32/255; otherwise they use `BG_CARD_GLASS` 188/255 and `BG_HEADER_GLASS` 194/255; the choice follows the actual applied status rather than the platform name, Windows uses the same selection, and the border, soft shadow, and 1px top highlight remain unchanged

#### Scenario: Explorer fallback glass readability

- Given no native backdrop (disabled, unsupported, or failed) and a bright desktop behind the translucent window
- When the Explorer card and header render
- Then they use the darker 188/255 and 194/255 fallback glass rather than the native 24/32 tints, primary and secondary Explorer text remain legible without blur, the fallback legibility check covers a bright composited desktop, and native glass text contrast is not claimed

#### Scenario: Conditional terminal glass

- Given a terminal on its primary screen, then a full-screen TUI that enters the alternate screen (for example OpenCode, vim, or less), then returns to the primary screen
- When the terminal surface is rendered
- Then while the primary screen is active the terminal card fill uses `BG_TERMINAL_GLASS` alpha 160/255 over the `BG_TERMINAL` RGB (11/13/16) and is the only glass layer: the terminal body and terminal-owned path-line chrome paint no second translucent fill; while the terminal reports the VT `ALT_SCREEN` mode, the terminal card fill, path-line chrome, and body use fully opaque `BG_TERMINAL` 255/255; the optional session strip/header remains opaque `BG_HEADER`; the surface follows that mode signal rather than title/content heuristics and returns to the single glass layer when the alternate screen exits; and preview, diagnostics, tooltip, and toast remain opaque while the Open workspace card follows its conditional glass

#### Scenario: Floating header controls

- Given a workspace header over the in-app clear or an applied native backdrop
- When the header is rendered at 1280×800 or 720×480
- Then no persistent band fill, border, shadow, or top highlight is painted behind the wordmark, tabs, or icon actions; the card content starts at 38/34 and the 26/24 strip is vertically centered in that header band with 6/5 above and below it; the header row spans the card left/right edges with no extra horizontal padding; the last right-side icon’s 26/24 target, selected/hover fill, and focus ring all end at the card right edge with no gutter overhang, and its 14px glyph remains centered with normal optical padding on both sides; and header controls share one vertical center with square hit targets and vertically centered glyphs

#### Scenario: Native backdrop auto and fallback

- Given `MADE_NATIVE_BACKDROP=auto`, `off`, or an unsupported platform/compositor
- When the first UI frame requests or skips the native material
- Then the UI uses native clear alpha 135/255 and the native Explorer tints 24/255 and 32/255 only when an effect was actually applied; otherwise it uses the darker fallback clear alpha 215/255 and fallback Explorer glass 188/255 and 194/255 with `applied=false` and an honest reason; Windows uses the same applied/fallback selection; no native effect is claimed without runtime evidence

#### Scenario: Reading-surface contrast

- Given primary, secondary, tertiary, accent, status, focus, terminal, and control-boundary text and surfaces
- When contrast is measured against the documented pairs
- Then the documented ratios hold for opaque reading surfaces, the fallback Explorer glass is checked for legibility over a bright composited desktop, the native Explorer tints and primary-screen terminal glass are not claimed to meet a separate text-contrast ratio without evidence, decorative borders are not the sole control boundary, and native glass/blur contrast is not claimed without evidence

#### Scenario: Open workspace card glass

- Given the Open workspace screen at 1280×800 and 720×480, first with a native backdrop applied and then with native backdrop disabled or unavailable
- When the card renders over the window backdrop
- Then the card uses `BG_CARD_GLASS_NATIVE` 24/255 with the native backdrop and `BG_CARD_GLASS` 188/255 without it, with the soft shadow and 1px top highlight, the backdrop shows in the margins around the card, and the address bar, folder and worktree rows, pills, notices and launch buttons keep their opaque fills and remain legible

### Requirement: Optional Session Map Presentation

Made MUST present the OpenCode session map as an optional, initially hidden surface scoped to the selected workspace and visually distinct from workspace tabs. The session map is OpenCode-only and MUST say so: the Sessions action's tooltip and accessible name MUST be `Show OpenCode sessions` while the map is closed and `Hide OpenCode sessions` while it is open, and the map's empty state MUST read `No OpenCode sessions`. While the tab's running target is another integrated TUI (Grok Build or Claude Code), the map MUST be hidden and reserve no space, even if it was previously toggled open; it MAY reappear when that program exits and the map's open state still applies. It MUST NOT reserve an empty strip before OpenCode data exists or when the map is closed. When opened, it MUST show each session's title and path, exactly one active session, and busy, attention, unread, and renaming indicators using shape plus text, not color alone. The active session tab MUST use a `BG_SELECTED` background with a `TEXT_PRIMARY` 13.0px label and MUST NOT draw an underline, border, or width animation; inactive session-tab labels use 12.5px text at reduced opacity. Long labels and overflow MUST remain inspectable; the map MUST NOT hide sessions from its overflow list. Session data, focus behavior, and failures are owned by the core spec.

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

#### Scenario: Session map is labeled and scoped to OpenCode

- Given a shell tab with the map closed, then opened with no OpenCode session, then a tab whose running target is Grok Build after the map was toggled open, then a Claude Code tab
- When the Sessions action and the map are inspected by pointer and keyboard
- Then the action reads `Show OpenCode sessions` while closed and `Hide OpenCode sessions` while open, the empty map reads `No OpenCode sessions`, and in the Grok Build and Claude Code tabs the map is hidden and reserves no space although it was toggled open

### Requirement: Diagnostics Presentation

When revealed from About, Diagnostics MUST be presented as the existing opaque Diagnostics card with a status pill, reason, copy actions, sectioned connection/session/worktree/version/metrics/log detail, bounded log block, and a path/copy footer. OpenCode MUST be an ordinary integration row like the Grok Build and Claude Code rows, not the card's own status; the Grok Build row is owned by `specs/grok-session-follow.md` and the Claude Code row by `specs/claude-code-session-follow.md`, each authoritative for its row. The card's headline (status pill and reason) MUST summarize the tab's overall state across its rows: the most severe actionable row (an unsupported, degraded or failed state; ties broken by catalog order OpenCode, Grok Build, Claude Code), where a row whose program is merely absent (not on PATH, not detected or not installed) is not actionable unless that program is the tab's active program, and such rows still appear in the list; otherwise the row sentence of the program running as the tab's active program, which is its ready sentence when ready; otherwise, for a plain shell with no integration active, the neutral sentence `No program integration is active.` with the inactive pill. The sections that only carry OpenCode bridge data MUST be titled `OPENCODE CONNECTION` and `OPENCODE SESSION`; the version row `Plugin` MUST be labeled `OpenCode plugin`; and the copied diagnostics keys `plugin version` and `protocol version` MUST be `opencode plugin version` and `opencode protocol version`. Section and row wording that applies to every program MUST follow `Program-neutral wording`. Under `LOG`, the card MUST show a compact Previous log path row. When the previous log exists, the row MUST offer a "Copy previous log path" action, show the full path in a tooltip, and remain truncation-safe at 720×480; when it does not exist, the row MUST show a quiet "not yet rotated" state with that action disabled. The existing active-log path and copy row MUST remain. A retention warning or error present while the diagnostics detail is rendering (for example the startup legacy-size warning) MUST use the warning or error tone inside the card. Final-exit write/rotation failures are stderr-only and MUST NOT be claimed as in-card notices. The card MUST remain usable at the documented responsive sizes and MUST NOT turn technical details into routine chrome outside the card. Shell-follow diagnostics MUST expose workspace identity, reporting capability/reason, latest shell cwd, effective root/source, and inactive optional integration without treating shell-only use as degraded. Data fields, status semantics, redaction, retention, and log content are owned by the core spec.

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

#### Scenario: Headline summarizes every integration row

- Given a plain shell tab, then a tab running Claude Code with session follow ready on a machine where OpenCode is not on PATH, then the same tab while the OpenCode row is unsupported, then a remote OpenCode tab with diagnostics copied
- When the user reveals About diagnostics in each case
- Then the headline is `No program integration is active.` with the inactive pill, then the Claude Code ready sentence (not `OpenCode is not on PATH.`, which still appears as the OpenCode row), then the OpenCode unsupported sentence; OpenCode appears as an ordinary row beside Grok Build and Claude Code; the OpenCode-only sections read `OPENCODE CONNECTION` and `OPENCODE SESSION`; the version row reads `OpenCode plugin`; and the copied text uses `opencode plugin version` and `opencode protocol version`

### Requirement: OpenCode diagnostics wording

The OpenCode diagnostics row MUST use the same visible wording pattern as the Grok Build row for the not-running, ready, and not-on-PATH cases. While OpenCode session follow is not connected and no other OpenCode reason applies, the OpenCode pill MUST stay inactive and its sentence MUST be `OpenCode is not running.` While OpenCode session follow is connected, including after a reconnect, the visible pill MUST say `ready` in the existing success tone and the sentence MUST be `OpenCode session follow is ready.` The machine-readable status code MAY stay `connected`, and that code is what connected status means in Diagnostics. When OpenCode is not on PATH, the sentence MUST be `OpenCode is not on PATH.` and the pill MUST stay the existing degraded or unsupported treatment that already applies to that reason. The focus-unavailable sentence, wherever it is shown (the session-map focus notice and the diagnostics row), MUST be `Focus is unavailable: OpenCode is not connected.` Every other OpenCode reason sentence MUST stay as it is, including heartbeat, protocol, and directory messages. Every Grok Build sentence required above MUST stay as stated. The OpenCode row MUST NOT show `Optional OpenCode integration is inactive.` This wording MUST NOT change OpenCode launch, follow, or dots, and MUST NOT change the Grok Build tab label or menu row.

#### Scenario: OpenCode is not running

- Given OpenCode session follow is not connected and no other OpenCode reason applies
- When the user reveals About diagnostics
- Then the OpenCode pill stays inactive and its sentence is `OpenCode is not running.`

#### Scenario: OpenCode session follow is ready

- Given OpenCode session follow is connected
- When the user reveals About diagnostics
- Then the visible pill says `ready` in the existing success tone, the sentence is `OpenCode session follow is ready.`, and the machine-readable status code may still be `connected`

#### Scenario: OpenCode session follow is ready after a reconnect

- Given OpenCode session follow has reconnected
- When the user reveals About diagnostics
- Then the visible pill says `ready` in the existing success tone, the sentence is `OpenCode session follow is ready.`, and the machine-readable status code may still be `connected`

#### Scenario: OpenCode is not on PATH

- Given OpenCode is not on PATH
- When the user reveals About diagnostics
- Then the sentence is `OpenCode is not on PATH.` and the pill stays the existing degraded or unsupported treatment that already applies to that reason

#### Scenario: Other OpenCode reason sentences stay

- Given OpenCode diagnostics are showing a heartbeat, protocol, or directory reason, or any other OpenCode reason that is not the not-running, ready, or not-on-PATH case
- When the user reveals About diagnostics
- Then that reason sentence stays as it is

#### Scenario: Grok sentences stay

- Given the OpenCode row is showing any of its wording cases
- When the user reveals About diagnostics
- Then each Grok sentence stays the one already required for that Grok state

#### Scenario: Focus unavailable names OpenCode

- Given OpenCode is not connected in the tab and the session map still lists a session
- When the user activates that session in the map and then reveals About diagnostics
- Then the focus notice and the diagnostics reason both read `Focus is unavailable: OpenCode is not connected.`

### Requirement: Local preservation and truthful parity evidence (SSH-09)

All existing local workflows MUST remain available and MUST NOT wait for SSH test/detection/connectivity or change local path/security semantics. Remote work MUST remain asynchronous and bounded so stalled servers cannot block input/tab switching/preview return/Explorer controls in other workspaces. Diagnostics MUST identify selected authority, cwd/root/source, shell reporting capability, transport versus OpenCode integration status, remote Grok Build tier status under SSH-10, remote Claude Code status under SSH-11, supported versions and actionable failure reasons without adding routine technical chrome. Remote SSH transport reasons MUST be program-neutral under `Program-neutral wording`: `The SSH session is not attached.`, `The SSH connection was lost; the remote session state is unknown until it reconnects.`, and `The SSH session is connected.` for a tab not carrying OpenCode, including a fresh `Terminal SSH` tab before OpenCode is seen; OpenCode integration status stays in the OpenCode row. Acceptance MUST execute real local and remote workflows in a controlled environment, not only inspect source or mock a PTY. Evidence MUST record client/server OS/architecture/shell/OpenCode versions, network profile, workload, tab count, window size and backend, and distinguish automated/model/software-rendered evidence from native GUI evidence and untested combinations.

A scoped implementation archive of this change folder MAY proceed with environment combinations explicitly labeled `UNVERIFIED` and accepted by the requester, such as the unexecuted native reference slice and full client/server matrix; this records an honest evidence limit, does not satisfy the acceptance above, and MUST NOT remove or downgrade any product guarantee. The bounded Linux loopback CI smoke job is regression evidence for the protocol and harness surface only, not a substitute for the approved client/server matrix or native GUI evidence, and full release acceptance MUST remain blocked until the `UNVERIFIED` combinations execute and their results are recorded.

Initial remote targets MUST be Linux glibc x86_64/aarch64 (glibc 2.31 or newer) and macOS x86_64/arm64 (macOS 12 or newer), with OpenSSH server, POSIX `sh`, sha256sum or shasum, and writable private user app data. tmux MUST NOT be a target prerequisite. The companion MUST operate without a display server, GUI runtime, remote system clipboard, or tmux. Made MUST NOT install tmux or a Rust toolchain on the remote, or in a development container, to satisfy this change. bash, zsh and fish MUST support launch-local cwd reporting on these targets when installed; other shells retain honest launch-directory/latest-cwd fallback. Windows remotes and Linux musl targets are unsupported and MUST fail capability preflight explicitly. Client targets remain the existing macOS arm64/x64, Linux x64 X11/Wayland and Windows x64, using installed OpenSSH 8.4 or newer and working interactive prompt support. Remote OpenCode v2 is optional and must be detected on the remote user's executable path; no host executable is reused. Remote grok is likewise optional and detected only on the remote PATH under SSH-10, and remote claude is likewise optional and detected only on the remote PATH under SSH-11. Remote python3, or any other interpreter or language runtime, MUST NOT be a prerequisite for any SSH-10 Grok Build tier, for the consented hook install, for hook activity, or for SSH-11 Claude Code hook activity; its presence or absence MUST NOT change any tier, install outcome or diagnostics state, and Made MUST NOT install python3 or any other interpreter on the remote or in a development container.

Controlled acceptance MUST use at least two allocated client/server CPU cores, 4 GiB RAM, a 10,000-file fixture, three live tabs, RTT at most 50 ms, throughput at least 20 Mbit/s and no packet loss. With prerequisites/artifacts present and authentication completed, remote terminal plus first root listing SHOULD become usable within five seconds; reported cwd/session/view changes and visible file create/delete/rename/preview-change-or-stale MUST appear within two seconds; remote Git status SHOULD update within three seconds. Authentication, explicit consent and first artifact transfer MUST be measured separately and excluded only from the five-second warm-start budget. Nonterminal RPC MUST time out within ten seconds unless it is an explicitly displayed/cancellable auth/provision/upload operation; stalled work MUST NOT block the UI. Under a 250-ms RTT/1% loss profile and a 60-second outage, correctness, stale/unavailable feedback, bounded work and same-session reattach remain required, not the low-latency remote budgets. Existing local responsiveness targets remain unchanged; native hardware evidence and all unverified combinations MUST be labeled honestly.

#### Scenario: Local regression under a stalled remote server

- Given an existing local shell/OpenCode workspace and an unreachable/stalled remote server
- When the user starts a remote test, then uses local cwd/session follow, explorer/Git/preview, image/text paste, tab switching and layout controls
- Then local behavior and targets remain unchanged, no network call blocks the UI, and diagnostics attribute failures only to the correct remote authority

#### Scenario: Controlled end-to-end parity evidence

- Given disposable controlled SSH servers matching the approved Q4 matrix, real remote shell/Git/OpenCode fixtures and the local baseline
- When the SSH-01 through SSH-11 scenarios and inherited local scenarios are executed, including actual interrupted SSH, wrong host key, bad authentication, denied provisioning, cross-tab spoofing and traversal/symlink attacks
- Then scenario-to-test evidence and measured budgets are recorded, real versus simulated/native evidence is labeled, and unsupported/untested behavior is not certified as parity

#### Scenario: Remote transport reasons are program-neutral

- Given a fresh `Terminal SSH` tab, a `Grok SSH` tab, a tab whose SSH connection drops, and a tab not yet attached
- When the user reveals About diagnostics for each
- Then the transport reason reads `The SSH session is connected.`, `The SSH session is connected.`, `The SSH connection was lost; the remote session state is unknown until it reconnects.` and `The SSH session is not attached.` respectively, none names OpenCode or another program, and OpenCode status appears only in its own row

### Requirement: Brand and Workspace Tabs

Made MUST present editor-like workspace tabs starting at the window gutter/card left edge, with the shell labels and full-path tooltips defined below and a reachable close affordance; an OpenCode-active tab MUST instead use the OpenCode program label and path-free tooltip defined below. An open Grok Build tab is not required to use that shell label or full-path tooltip; its label and tooltip are owned by `specs/grok-session-follow.md`. A tab where Claude Code is present is likewise not required to use them; its label and tooltip are owned by `specs/claude-code-session-follow.md`. Every workspace tab MUST begin with a 16px leading icon slot at the 10px left inset. An OpenCode-active tab MUST paint the vendored 16px `opencode.svg` in its shipped colors in that slot, while every other workspace tab—including a plain shell tab with no OpenCode integration—MUST paint the existing app-drawn terminal glyph centered in the same slot, colored like the tab label: `TEXT_PRIMARY` when the tab is active or hovered and the inactive tab text otherwise. A Grok Build tab is not required to paint that glyph; its leading icon is owned by `specs/grok-session-follow.md`. A Claude Code tab is likewise not required to paint it; its leading icon is owned by `specs/claude-code-session-follow.md`. The label MUST start after the icon slot, and that slot MUST always be reserved in the label-width budget so the icon alone never changes tab width. The header row MUST span the card left/right edges with no extra horizontal padding. The last right-side icon’s 26/24 target, selected/hover fill, and focus ring MUST all end at the card right edge with no gutter overhang; the 14px glyph MUST remain centered inside its target with normal optical padding on both sides. The two-tone monospace `MADE` wordmark with its accent `M` and its `About` trigger identity MUST sit to the right of the tab strip, immediately before the right icon group, with a reserved 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark (the visible flexible space after the last visible control is larger when few tabs are shown), the same exact 24px wide / 18px compact gap from the wordmark to the first right icon, and no divider. The wordmark MUST NOT look like a tab and MUST NOT paint a fill, hover, or selection treatment. It MUST be the persistent About trigger defined by the Minimal Workspace Layout requirement, which opens the About card and its on-demand diagnostics detail, with the `About` tooltip and accessibility name, a pointer cursor, a target of at least 24×24, a visible keyboard focus ring, and Enter/Space activation. The top workspace strip and workspace tabs MUST both be 26/24 high; the session-map strip/tabs keep their previous 36/34 and 28/26 metrics unless they directly depend on the reduced top height. Workspace tabs MUST be modestly wider than the previous layout so longer labels stay usable: the base tab width MUST clamp to 84–230px in compact layout and 96–280px in normal layout. A fixed 12px text-growth allowance MUST be included beyond the reserved slots, keeping an OpenCode tab's usable label area about 12px wider than the previous layout at the same label length. The 16px leading icon slot with its 6px gap, a fixed 14px right-edge allowance, and the close slot MUST be reserved in the label layout; that allowance keeps the label clear of the corner session-dot/close treatment. The session-dot row MAY add up to 31px beyond the base width when needed, giving final maxima of 261px compact and 311px normal with 8 dots; that added width MUST keep the label area unchanged, MUST keep the label from overlapping the dots, and MUST NOT animate. The OpenCode icon MUST NOT add width when it appears or disappears; any width growth comes only from the session-dot row. The selected workspace tab MUST use the opaque `BG_TAB_SELECTED #1B231F` dark green-gray fill with a `TEXT_PRIMARY` 13.0px label, the faint 1px inside stroke, and no underline, dot/hollow marker, accent bar, or width animation. Selection MUST also carry a subtle non-color cue: the selected label is set at 13.0px versus 12.5px for inactive tabs, vertically centered, with tab width and layout unchanged and no animation; the selected fill is also a luminance step above the idle/hover fills. Inactive workspace tabs MUST use the translucent black `BG_TAB_IDLE` fill, no stroke, and secondary text at reduced opacity. Hovering an inactive workspace tab MUST replace the idle fill with the slightly darker opaque `BG_TAB_HOVER #151C18` fill, paint no stroke, and restore the label to full-opacity `TEXT_PRIMARY`; a selected tab keeps `BG_TAB_SELECTED` and the faint inside stroke on hover. The tab close affordance MUST keep its 24×24 hit area and keyboard focusability reserved at all times. That target MUST remain centered at the tab's right edge, and its `×` glyph MUST be hidden at rest—including on the selected tab—and MUST appear only while the tab is pointer-hovered, the close affordance is pointer-hovered, or the tab or close action has keyboard focus. The handoff between the session dots and the `×` MUST be a smooth crossfade of about 0.15s with quadratic ease-out, not an instant swap: while the close glyph is becoming visible, all session status dots MUST fade out with `(1 - reveal)` opacity and the `×` MUST fade in with `reveal` opacity, so mid-transition both may be partially visible; when fully hovered or focused only the `×` remains at the right corner, and leaving MUST reverse the crossfade smoothly. At rest, the session dots MUST be fully visible when the workspace has open sessions and the `×` MUST be hidden. The close affordance and the crossfade MUST paint no background fill. Hovering only the close icon MUST turn the visible glyph `ERROR` and MUST also apply the whole tab's hover fill and label treatment without changing tab width, label truncation, or layout. Direct close hover MUST still show the close tooltip `Close workspace` instead of the tab's tooltip. The glyph MUST use the fixed documented size with no scale or width animation. When OpenCode is active in that workspace, the tab MUST paint the vendored `opencode.svg` icon in its leading icon slot before its label at 16px in its shipped colors with no tint and a 6px gap to the label. OpenCode is active when its integration is connected in that workspace—either a directly launched OpenCode TUI or a shell tab where the user ran `opencode` manually—or when the tab was launched directly as the OpenCode TUI and its PTY child is still running. The icon MUST be absent only when the integration is disconnected and no directly launched OpenCode TUI's PTY child remains running; it MUST be absent on shell tabs with neither a connected OpenCode integration nor a running directly launched TUI child, while a shell tab where the user ran `opencode` manually remains OpenCode-active while that integration is connected; a tab without the OpenCode icon MUST paint the app-drawn terminal glyph in its leading icon slot, except a Grok Build or Claude Code tab, whose leading icon is owned by `specs/grok-session-follow.md` or `specs/claude-code-session-follow.md` respectively. When OpenCode is active in that workspace, the tab label MUST be the OpenCode program label instead of the shell label, and it MUST revert to the shell label when the workspace stops being OpenCode-active, except an open Grok Build or Claude Code tab, whose label is owned by `specs/grok-session-follow.md` or `specs/claude-code-session-follow.md` respectively; non-OpenCode tabs MUST keep shell labels, except an open Grok Build or Claude Code tab, whose label is owned by `specs/grok-session-follow.md` or `specs/claude-code-session-follow.md` respectively. An OpenCode-active tab's tooltip MUST omit the workspace path: it MUST show the full session title when the label shows one, followed by ` — ` and the session summary when the workspace has open sessions, and with neither it MUST show no parent tooltip. Hovering its close affordance MUST still show `Close workspace`, and non-OpenCode tabs MUST keep the full-path tooltip, except an open Grok Build or Claude Code tab, whose tooltip is owned by `specs/grok-session-follow.md` or `specs/claude-code-session-follow.md` respectively. The OpenCode tab's accessible name MUST be `OpenCode`, then ` — ` and the session title when the label shows one, then ` — ` and the summary when present, for example `OpenCode — Fix login flow — Sessions: 2 running, 1 needs attention, 1 idle`. When OpenCode is active in that workspace and the workspace has open sessions, the tab MUST paint one 4px diameter (2px radius) status dot per open session in `SessionMap` row order, with 3px gaps, right-aligned so the last dot is centered at `tab.right() - 10`, and vertically centered. At most 8 dots MAY be shown; sessions beyond 8 MUST remain in the tooltip and accessibility count summary but MUST NOT get a visual marker. Each dot MUST reflect its own session state: busy uses `ACCENT` and MUST gently pulse via opacity only, about a 1.8s period with opacity between roughly 0.35 and 1.0, without movement or size change; attention uses static `WARNING`; idle or stopped uses static neutral `TEXT_TERTIARY`. Within one session, attention takes precedence over busy; attention is per session and MUST NOT suppress another session's busy dot. The dots MUST NOT convey status by color alone: the tab tooltip and accessible name MUST summarize all sessions, including sessions beyond the 8 visible dots, for example `Sessions: 2 running, 1 needs attention, 1 idle`, omitting zero categories. The session-dot row is a state-driven operational indicator separate from selection: it MUST NOT act as, or add, a selection underline, selection dot or hollow marker, accent bar, or width animation, and MUST NOT alter the selected tab's documented fill, stroke, or 13.0px label. Workspace tabs MUST remain visually distinct from optional OpenCode session tabs and MUST NOT animate their width. When tabs do not fit, a labeled, keyboard-reachable overflow list MUST expose every workspace tab; labels MUST truncate with an ellipsis and retain full-value tooltips.

Workspace tab labels are defined here and are authoritative for every tab label rule of the live contract. A shell (terminal) tab, local or SSH, MUST be labeled with the shell label: the basename of the shell's current directory, following each validated cwd report (an SSH tab uses the remote shell's cwd; the filesystem root is shown as `/`), with the full path in its tooltip; an SSH tab's tooltip MUST be the server display name, `: `, and the full remote path. A tab whose OpenCode, Claude Code or Grok Build presentation applies under its own requirement MUST be labeled with that program's program label: the active session's title (OpenCode: the active or focused session's title; Claude Code: the hook-derived sanitized session title of its most recent live owned session; Grok Build: the session title or summary when the inventory exposes one), or the catalog label `OpenCode`, `Claude Code` or `Grok Build` when there is no session or no title. Every label MUST be sanitized (control characters and newlines replaced with spaces, bidirectional and format control characters removed), bounded to at most 120 characters before layout, and ellipsized to fit the tab; the tooltip MUST carry the full sanitized value. A program tab's accessible name MUST be its catalog label, then ` — ` and the session title when the label shows one, then ` — ` and the status summary when present. The leading program icon, status dots, close behavior and presentation precedence are unchanged by these label rules. Wherever another requirement of the live contract says a tab uses the exact `OpenCode`, `Grok Build` or `Claude Code` label, a folder label or folder name, or the server-name label, that label MUST be read as the program label or the shell label defined here.

#### Scenario: MADE wordmark identity

- Given the running UI
- When the header brand is inspected or activated by pointer or keyboard
- Then workspace tabs start at the window gutter/card left edge, the last right-side icon’s 26/24 target, selected/hover fill, and focus ring end at the card right edge with the 14px glyph centered inside the target with normal optical padding, the top strip and tabs are 26/24 high and vertically centered in the 38/34 header band with 6/5 above and below, and `MADE` is a cached two-tone monospace wordmark with accent `M`, primary `ADE`, the documented tracking and mint accent, the `About` tooltip and accessibility name, a pointer cursor, a target of at least 24×24, a visible keyboard focus ring, Enter/Space activation that opens the About card, a reserved 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark (visible space after the last visible control may be larger), an exact 24px wide / 18px compact wordmark-to-first-icon gap, no divider, no tab fill/hover/selection, and no `BG_HOVER`, `BG_SELECTED`, `BG_TAB_HOVER`, or `BG_TAB_SELECTED` fill

#### Scenario: Workspace tab presentation

- Given two or more open workspaces, including an OpenCode-active workspace and a shell workspace with no OpenCode integration
- When the tab strip is displayed
- Then the shell tab shows the app-drawn terminal glyph centered in its 16px leading icon slot at the 10px left inset, colored like its label, except a Grok Build or Claude Code tab, whose leading icon is owned by `specs/grok-session-follow.md` or `specs/claude-code-session-follow.md` respectively, plus its shell label starting after that slot and full-path tooltip, except an open Grok Build or Claude Code tab, whose label and tooltip are owned by `specs/grok-session-follow.md` or `specs/claude-code-session-follow.md` respectively, plus its close affordance, while the OpenCode-active tab shows the vendored 16px OpenCode icon in its shipped colors in the same leading icon slot with a 6px gap before its label, and the OpenCode program label starting after the icon slot, a path-free tooltip (the full session title when shown, then the session summary, or no parent tooltip when it has neither), and its close affordance, while that shell tab shows no OpenCode icon; both tabs always reserve the icon slot in the label-width budget; tabs keep base widths of 84–230px compact and 96–280px normal with the fixed 12px text-growth allowance plus the reserved icon, fixed 14px right-edge allowance, and close slots accounted for, so an OpenCode tab's usable label area is about 12px wider at the same label length; a session-dot row MAY add up to 31px beyond the base to final maxima of 261px compact and 311px normal for 8 dots, keeping the label area unchanged and clear of the dots without animation, and the icon alone does not change tab width; the selected tab uses the opaque dark green-gray `BG_TAB_SELECTED` fill, a `TEXT_PRIMARY` 13.0px label, the faint 1px inside stroke, and no underline, dot/hollow marker, accent bar, or width animation; inactive tabs use the translucent black `BG_TAB_IDLE` fill, no stroke, secondary text at 12.5px and reduced opacity; and session tabs are not presented as workspace tabs

#### Scenario: Workspace tab OpenCode icon presence

- Given a workspace launched directly as the OpenCode TUI whose integration disconnects while its PTY child still runs, then that child exiting; and a shell workspace where the user runs `opencode` manually, then that integration disconnecting
- When the workspace tab strip is displayed after each change
- Then the icon remains while the directly launched TUI's PTY child still runs even if its integration is disconnected, and appears when the shell's OpenCode integration is connected; it disappears only when the integration is disconnected and no directly launched TUI's PTY child remains running, including on shell tabs with no OpenCode integration; it is the vendored 16px OpenCode icon in its shipped colors with a 6px gap to the label; while the icon is present the tab label is the OpenCode program label, starts after the leading icon slot, and its tooltip omits the workspace path, and when the icon disappears the tab paints the app-drawn terminal glyph in the same leading slot, except a Grok Build or Claude Code tab, whose leading icon is owned by `specs/grok-session-follow.md` or `specs/claude-code-session-follow.md` respectively, and the label reverts to the shell label with the full-path tooltip, except an open Grok Build or Claude Code tab, whose label and tooltip are owned by `specs/grok-session-follow.md` or `specs/claude-code-session-follow.md` respectively; and the icon's appearance adds no width beyond its reserved slot

#### Scenario: Workspace tab OpenCode activity

- Given a workspace whose OpenCode sessions are, in row order, busy, attention, idle, and busy, then a workspace with nine busy sessions, then a workspace with no sessions
- When the workspace tab is displayed
- Then with sessions present the tab paints one 4px diameter (2px radius) dot per session in that row order with 3px gaps, right-aligned so the last dot is centered at `tab.right() - 10` and vertically centered: the busy dots use `ACCENT` and gently pulse via opacity only over about a 1.8s period between roughly 0.35 and 1.0 with no movement or size change, the attention dot uses static `WARNING`, and the idle dot uses static `TEXT_TERTIARY`; attention takes precedence within its own session but does not suppress the other busy dots; the tooltip is `Sessions: 2 running, 1 needs attention, 1 idle` and the accessible name is `OpenCode — Sessions: 2 running, 1 needs attention, 1 idle`; with nine sessions only 8 dots are painted, the ninth session gets no visual marker, the tooltip is `Sessions: 9 running`, the accessible name is `OpenCode — Sessions: 9 running`, and at most 31px of dot-row width is added; with no sessions no dots are painted, no summary is added, no parent tooltip is shown, and the accessible name is `OpenCode`; status is not conveyed by color alone, the tab width does not animate, and the session-dot row neither replaces nor adds a selection underline, selection dot or hollow marker, accent bar, or width animation

#### Scenario: Workspace tab hover and close

- Given selected and inactive workspace tabs with their close affordances
- When an inactive tab or only its close icon is hovered
- Then an inactive tab switches to the opaque `BG_TAB_HOVER` fill, paints no stroke, and shows a full-opacity primary label; a selected tab keeps `BG_TAB_SELECTED` and the faint inside stroke; the hidden-at-rest close glyph appears while the tab is hovered or the close affordance is hovered through a smooth crossfade of about 0.15s with quadratic ease-out in which all session status dots at the right corner fade out with `(1 - reveal)` opacity while the `×` fades in with `reveal` opacity, so both may be partially visible mid-transition before only the `×` remains, paints no background, and, when hovered directly, turns only the glyph `ERROR` while the parent tab also shows its hover fill and primary label, the close tooltip replaces the tab tooltip, including the OpenCode session summary, the tab width/label layout stays unchanged, and leaving reverses the crossfade smoothly

#### Scenario: Workspace tab close visibility

- Given selected and inactive workspace tabs with their 24×24 close affordances, first with OpenCode session dots and then without any
- When the tabs are at rest, then pointer-hovered, then keyboard-focused on the tab or close
- Then the `×` glyph is absent at rest on both inactive and selected tabs, and, when the workspace has open sessions, all session dots are painted right-aligned at the right corner; on tab hover, close hover, or keyboard focus the `×` fades in over about 0.15s with quadratic ease-out while all session status dots fade out with `(1 - reveal)` opacity over the same transition, so both may be partially visible mid-transition and only the `×` is painted once fully hovered or focused; leaving the tab or close reverses the crossfade smoothly; the `×` keeps its 24×24 keyboard-reachable target and focus ring centered at the right edge, and no background is painted; the glyph uses the `ERROR` tone only on direct close hover, never scales, and the crossfade changes neither tab width nor label layout

#### Scenario: Workspace tab overflow and close

- Given enough workspace tabs to exceed the available width at 1280×800 or 720×480
- When the header lays out
- Then a labeled keyboard-reachable overflow list exposes every tab, the active tab remains visible, no tab or close affordance is clipped, and closing still targets only its workspace

#### Scenario: Shell tab follows cd

- Given a local shell tab started in `/work/made` and an SSH tab on server `prod` started in `/srv/app`
- When the user runs `cd crates/made-app` locally, `cd /var/log` remotely, then `cd /` locally
- Then the local tab label becomes `made-app` with tooltip `/work/made/crates/made-app`, the SSH tab label becomes `log` with tooltip `prod: /var/log`, the local label then becomes `/`, and no other tab changes

#### Scenario: TUI tab shows the session title, then falls back

- Given an OpenCode tab whose active session is titled `Fix login flow`, a Claude Code tab whose owned session's hook title is `Refactor tabs`, and a Grok Build tab whose inventory exposes no title
- When the tabs render, and later the OpenCode session closes with no other session and the Claude Code session ends while the program keeps running
- Then the labels are `Fix login flow`, `Refactor tabs` and `Grok Build` with their program icons, dots and close behavior unchanged, tooltips carrying the full titles and accessible names such as `OpenCode — Fix login flow — Sessions: 1 running`; afterwards the labels fall back to `OpenCode` and `Claude Code`

#### Scenario: Long titles ellipsize within the new maxima

- Given a tab whose session title is 300 characters with a newline and a bidirectional control character, at 1280×800 and 720×480, with and without 8 session dots
- When the tab strip lays out
- Then the title is sanitized and bounded before layout, the base width never exceeds 280px normal and 230px compact (311px and 261px with 8 dots), minima stay 96px and 84px, the label ellipsizes without overlapping the dots or close target, and the tooltip shows the full sanitized title

### Requirement: Claude Code tab presentation

While OpenCode is not active in a tab and Claude Code is present there, the tab MUST use the Claude Code program label of `Brand and Workspace Tabs` (the hook-derived sanitized session title, else `Claude Code`) and paint the Claude Code mark in its 16px leading icon slot, even before any session or status exists, and Grok Build presentation MUST yield to it. The Claude Code mark MUST be the official Claude Code mark, shipped as an app asset and painted at 16px in its official colors with the same treatment as the official Grok mark, without redrawing or tinting, and it MUST be the same asset the `+` menu row paints. The tab MUST paint one status dot per live owned Claude Code session, or exactly one dot when Claude Code is present with a protocol status and no live owned session, using the same size, placement, cap of eight, pulse, colors, tooltip summary and accessible-name summary as OpenCode session dots. A dot's status MUST be the tab's protocol status when one is set, applied to the tab's most recent live owned session, otherwise that session's hook-derived status, otherwise idle; busy is busy, attention and error are attention, and idle is idle. A finished turn MUST show idle. The tooltip MUST omit the workspace path and MUST be the session title, followed by ` — ` and the session summary when dots exist, where the session title is the first of these that is non-empty: the terminal's window title, which is the title the program in that tab sets for its own terminal and never the OS window title, read only while a Claude Code protocol status is live in that tab; the most recent session title carried by an owned record of the tab's most recent live owned session; `Claude Code`. The window title is untrusted and MUST be sanitized by replacing control characters and newlines with spaces and removing bidirectional and format control characters and truncated to at most 120 characters. The accessible name MUST be `Claude Code`, then ` — ` and the session title when the label shows one, then ` — ` and the session summary when dots exist. When Claude Code is no longer present and no Grok Build presentation applies, the label, tooltip and leading icon MUST return to the shell label, full-path tooltip and terminal glyph, and its dots MUST be removed. If OpenCode is active, the label, leading icon and dots MUST stay OpenCode's even while Claude Code is present. A terminal window title applies only to its own tab: the OS window title MUST always be `Made` and MUST NOT be derived from any tab's terminal title, any status or the workspace.

#### Scenario: OS window title stays Made

- Given two tabs whose programs, including Claude Code, set different terminal window titles, in any workspace
- When the user switches between the tabs and the titles change
- Then the OS window title is `Made` throughout and never contains a tab's terminal title or the workspace name

#### Scenario: Direct launch is branded before any status

- Given the user chose the Claude Code row
- When the new tab is shown before any status emission or hook record arrives
- Then its label is the catalog label `Claude Code` (no session title is known yet), its leading slot paints the Claude Code mark, it shows no dot, and its tooltip is `Claude Code` without the workspace path

#### Scenario: Dots follow protocol status with a title

- Given a live owned Claude Code session, a protocol emission with state `working`, and Claude Code setting its window title to `Refactor parser`
- When the tab is displayed
- Then it shows one pulsing busy dot, the tooltip is `Refactor parser — Sessions: 1 running`, and the accessible name is `Claude Code — Sessions: 1 running`

#### Scenario: Hook-only dots

- Given a live owned Claude Code session started with `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1`
- When the user submits a prompt and Claude Code later requests a permission
- Then the dot is busy and then attention, and the tooltip begins with `Claude Code`

#### Scenario: Window title only while protocol status is live

- Given Claude Code runs with the status protocol unavailable and sets its window title to `◐ Refactor parser`
- When the tab is displayed
- Then the tooltip begins with `Claude Code` rather than the window title, and no status is derived from the title or its glyph

#### Scenario: Renamed session title from hooks

- Given a live owned Claude Code session with the status protocol unavailable
- When the user renames the session to `foo` and submits the next prompt
- Then the tooltip begins with `foo`

#### Scenario: Empty window title falls back

- Given a live Claude Code protocol status and an owned session that has carried no session title
- When Claude Code sets an empty window title
- Then the tooltip begins with `Claude Code`

#### Scenario: Untrusted title is sanitized

- Given a window title read while protocol status is live, containing control characters, newlines, bidi controls and more than 120 characters
- When the tab tooltip and copied diagnostics show it
- Then controls and newlines are replaced, bidi and format controls are removed, and at most 120 characters are shown

#### Scenario: Claude Code exits and the tab returns to its folder

- Given a shell tab branded `Claude Code` with no open Grok session
- When Claude Code exits and the shell is back at its prompt
- Then the tab returns to the shell label, full-path tooltip and terminal glyph, and its Claude Code dots are removed

#### Scenario: Claude Code outranks Grok presentation

- Given a shell tab with two open Grok sessions showing `Grok Build` and Grok dots
- When the user starts Claude Code in that tab and later exits it
- Then while Claude Code is present the tab shows `Claude Code`, the Claude Code mark and only Claude Code dots, and after it exits the tab returns to `Grok Build`, the official Grok mark and the Grok dots

### Requirement: Claude Code channel fallback

The status protocol and the session hooks MUST each work without the other, and losing either MUST NOT break the terminal, shell, explorer, diagnostics or other tabs. With both channels, protocol status drives the dots, the window title read under `Claude Code tab presentation` supplies the tooltip title and selects an agent-view session under `Claude Code agent-view session follow`, hook records drive session identity, and hook records together with the `claude agents --json` listing drive the explorer. Without the status protocol, for example with a Claude Code version that does not emit it, inside tmux, in a background session, with `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1`, when no probe reply reaches Claude Code, or whenever Claude Code does not enable the protocol for another reason, hook-derived status drives the dots, the session title comes from hook records or is `Claude Code`, and explorer follow is unchanged except that no agent-view session is selected. Without hooks, for example with `--bare`, safe mode, `disableAllHooks`, a managed-hooks-only policy, a missing shipped plugin, an unwritable spool, or Windows where the hook cannot run, protocol status and the window title drive the presentation, Claude Code does not move the explorer, and the explorer follows the next applicable source. With neither channel, the tab behaves as a plain terminal, except that a directly launched Claude Code tab keeps the Claude Code program label (the catalog label `Claude Code` while no session title is known) and mark while its child runs. Made MUST NOT retry, relaunch, add arguments, change Claude Code configuration, drop, reorder or replace the user's `CLAUDE_CODE_PLUGIN_DIRS` entries, or prompt the user in order to restore a channel.

#### Scenario: No probe reply or tmux

- Given Claude Code receives no status-protocol channel to Made, once because it runs inside tmux started from a Made shell tab and once because the reply does not reach it in a tab where its hooks are owned
- When the user works in Claude Code
- Then in the owned-hooks case the dots come from hook-derived status and the explorer follows the session, and in the tmux case, where the Claude Code process runs outside the tab's process tree, the tab stays a plain terminal with no misattributed session, dot or explorer move

#### Scenario: Bare mode keeps protocol status only

- Given claude 2.1.295 or later started with `--bare` in a local shell tab
- When Claude Code emits `working` and then `done`
- Then the tab shows `Claude Code`, a busy then idle dot, the explorer stays on the shell directory, and the diagnostics row reports hooks unavailable

#### Scenario: Terminal title disabled keeps hooks only

- Given Claude Code started with `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1` in a local shell tab
- When its session starts, receives a prompt and stops
- Then the tab shows `Claude Code` with an idle, busy, then idle dot from hook-derived status, the explorer follows the session directory, and the diagnostics row reports the status protocol unavailable

#### Scenario: Status protocol not enabled while hooks work

- Given Made was started with a user entry in `CLAUDE_CODE_PLUGIN_DIRS`, so a shell tab's Claude Code receives that entry and Made's, and that Claude Code does not enable the status protocol
- When the user works in Claude Code in that tab
- Then hook-derived status drives the dot, the explorer follows the session, the diagnostics row reports `Claude Code status protocol is unavailable.`, and the user's entry stays in the variable in its original position

#### Scenario: Neither channel

- Given Claude Code runs with hooks disabled and the status protocol unavailable, once typed in a shell tab and once launched from the Claude Code row
- When the user works in Claude Code
- Then the shell-typed tab stays a plain terminal with its shell label, the directly launched tab keeps the Claude Code program label and mark with no dot while its child runs, and neither tab's terminal, explorer or diagnostics breaks

### Requirement: Grok tab dots

When OpenCode is not connected in a workspace tab, Claude Code is not present there as `specs/claude-code-session-follow.md` defines, and that tab has open Grok sessions, the tab MUST paint one status dot per open Grok session using the same size, placement, cap of eight, pulse, colors, tooltip summary, and accessible-name summary as OpenCode session dots. Working is busy, attention and error are attention, and idle is idle. A session present only in the inventory, before any hook event, MUST show idle. While OpenCode is not active and Claude Code is not present, a tab whose target is a running Grok TUI, or a shell tab with open Grok sessions, MUST use the Grok Build program label of `specs/made-ui-ux.md` `Brand and Workspace Tabs` (the session title or summary when the inventory exposes one, else `Grok Build`) and the official 16px `grok.svg` mark in the leading icon slot, even before a session exists. Its tooltip MUST be the session summary when sessions exist and MUST omit the workspace path. When that Grok is no longer running and the tab has no open Grok sessions, the label and leading icon MUST return to the shell label and terminal glyph, unless Claude Code is present, in which case `specs/claude-code-session-follow.md` owns them. If OpenCode is active, the label, leading icon, and dots MUST stay OpenCode's even when Grok sessions exist. If OpenCode is not active and Claude Code is present, the label, leading icon, and dots MUST be Claude Code's even when Grok sessions exist.

#### Scenario: Dots match session state

- Given a Grok tab whose sessions are working, waiting on a question, and idle, with no Claude Code present
- When the workspace tab is displayed
- Then it shows a pulsing busy dot, a yellow attention dot, and a neutral idle dot, the label is the Grok Build program label (`Grok Build` when the inventory exposes no title), the leading icon is the official `grok.svg` mark, and the tooltip counts those three categories without the workspace path

#### Scenario: Claude Code takes over and hands back the tab

- Given a shell tab showing `Grok Build` with two open Grok sessions
- When the user starts Claude Code in that tab, and later Claude Code exits while both Grok sessions stay open
- Then while Claude Code is present the label, leading icon and dots are Claude Code's, and after it exits the tab shows `Grok Build`, the official mark and the two Grok dots again

### Requirement: Remote OpenCode session and cwd follow (SSH-06)

Supported remote integration MUST expose the current optional session-map data/actions, activity indicators and focus correlation within the owning remote OpenCode instance. Root precedence MUST remain active OpenCode session directory, then usable connected sessionless view directory when tabs are enabled, then latest validated remote shell cwd, then remote launch directory. Shell cwd MUST continue to be recorded while OpenCode takes precedence. Integration disconnect/normal exit/unsupported state/tabs disabled/new epoch MUST clear view-directory state and resume remote shell fallback without using the host cwd. Worktree/session changes MUST not restart terminal processes or manage worktrees. Missing or incompatible OpenCode/plugin MUST be distinguished from loss of SSH transport; optional integration failure MUST NOT by itself disable a healthy remote filesystem or terminal. Safe shell hooks MUST preserve remote user startup files and existing hooks; unsupported reporting MUST be reported honestly, never replaced by prompt scraping or injected commands in a running shell.

Initial helper/plugin staging, including the Claude Code plugin package and hook of SSH-11, MUST require explicit per-server/version consent before app-owned remote files are written; consent rejection, unsupported artifacts, missing supported-target capability, missing checksum tool, or transfer/integrity failure MUST fail clearly without mutating personal configuration or silently offering terminal-only parity. Existing pre-provisioned matching private artifacts MAY be verified and reused. Made MUST NOT install OpenCode. Made MUST NOT install, probe, execute, configure, or fall back to tmux; a server with tmux absent and a server with a user-installed tmux MUST be treated identically, and that user installation MUST NOT become a dependency, fallback, or hidden prerequisite. The frozen major-1 `Capabilities.tmux` field MUST remain for wire compatibility, MUST be encoded false, and MUST be ignored by every consumer; its presence is not permission to probe or require tmux. Optional OpenCode absence/version/integration failure MUST still leave an otherwise supported terminal/explorer usable. Remote config composition MUST read only remote user config/inline settings and inject only the remote app-owned plugin and remote loopback bridge for that launch; host provider/config/environment secrets MUST NOT be uploaded. Direct OpenCode and manual `opencode` inside a supported remote shell MUST both receive this launch-local integration environment. User shell, OpenCode, SSH, and any pre-existing tmux configuration bytes MUST remain unchanged.

Initial staging and exact reuse MUST remain unchanged: a missing version target is staged once after per-server/version consent, and an existing target whose shipped artifacts verify successfully is reused without rewriting. When an existing same-version target contains a missing, corrupt or different artifact set, Made MUST detect the mismatch, MUST NOT overwrite, delete or repair anything before a separate explicit `Update companion` approval for the current attempt, and MUST report a sanitized update-required state naming the affected version and target instead of a generic `ssh connection failed` or an app-version bump recommendation. The existing per-server/version staging consent remains valid for initial staging and exact reuse but does not authorize changed bytes; refusal or cancellation of the update approval MUST perform no remote write or delete and MUST leave the prior state actionable and retryable.

An approved same-version update MUST run through the fixed staging script: write all 14 shipped artifacts (the companion, the five OpenCode plugin files, the grok-made `plugin.json`, `README.md`, `hooks/hooks.json` and native session-status hook executable built for that target, and the claude-code-made `.claude-plugin/plugin.json`, `README.md`, `hooks/hooks.json` and native session-event hook executable built for that target) into a private temporary sibling of the existing target, verify every size, hash and required permission/executable mode there, then replace the existing target path through the narrowest practical atomic or rollback-safe swap; only after the verified swap may Made remove the temporary and replaced app-owned artifact directories so no per-build copies accumulate. Any failure before the verified swap MUST leave the original target byte-for-byte untouched; after classifying a failed update, Made MUST preserve the actionable failing phase and reason and keep the original reusable or update-required as appropriate. This update path MUST NOT add an arbitrary remote delete, rename or exec API. The companion and Grok-install commands MUST continue resolving the same current version and target path after an update, `plugins/grok-made` MUST remain present there for the consented installer, `plugins/claude-code-made` MUST remain present there for the SSH-11 launch environment, and exact reuse of the updated set MUST be verified on the next attempt.

Updating artifacts MUST NOT terminate a running companion, its supervisor, its PTY or its terminals: an already-running process keeps its own OS process lifecycle and open files, while subsequent launches resolve the updated path; reattach semantics, protocol major 1 and existing tabs MUST remain unchanged. A staging or companion-open failure before attach MUST surface a bounded Explorer unavailable/failure state with the actionable update or provisioning reason, MUST NOT display a perpetual `Loading folder…`, MUST NOT claim the Explorer is usable when it is not, and MUST NOT spawn a companion after a failed stage; optional OpenCode/Grok/Claude Code integration failure is not this provisioning state and still leaves a healthy terminal/filesystem usable. Routine SSH chrome MUST NOT be added for this state.

Remote OpenCode workspace-tab presentation and diagnostics MUST match the local presentation for these flows: while the owning remote OpenCode integration is connected, or a directly launched remote OpenCode process is still alive, the workspace tab MUST use the local OpenCode program label, the vendored 16px `opencode.svg` leading icon and the same busy/attention/idle session dots derived from that tab's remote session map, with the local size, placement, cap, pulse, colors, tooltip-summary and accessible-name treatment, and revealed diagnostics MUST report that tab's remote integration status honestly instead of the local host integration state. Manual `opencode` in a supported remote shell and direct `OpenCode SSH` launch MUST receive this presentation identically. When the remote integration disconnects or remote OpenCode exits normally, stale session activity and dots MUST be cleared and the tab MUST return to the normal remote presentation, without losing terminal continuity and without changing server or tab scoping; the server identity MUST remain inspectable under SSH-09 without violating the local path-free tooltip rule. This paragraph changes only labeling/activity presentation and diagnostics sourcing; SSH-07 attach, resnapshot and input gating remain unchanged.

#### Scenario: Remote session precedence and focus

- Given remote shell cwd A, session directory B, connected view V and multiple OpenCode sessions/worktrees
- When the user switches/focuses sessions, shell cwd changes to C, then the active session becomes absent with V usable, then integration exits
- Then the explorer follows B, then V, then C at the appropriate transition; the map/activity/focus results remain server/tab-scoped; the terminal is unchanged; and no host directory is consulted

#### Scenario: Integration failure is not SSH failure

- Given SSH terminal/filesystem transport is healthy but integration becomes incompatible, disconnects, disables tabs or returns an unavailable active directory
- When Made handles the state
- Then existing remote shell fallback/unavailable-root rules apply, diagnostics explain the integration reason, sessionless view state cannot outlive its epoch, and terminal/filesystem access remain usable without silent config changes

#### Scenario: Consented provisioning and manual remote OpenCode

- Given matching packaged artifacts, an isolated remote user config with existing plugins/hooks, plus tmux-absent, user-installed-tmux, denied-consent, corrupt-transfer and unsupported-target variants
- When the user approves or refuses provisioning, opens a remote shell, runs OpenCode manually and directly, and switches sessions/view directories
- Then only approved private verified artifacts are staged, both tmux variants provision and launch identically without probing or invoking tmux, missing checksum/target failures fail with manual instructions rather than installation, supported launches preserve config/plugins/hooks and remote follow, no host config/secrets are sent, and failures do not claim full workspace readiness

#### Scenario: Same-version companion mismatch requires approved update

- Given an existing app-owned same-version target with one changed, missing or corrupt artifact beside intact files, plus intact-matching and absent-target variants
- When the user attempts the remote workspace and then approves, refuses or cancels `Update companion`
- Then detection reports a sanitized version/target update-required state rather than a generic SSH failure or app-version bump advice, nothing is written or deleted before approval, refusal or cancellation writes nothing and leaves the attempt retryable, and the intact-matching variant is reused without rewriting

#### Scenario: Approved companion update is verified, swapped and reused

- Given an approved update, a companion and terminal still running from the old artifacts, and the fixed Grok-install path
- When one update succeeds, another fails before its swap, and a later attempt reuses the result
- Then all 14 artifacts are written to a private temporary sibling and verified for size, hash and permissions, the existing target is replaced atomically or rollback-safely, temporary and replaced app-owned directories are removed without per-build accumulation, the pre-swap failure leaves the original untouched with its actionable phase preserved, the running process keeps its OS lifecycle, later companion, Grok-install and Claude Code launches resolve the same updated version/target path, and the next attempt verifies and reuses the exact updated set

#### Scenario: Pre-attach provisioning failure bounds the Explorer

- Given a staging or companion-open failure before attach, such as an unapproved mismatch, corrupt transfer or failed update
- When the remote workspace is presented
- Then the Explorer shows a bounded unavailable/failure state with the actionable reason instead of perpetual `Loading folder…`, does not claim usability or stream a partial tree, no companion is spawned after the failed stage, and no routine SSH chrome is added

#### Scenario: Remote OpenCode tab parity with the local presentation

- Given a remote workspace whose OpenCode integration is connected with sessions working, awaiting attention and idle, and a second remote OpenCode tab launched directly whose process is alive before the bridge handshakes
- When the tab strip and revealed diagnostics render for both tabs
- Then each tab shows the OpenCode program label, the vendored 16px `opencode.svg` icon and one busy/attention/idle dot per remote session with the local treatment, the directly launched tab keeps that presentation while its process is alive, diagnostics report each tab's own remote integration status rather than the host integration state, and the server identity remains inspectable without a workspace path in the tooltip

#### Scenario: Remote OpenCode exit or disconnect restores remote presentation

- Given a remote OpenCode tab showing the OpenCode program label, icon and session dots
- When the remote integration disconnects or the remote OpenCode exits normally, and the tab later reattaches per SSH-07
- Then stale dots and session activity are cleared at the transition, the tab returns to the remote shell label and terminal glyph, no pre-disconnect session state is applied after reconnect before a fresh snapshot, and the same PTY session and terminal continuity are preserved

### Requirement: Remote Grok Build session follow and tab parity (SSH-10)

Remote Grok support MUST mirror the local Grok Build contract for the covered flows. Remote grok detection MUST be the same bounded metadata-only PATH lookup performed on the server: it MUST ignore empty and relative PATH entries, MUST NOT spawn grok, MUST NOT check a version, MUST NOT install the plugin, and MUST NOT delay terminal or explorer startup. Choosing `Grok SSH` MUST start the detected remote grok absolute path with the single argument `dashboard` as SSH-01 defines, and typing `grok` in a remote shell MUST remain valid, MUST NOT be replaced, and MUST NOT be given the `dashboard` argument. While remote OpenCode is not connected in the tab, Claude Code is not present in the tab under SSH-11, and the tab's remote target is a living grok process, or the tab's remote shell has at least one open Grok session, the tab MUST use the local Grok Build program label and the official 16px `grok.svg` mark in the leading icon slot, even before any session exists; its tooltip MUST be the session summary when sessions exist and MUST omit the workspace path. When that grok is no longer running and the tab has no open Grok sessions, the tab MUST return to the remote shell label and terminal glyph, unless Claude Code is present, in which case SSH-11 owns them. When remote OpenCode is connected, the label, leading icon and dots MUST stay OpenCode's even when Grok sessions exist. When remote OpenCode is not connected and Claude Code is present under SSH-11, the label, leading icon and dots MUST be Claude Code's even when Grok sessions exist.

Made MUST treat the remote `$GROK_HOME/active_sessions.json` (`~/.grok/active_sessions.json` when `GROK_HOME` is unset on the server) as the inventory of sessions that remote Grok still has open, refreshed about once a second by the owning companion. A row belongs to this tab only when its process id is the tab's managed terminal process or a descendant of it, determined by live remote PID ancestry at read time; a stale, exited or foreign pid MUST NOT count. The inventory supplies session id, directory and attachment order and does not supply status. An unreadable or missing file, a truncated or malformed row, more than one matching interpretation of a malformed row, or a row outside this tab MUST NOT move the explorer or change tab presentation, and ingestion MUST stay bounded. This inventory tier requires no plugin, no consent and no remote write.

Hook ingestion requires a grok-made plugin equal to or newer than the shipped one installed in the remote Grok home: when such a plugin is present, whether the SSH-10 consented install put it there or the user installed it themselves (in which case Made performs no Grok-home write and needs no consent), Made MUST ingest the remote `$GROK_HOME/made-grok-status.jsonl` lines appended after the current attachment starts, applying the same event semantics as the local contract: session start and prompt submission move the explorer without waiting for the next inventory refresh, a session start shows idle, a prompt submission shows working, questions and permission requests show attention, finished stops and idles show idle, failures show attention, cancellations show idle, session end removes the dot, and parallel sessions keep distinct dots. Hook data is server-global, so Made MUST apply an event only to a tab whose process ancestry owns that session, and MUST NOT leak sessions, dots or explorer moves between tabs on the same server.

Remote explorer follow MUST behave identically to the local Grok contract: the newest attachment for this tab moves the explorer to that session's directory, a prompt in an older session keeps the explorer there instead of being stolen back, focusing an already-open session without a new attach or prompt MUST NOT move the explorer, and when no Grok session remains open for the tab and no OpenCode or Claude Code source applies the explorer MUST return to the remote shell-cwd fallback. Remote OpenCode keeps precedence: a connected remote OpenCode session, or its usable sessionless view directory under SSH-06, MUST keep the root and dots. A live owned remote Claude Code session directory, or its agent-view selection, under SSH-11 MUST also keep the root over Grok, and a usable Grok directory reported while Claude Code holds the root MUST be retained for when Claude Code exits. An open Grok session MUST win over the remote shell cwd only while OpenCode is not connected and the tab has no live owned remote Claude Code session. Every Grok-reported directory MUST be validated against the owning remote authority like any SSH-02 report and MUST NOT be canonicalized, opened or resolved against the host filesystem. Repeated A→B→C switches between Grok sessions, Claude Code sessions, OpenCode sessions and shell directories MUST land on the correct root each time.

Open/close lifecycle MUST be honest: a session appears when it first becomes owned, disappears when its process is gone or its session end is reported, and disconnect or exit clears that tab's stale Grok activity and dots before restoring the presentation above. On reconnect the client MUST discard previous-epoch Grok state and apply a fresh snapshot; events and snapshots from a previous attachment, stale revisions, or identities that do not match the owning tab MUST be discarded, and late or duplicated events MUST NOT double-apply. Corrupt, truncated or oversized ingestion MUST fail bounded and honest without breaking the terminal or filesystem.

Remote Grok activity has exactly two honest capability tiers. SSH-10 consent is Made's authorization for its own Grok-home write, running the official installer; it is not a precondition for reading hook state that an already-installed plugin supplies. When no grok-made equal to or newer than the shipped one is installed in the remote Grok home, which is the case when consent was never given or was refused so Made's install never ran, when the consented install failed, or when only an older copy is installed, Made MUST keep the inventory tier: owned session inventory, cwd follow, open/close lifecycle and running indication continue, while hook-driven activity/status dots and prompt-event timing are unavailable; sessions seen only in the inventory MUST show idle, and diagnostics MUST say visibly and honestly that session-status hooks are unavailable and why, without degrading the terminal or filesystem. Made runs the official `grok plugin install <staged-plugin> --trust` with the grok executable that detection found only after a separate explicit per-server/version consent and only after staging the shipped `plugins/grok-made` package into the private remote version directory and verifying its hashes under the SSH-06 staging rules; the staged package MUST contain the native session-status hook built for the server's own target, the same Linux glibc or macOS operating system and CPU architecture as the companion staged for that server, with the executable mode SSH-06 requires, so the installed hook runs there with no interpreter or runtime prerequisite. A pre-existing or user-self-installed grok-made equal to or newer than the shipped one MAY supply full hook activity without that consent because Made performs no Grok-home write in that case, and it MUST NOT be treated as permission to run the installer. That action MUST be bounded and idempotent per server and version: an installed grok-made version equal to or newer than the shipped one MUST skip the command, a missing or older install MUST run it once, and a newer installed copy MUST be left in place. Made MUST NOT run the installer silently, MUST NOT hand-edit the user's Grok `config.toml` or any other Grok-home file, MUST NOT redirect or set `GROK_HOME` for the remote user, and MUST NOT pass OpenCode's plugin environment to Grok. The installer persistently installs and enables the plugin in the remote user's existing Grok home so existing settings, sessions and authentication remain visible; because plugin hooks attach when Grok starts, they apply only to Grok sessions started after the install, and Made MUST disclose that limit. The presence or absence of python3 or any other interpreter on the server MUST NOT gate the hook tier, the consent offer or the install, and MUST NOT be reported as a failure reason; uninstall remains the user's own `grok plugin uninstall grok-made --confirm` and Made MUST NOT run it. A failed or refused install MUST still let the user start and use Grok with the inventory tier whenever no grok-made equal to or newer than the shipped one is otherwise installed, and a refused consent MUST always leave the terminal and inventory follow usable with no Made Grok-home write.

Revealed About diagnostics MUST show the same single Grok Build row treatment for a remote workspace, driven by that tab's remote detection, install and running state rather than the host's, using the realized Grok wording for shared states and a bounded honest reason for remote-only states such as inventory-only mode or a failed or absent install; the row MUST stay hidden until diagnostics are revealed and out of routine chrome, and copied diagnostics MUST include it.

#### Scenario: Remote Grok detection, direct launch and sessionless branding

- Given server A with grok on its remote PATH and no session open, and server B without grok
- When the user opens the remote launch choices and chooses `Grok SSH` on A
- Then only A offers the row, the launch starts A's grok executable with the single argument `dashboard` directly in a new tab bound to A and its selected remote directory with no shell first or separate picker, the current tab keeps running, no plugin configuration is injected, the tab is branded `Grok Build` with the official mark before any session exists, and a manual `grok` typed in A's remote shell is not given that argument

#### Scenario: Inventory-only follow without consent

- Given a remote tab that never granted the plugin consent, with unreadable-inventory, foreign-pid-row and malformed-row variants available
- When sessions start and end for this tab's process tree and the once-a-second inventory refresh runs
- Then owned sessions appear and move the explorer to their directories, foreign, stale and malformed rows change nothing, each owned session shows an idle dot, session end removes the dot and the explorer returns to the remote shell cwd, no plugin is installed or consent requested by the inventory, and the terminal and filesystem remain usable

#### Scenario: Consented install enables hook activity

- Given a supported server with grok present, no python3 or other interpreter installed or on its PATH, no grok-made installed, and an explicit per-server/version consent granted
- When Made stages the shipped `plugins/grok-made`, verifies hashes, runs the official `grok plugin install <staged-plugin> --trust` with the detected grok executable, and the user starts a new Grok session and submits a prompt
- Then the install runs exactly once for that version without editing `config.toml`, the new session moves the explorer without waiting for the inventory refresh, its dot shows working and then attention on a question, hooks apply only to sessions started after the install and that limit is disclosed, and the dots match the local Grok Build treatment

#### Scenario: Staged hook matches each server's target

- Given one Linux glibc aarch64 server and one macOS x86_64 server, both supported, with grok present and consent granted
- When Made stages the shipped package on each and runs the consented install
- Then each staged and installed grok-made carries the hook built for that server's operating system and architecture, verified by hash and executable mode under SSH-06, and a new Grok session on each server produces hook lines that drive working and attention dots

#### Scenario: Install idempotence, refusal and failure keep honest tiers

- Given one server whose installed grok-made equals the shipped version, one with a newer installed copy, one where consent is refused, and one where the install command fails
- When the consent flow is exercised on each
- Then equal and newer installs skip the command and stay in place, refusal performs no write, failed servers record an honest degraded reason, none of them hand-edits Grok configuration or redirects `GROK_HOME`, every variant still lets Grok start with inventory-tier follow, and no variant silently retries or claims hook readiness

#### Scenario: OpenCode precedence and Grok tab presentation

- Given a remote tab with a connected remote OpenCode session and two open Grok sessions, and a second remote tab whose OpenCode exited while two Grok sessions remain
- When Grok events and session changes arrive on both tabs
- Then the OpenCode tab keeps the OpenCode program label, icon, dots and root, the other tab shows the Grok Build program label, official mark and Grok dots, each Grok session's directory wins over the shell cwd only in its own tab, and no session, dot or root crosses between the tabs

#### Scenario: Two tabs on one server do not leak Grok sessions

- Given two remote tabs on one server, each with its own Grok session, and the server-global hook log receiving events for both
- When events for both sessions are appended and one tab disconnects
- Then each tab applies only events owned by its own process ancestry, including identical session identifiers disambiguated by ancestry, the disconnected tab clears its stale activity and presentation without touching the other tab, and the surviving tab keeps its own root, dots and terminal

#### Scenario: Disconnect, reconnect and stale Grok state

- Given a remote Grok tab following session B when the SSH transport drops, with hook events appended during the outage
- When the tab reconnects before lease expiry and receives a fresh snapshot
- Then the tab discards pre-disconnect activity and dots, discards events and revisions from the previous attachment, applies the fresh snapshot only after identity and revision checks pass, does not move the explorer for corrupt or truncated ingestion, and preserves terminal continuity under SSH-07

#### Scenario: Remote Grok diagnostics stay honest and hidden until revealed

- Given remote workspaces in the inventory-only, installing, ready, not-running and install-failed states
- When the user reveals About diagnostics and copies the report
- Then exactly one Grok Build row per workspace appears beside the OpenCode pill driven by that tab's remote state, inventory-only and failure states state the honest reason instead of claiming readiness, no state mentions python3, the row stays out of header/tab/explorer/terminal chrome, and the copied report includes the Grok and OpenCode sentences

### Requirement: Remote Claude Code launch and status (SSH-11)

Remote Claude Code support MUST mirror the local Claude Code contract of `specs/claude-code-session-follow.md` for these flows: `Claude Code presence`, `Claude Code launch`, `Program status protocol in the terminal`, the session-only plugin and its `Claude Code session events`, `Claude Code session ownership and liveness`, `Claude Code explorer follow and precedence` including `/cd` follow through `claude agents --json`, `Claude Code agent-view session follow`, `Claude Code tab presentation` including the hook-derived session title and status fallback, `Claude Code channel fallback`, the `Claude Code diagnostics row` and `Claude Code privacy and observation boundary`. In that mirror, "local tab" means the remote tab, the tab's terminal process is the remote managed terminal process that SSH-07's supervisor owns, process ancestry, liveness and the listing are evaluated on the owning server, and every directory is a remote path of that server; the deltas below take precedence over the local wording. Remote claude detection MUST be the same bounded metadata-only remote PATH lookup as SSH-01: it MUST ignore empty and relative PATH entries, MUST NOT spawn claude, MUST NOT check a version, MUST NOT install or load a plugin, and MUST NOT delay terminal or explorer startup.

Choosing `Claude Code SSH` MUST start the detected remote claude absolute path with no arguments directly as a new tab bound to that server and its selected remote directory, with no shell first and no separate folder picker, while the current tab keeps running; that launch MUST NOT receive OpenCode's plugin environment and MUST NOT add any command-line argument. An out-of-date companion that cannot start Claude Code MUST yield the existing update-required state from SSH-06, never a shell fallback. Typing `claude` in a supported remote shell MUST remain valid and MUST NOT be replaced by the menu.

The shipped Claude Code plugin package, with its native session-event hook built for the server's own target (the same Linux glibc or macOS operating system and CPU architecture as the companion staged for that server) and the executable mode SSH-06 requires, MUST be staged and verified as part of the SSH-06 companion artifact set in the private remote version directory, so that the hook runs on the server with no interpreter, language runtime or separately installed package. The staging of the Claude Code package MUST be authorized by the existing SSH-06 per-server/version companion staging consent and needs no separate Claude Code consent; a refused staging consent writes no app-owned remote file, exactly as SSH-06 defines. On the server, Made MUST add the staged plugin's directory to `CLAUDE_CODE_PLUGIN_DIRS` and bind that tab's private remote spool in the environment of every `Claude Code SSH` launch and of every supported remote shell tab, so that `claude` typed in a remote shell loads the plugin as locally typed `claude` does. The variable MUST be composed on the server from the remote user's own value with the local rules: every existing entry kept in its original order, Made's entry added once and not again when already present, and no user entry overwritten, reordered or dropped. The host's value of `CLAUDE_CODE_PLUGIN_DIRS`, the host plugin path and the host spool path MUST NOT be sent to the server. Directly launched remote OpenCode and Grok tabs MUST NOT receive the Claude Code entry or spool binding, and the Grok launch still injects no plugin configuration. When the staged package is missing or unreadable for a tab, including a tab attached to a supervisor started from artifacts that predate the package, Made MUST still start the shell and Claude Code, MUST NOT add an entry that points to it, MUST NOT restart or replace the running supervisor, and MUST record that Claude Code session-status hooks are unavailable for that tab. Made MUST NOT run `claude plugin install` or any `claude` subcommand on the server other than the `claude agents --json` listing below, and Claude Code started on the server outside a Made tab MUST be unaffected. Claude Code parity MUST NOT write anything on the server outside app-owned remote storage: Made MUST NOT create, edit, delete or read any file of the remote user's Claude Code configuration or state, including everything under `~/.claude/**`, and SSH-10's consented Grok install remains the single write outside app-owned storage.

The hook MUST append records to the tab's remote spool with exactly the fields, status derivation, title sanitization, 4 KiB record bound, 1 MiB spool bound and never-block, always-succeed behavior of `Claude Code session events`. Each remote spool MUST be in private app-owned remote storage, never in the workspace or in the remote user's Claude Code home, with directory mode 0700 and file mode 0600, created without following symlinks. The spool MUST be per-session runtime state owned by that tab's supervisor, removed by that supervisor when SSH-07 owned-session teardown ends the tab's managed terminal session and on detached-lease expiry (a Claude Code session ending does not remove it), and swept at helper start only when its owning supervisor is no longer running, leaving every live supervisor's spool intact; while the server is unreachable no removal MUST be claimed. The server side MUST apply only records appended after the tab's managed session started, MUST ignore malformed, truncated or oversized records without moving the explorer or changing presentation, and MUST NOT apply a spool owned by another tab, another Made client, or an earlier managed session.

Ownership and liveness MUST be determined on the server from live remote process ancestry: a session belongs to the tab only when the Claude Code process that ran its hook is that tab's managed terminal process or a descendant of it within 32 parent hops at ingestion; records from a process outside the tab, already exited at first ingestion or unresolvable MUST be ignored, which includes Claude Code started inside a tmux server or another process outside the tab's managed process tree that inherited the tab's environment. Owned processes MUST be checked for exit about once a second, each live owned process MUST have at most one current session, and separate processes MUST remain separate sessions. Hook data from different tabs on the same server, and from different Made clients attached to the same server user, MUST NOT cross tabs.

Only owned-session state MUST cross the SSH transport to the client: session identity, session directory, derived status, the sanitized session title, the recency needed to choose among sessions, and the listing-derived directory and agent-view selection results, scoped to the owning tab, connection epoch and root generation. Prompt text, the protocol `msg` field, notification message text, tool names, inputs and outputs, transcript paths or contents, environment values and any listing field other than `pid`, `cwd`, `kind` and `name` MUST NOT be stored, logged or relayed by the server side and MUST NOT be stored, logged, displayed or copied by the client. The client MUST reject relayed state for another tab, a previous epoch or a stale revision, MUST NOT double-apply late or duplicated state, and corrupt, truncated or oversized relay data MUST fail bounded without breaking the terminal or filesystem.

Remote explorer follow MUST apply the local hook-record and listing semantics, including trigger bursts, counted listings, at most one move per trigger from the owning tab, the about 5-second listing age limit and the agent-view selection by the tab's own sanitized and normalized remote terminal title. The listing MUST run on the owning server with the remote claude absolute path found by that server's detection and exactly the arguments `agents --json`, directly without a shell, with standard input closed, as the leader of its own process group, never in parallel with its previous run, killed with its process group after about 2 seconds or when every tab it serves, or the helper, is torn down, and only on the local trigger set observed for the remote tab: the tab starts owning a live session, the user submits input to that tab (Enter or Return, or pasted text containing a newline), or the tab's terminal title, as seen while a Claude Code protocol status is live in that tab, changes. The client MUST detect these triggers for its remote tab, checking input only for a submitted line, and MUST send the server a bounded trigger that carries no input bytes and no title text; one listing worker per server user MUST serve every remote tab of that client on that server, so runs for those tabs never run in parallel and one run serves them all. No listing MUST run while no trigger is pending, an idle live session MUST run none, and when claude was not detected on that server no listing runs and hook-driven follow continues. Any listing failure MUST change nothing. The published listing result kept in private app-owned remote storage MUST be removed when the last supervisor of that server user that can run the listing ends, and swept at helper start when no such supervisor is running. Every reported or listed directory MUST be validated against the owning remote authority like any SSH-02 report and MUST NOT be canonicalized, opened or resolved against the host filesystem; a relative, missing or unreadable directory MUST NOT move the explorer. Under the SSH-09 controlled profile, hook-driven moves and `/cd` follow after Enter MUST appear within the SSH-09 two-second budget instead of the local about 250 ms and about one second.

Remote explorer root precedence MUST be: a connected remote OpenCode session directory or its usable sessionless view directory under SSH-06, then the agent-view selection or the live owned remote Claude Code session directory, then open remote Grok sessions under SSH-10, then the latest validated remote shell cwd, then the remote launch directory. A Claude Code directory reported while remote OpenCode holds the root MUST be retained for when OpenCode releases it, a usable Grok directory reported while Claude Code holds the root MUST be retained for when Claude Code exits, and with more than one live owned session the most recently started, prompted or moved session wins. Claude Code presence known only from the status protocol, without a live owned session, MUST NOT move or hold the root. Remote tab presentation MUST be: while remote OpenCode is connected or its directly launched process is alive, OpenCode's label, icon and dots under SSH-06; otherwise, while Claude Code is present, the Claude Code program label, the Claude Code mark, one dot per live owned session or exactly one protocol dot, the local status precedence (protocol, then hook-derived, then idle), the local tooltip title rule (the tab's own remote terminal title while a protocol status is live, then the latest owned hook session title, then `Claude Code`) and accessible name, with Grok Build presentation yielding to it; otherwise SSH-10's Grok Build presentation; otherwise the remote shell label and terminal glyph. The tooltip MUST omit the workspace path, the server identity MUST remain inspectable under SSH-09, and the OS window title MUST stay `Made`. Repeated switches between OpenCode, Claude Code, Grok sessions and shell directories MUST land on the correct root each time.

Disconnect MUST clear that tab's remote Claude Code protocol status, dots, title and session-driven root before the SSH-07 stale presentation applies. Remote Claude Code and its hooks keep running under SSH-07, and the server side MUST keep that tab's owned-session state while its managed session lives. After reattach, the client MUST discard previous-epoch Claude Code state, MUST apply a fresh snapshot of the tab's live owned sessions, their current directories and hook-derived status only after identity and revision checks pass, MUST wait for a fresh emission rather than replay earlier protocol status, and MUST NOT replay individual records written during the outage. Lease expiry or supervisor loss MUST end the tab's sessions without a successful-reattach claim, under SSH-07.

Revealed About diagnostics MUST show the single Claude Code row for a remote workspace with the local states, pills and sentences of `Claude Code diagnostics row`, driven by that tab's remote detection, staged-package state, presence, live owned sessions and observed protocol status rather than the host's: `Checking for Claude Code.` and `Claude Code is not on PATH.` from remote detection, `Claude Code is not running.`, `Claude Code is starting.`, `Claude Code session-status hooks are unavailable.` when the staged package is unavailable for that tab or no live owned session arrived within the grace period, `Claude Code status protocol is unavailable.`, and the ready pill with `Claude Code session follow is ready.` when a live owned session exists and the status protocol is observed. The former remote sentence `Claude Code session-status hooks are unavailable over SSH.` is retired and MUST NOT be shown; a bounded remote-only reason, such as a tab on an older supervisor, MAY appear only in the sectioned diagnostics detail, not in the row sentence. No state mentions python3 or another interpreter, the row MUST stay hidden until diagnostics are revealed and out of routine chrome, and copied diagnostics MUST include it.

#### Scenario: Claude Code SSH row and direct launch load the staged plugin

- Given server A has claude on its remote PATH and a verified staged companion set including the Claude Code package, server B does not have claude, both supported and reachable, and A's remote user has no `CLAUDE_CODE_PLUGIN_DIRS`
- When the user opens each server's remote launch choices and chooses `Claude Code SSH` on A
- Then only A offers `Claude Code SSH`, A's detected claude starts with no arguments directly in a new tab bound to A and its selected remote directory with no shell first or separate picker while the current tab keeps running, that Claude Code loads the server-staged plugin for that session only with a spool bound to that tab, no host path or host variable value is sent, and B's choices omit Claude Code without affecting its terminal

#### Scenario: Shell-typed remote claude and existing plugin directories

- Given a supported remote shell tab on a server whose remote user's `CLAUDE_CODE_PLUGIN_DIRS` already holds two user entries
- When the user types `claude` in that remote shell
- Then Claude Code starts normally, the variable holds the two user entries in their original order plus Made's single server-staged entry, both the user's plugins and Made's plugin load, and no file of the remote user's Claude Code configuration changes

#### Scenario: Remote hook follow and hook-derived status without the protocol

- Given a remote Claude Code session with Made's staged plugin loaded and the status protocol unavailable, for example with `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1`
- When its session starts in another remote directory, the user submits a prompt, Claude Code requests a permission, the turn stops, and the user renames the session and submits a prompt
- Then the explorer moves to the session's remote directory within the SSH-09 budget, the single dot is idle, busy, attention and idle in turn, the tooltip begins with the renamed title, and the explorer, dots and title of no other tab change

#### Scenario: Remote `/cd` follow through the server listing

- Given a remote tab following a live owned Claude Code session on a server where claude was detected
- When the user types `/cd` to another readable remote directory, presses Enter and submits no prompt
- Then the listing runs on that server, the explorer switches to that remote directory within the SSH-09 budget, the next hook record carrying the same directory leaves it there, and an idle session with no trigger runs no listing

#### Scenario: Remote agent-view session follow

- Given a remote tab with a live owned session, a live protocol status, and a server listing with exactly one background entry named `Spec task` in a remote directory
- When the user opens that session from Claude Code's agent view and the remote terminal title becomes `✳ Spec task`, and later the title changes to `✳ Claude Code`
- Then the explorer follows the background entry's remote directory while the title is unchanged and returns to the owned session's directory after the title change, and duplicate names, no live protocol status or no live owned session select nothing

#### Scenario: Remote precedence matches local precedence

- Given a remote tab with open Grok sessions under SSH-10, and a second remote tab with remote OpenCode connected
- When the user starts Claude Code in each tab and later exits it in the first
- Then the first tab shows `Claude Code`, its mark and only Claude Code dots with the root on the Claude Code session directory while it is present, then returns to `Grok Build`, the Grok mark, Grok dots and the Grok session's directory, and the second tab keeps OpenCode's label, icon, dots and root throughout and moves to the Claude Code session directory if OpenCode disconnects while that session is live

#### Scenario: Foreign, tmux and other-tab processes are not claimed

- Given two remote tabs on one server, a Claude Code started inside a tmux server launched from the first tab's shell, and a Claude Code in a separate SSH login of the same server user with a copied spool binding
- When each Claude Code fires hook events
- Then each tab applies only sessions owned by its own managed process ancestry, the tmux and foreign sessions are claimed by no tab, and no label, dot, title or explorer root crosses between the tabs

#### Scenario: Remote disconnect and reconnect

- Given a remote tab following Claude Code session B with a busy dot when the SSH transport drops, while the remote Claude Code keeps running and its hooks report a prompt and a shell-command `cd` directory change during the outage
- When the tab reconnects before lease expiry
- Then the disconnect clears the Claude Code status, dots, title and session-driven root, no stale dot or previous-epoch state is applied after reconnect, the fresh snapshot after the completing resnapshot frame restores the session's current directory and hook-derived status, protocol status waits for a fresh emission, no outage record is replayed individually, and the same PTY session is preserved

#### Scenario: Out-of-date companion and older supervisor

- Given a server whose staged companion predates the Claude Code package, and separately a tab still attached to a supervisor of an older Made version (a same-version supervisor started from older artifacts is an accepted residual, relying on a version bump at every release; see the proposal decision index)
- When the user chooses `Claude Code SSH` on the first server, and types `claude` in the older tab's shell
- Then the first shows the existing SSH-06 update-required state without starting a shell or other program and writes nothing for Claude Code before an approved update, any approved update follows the SSH-06 staging rules, and the older tab keeps running unchanged with Claude Code started without a Made plugin entry and its diagnostics row reporting session-status hooks unavailable

#### Scenario: Staging consent declined

- Given a server with claude on its remote PATH and no staged companion
- When the user refuses the SSH-06 companion staging consent, which covers the Claude Code package
- Then no app-owned remote file is written, the attempt fails clearly and stays retryable as SSH-06 defines, and no Claude Code plugin, spool or hook exists on the server

#### Scenario: Remote hooks disabled keep protocol status only

- Given remote claude 2.1.295 or later started with `--bare` in a supported remote shell tab outside tmux
- When Claude Code sends the capability probe, emits `working`, `blocked` and `done`, sets its window title, and later exits
- Then Made answers the probe through that remote terminal, the tab shows `Claude Code`, the Claude Code mark and one dot that is busy, attention and idle in turn, the tooltip starts with the sanitized remote title, the explorer stays on the remote root chosen without Claude Code, the diagnostics row reports session-status hooks unavailable, no other tab changes, and after exit the tab returns to the remote shell label and terminal glyph

#### Scenario: Claude missing or undetected on the server

- Given server A without claude, and server B where the bounded detection did not find claude but the user's interactive remote shell PATH starts a claude
- When the user opens launch choices and works in remote shells on both
- Then neither offers `Claude Code SSH`, A's diagnostics row says `Claude Code is not on PATH.`, B's shell-typed Claude Code still loads the staged plugin and follows hook records, no listing runs on B, and no terminal or explorer behavior breaks on either

#### Scenario: Remote listing failures change nothing

- Given a remote tab following a live owned session, and a server listing that exits non-zero, times out, prints malformed output or has no entry for the session's pid
- When hook records with new remote directories arrive
- Then the explorer follows the hook records exactly as without the listing and no listing failure moves the explorer or changes presentation

#### Scenario: Remote directories are validated on the server

- Given a remote session whose hook records or listing report a relative path, a missing remote directory, an unreadable remote directory, and a path that exists only on the host
- When the reports are ingested
- Then none moves the explorer, nothing is opened or resolved on the host, and nothing in the report is executed

#### Scenario: Remote privacy markers never retained or relayed

- Given remote Claude Code sessions whose prompts, permission notifications, protocol `msg` fields and tool data contain unique marker strings
- When every remote spool, relay frame, server-side log, client log, diagnostics row and copied diagnostics report is searched
- Then no marker string is found, and the remote user's Claude Code configuration and state files are byte-for-byte unchanged and were not read by Made

#### Scenario: Staged hook matches each server's target

- Given one Linux glibc aarch64 server and one macOS x86_64 server, both supported, with no python3 or other interpreter on PATH
- When Made stages the companion set on each and a remote Claude Code session starts there
- Then each staged Claude Code package carries the hook built for that server's operating system and architecture, verified by hash and executable mode under SSH-06, and the hook writes owned records that move the explorer and drive the dot

#### Scenario: Remote Claude Code diagnostics

- Given remote workspaces where claude is undetected, not running, started less than about 10 seconds ago with no live owned session, running with `--bare` for longer than the grace period, running with a live owned session and no session-level emission for longer than the grace period, and running with a live owned session and observed emissions
- When the user reveals About diagnostics and copies the report
- Then each row is driven by that tab's remote state with `Claude Code is not on PATH.`, `Claude Code is not running.`, `Claude Code is starting.`, `Claude Code session-status hooks are unavailable.`, `Claude Code status protocol is unavailable.` and the ready pill with `Claude Code session follow is ready.` respectively, no row mentions an interpreter, the rows stay out of routine chrome, and the copied report includes them

#### Scenario: Remote Claude Code login opens only after confirmation

- Given a `Claude Code SSH` tab, or claude typed in a remote shell, that starts a login and runs `$BROWSER` with its sign-in URL
- When the request reaches the attached client
- Then the tab shows the URL card `from <server>` with the host emphasized, the host browser opens that URL only when the user chooses Open, Copy places the URL on the host clipboard, and when the client is detached the helper fails so claude prints the URL in the terminal instead

### Requirement: Declared precedence (AIA-05)
Each integration MUST declare a precedence rank, and ranks MUST be unique within a list. The core MUST derive local explorer-root precedence and local tab-presentation precedence from these ranks, never from catalog order or a hard-coded chain. The declared ranks MUST reproduce today's order: root follow OpenCode > Claude Code > Grok > shell, and local tab presentation OpenCode active > Claude Code present > Grok open > folder. When a holder releases, the root MUST return to the highest-ranked remaining holder with a usable directory, else the shell. Remote explorer-root and remote tab-presentation precedence MUST follow the same order, as SSH-06, SSH-10 and SSH-11 of `specs/ssh-remote-workspaces.md` define it, with the remote shell cwd and remote launch directory in place of the shell directory and the remote shell label and terminal glyph in place of the local shell label; the delimited remote paths of AIA-02 MAY implement it directly. Root-source identifiers MUST stay `shell`, `opencode`, `claude-code` and `grok`.

#### Scenario: Precedence by rank
- Given a local tab where OpenCode, Claude Code and Grok each hold a usable directory
- When holders release one at a time in the orders OpenCode, Claude Code, Grok and OpenCode, Grok, Claude Code
- Then the effective root source is always the highest-ranked remaining holder, then `shell`, with the same root, generation and switch results as before the change

#### Scenario: Rank is independent of catalog order
- Given the catalog order OpenCode, Grok Build, Claude Code
- When Claude Code and Grok both hold a directory and OpenCode holds none
- Then Claude Code owns the root and the tab presentation, although Grok Build is earlier in the menu

#### Scenario: Duplicate rank rejected
- Given an injected list in which two integrations declare the same rank
- When the list is validated by test
- Then the validation fails

#### Scenario: Remote order equals local order
- Given a remote tab where remote OpenCode, Claude Code and Grok each hold a usable remote directory
- When holders release one at a time in the orders OpenCode, Claude Code, Grok and OpenCode, Grok, Claude Code
- Then the remote root and tab presentation always belong to the highest-ranked remaining holder, then the remote shell cwd and remote shell label, matching the local order

### Requirement: Full-Left Preview Presentation

Made MUST present an opened file as a read-only preview occupying the entire left content pane in place of the terminal visually, never underneath the explorer or in a split with the terminal. No preview pane or placeholder MUST occupy space before a file is opened. The preview MUST include a compact header with filename, full-path tooltip, an understated read-only label, Copy path, Refresh and Close. The header MUST NOT show an age or freshness counter (such as `updated 5s ago`) and MUST NOT offer a separate Terminal or return-to-terminal action; closing the preview, by its Close control or the existing close keyboard shortcut, MUST return to and restore the same live terminal and its focus, and MUST clear the preview. The terminal surface MUST NOT offer a `File` control or any other control that re-shows a previously loaded preview; reopening a preview is done by selecting the file in the Explorer again. Preview content MUST use the documented gutter, code typography, horizontal scrolling, and safe loading/binary/size/encoding/unavailable states. Syntax highlighting SHOULD be applied where available and MUST fall back to plain text. Read-only safety, process continuity, input isolation, refresh, and root clearing are owned by the core spec.

#### Scenario: Text file preview

- Given a text file inside the active root
- When the user selects it in the explorer
- Then Made shows its content read-only across the entire left content pane, the full-height right explorer remains intact, and no editing or saving controls exist

#### Scenario: Preview terminal roundtrip

- Given a running terminal command in the selected workspace
- When a file is opened and the user closes the preview with its Close control, and separately with the close keyboard shortcut
- Then preview used the entire left content area and the left surface returns to the live terminal with terminal focus

#### Scenario: Syntax highlighting

- Given a supported text file is open in the preview
- When the file is rendered
- Then syntax highlighting is applied where available and plain text is shown when highlighting is unavailable

#### Scenario: Preview header has no age counter or terminal action

- Given a text file open in the preview, left open while the file changes on disk and is refreshed
- When the preview header is inspected by pointer and keyboard over time
- Then the header shows the filename with its full-path tooltip, the read-only label, Copy path, Refresh and Close only; no `updated … ago` or other age counter appears at any time; no Terminal or return-to-terminal action exists; Close returns to the same live terminal with terminal focus and clears the preview; the terminal shows no `File` control to re-show it; and selecting the file in the Explorer again reopens the preview

## REMOVED Requirements

### Requirement: Explicit Folder Picker
**Rationale:** The folder picker is replaced by the Open workspace screen. Its card, Close and Escape guarantees move to `Open Workspace Screen` (its Back action is dropped as redundant with Close, user decision); its tree, selection, keyboard, refresh, re-root, 1000-entry cap, double-click and unreadable-child guarantees move unchanged to `Open Workspace Folder Navigation`, except that rows become 24px high instead of 32px, matching the main Explorer tree, and the filled disclosure triangle becomes a thin stroked chevron (both user decisions); the path chip becomes the editable address bar; and the `Use current folder` and `Open selected folder` actions are replaced by the launch bar's `Terminal` button (and Cmd/Ctrl+Enter), which opens the same launch target with the same confirmation semantics.
