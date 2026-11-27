# Change Spec: open-workspace-redesign

This delta adds eight requirements to `specs/made-ui-ux.md` (`Open Workspace Screen`, `Open Workspace Host Bar`, `Open Workspace Folder Navigation`, `Open Workspace Launch Bar`, `Open Workspace Git Indicator and Worktrees`, `Open Workspace Change Pills`, `Hardened read-only git reads`, `Program-neutral wording`) and one requirement to `specs/ssh-remote-workspaces.md` (`Remote Open workspace browsing and Git parity (SSH-12)`). It modifies `specs/made-ui-ux.md` (requirements `Calm Editor Visual System`, `Minimal Workspace Layout`, `Grouped changes`, `Optional Session Map Presentation` and `Diagnostics Presentation`), `specs/opencode-session-follow.md` (requirement `Degraded Integration`), `specs/grok-session-follow.md` (requirements `Catalog-driven new-tab menu` and `OpenCode diagnostics wording`), `specs/claude-code-session-follow.md` (requirement `Claude Code launch`) and `specs/ssh-remote-workspaces.md` (requirements `SSH server entry and launch choices (SSH-01)` and `Local preservation and truthful parity evidence (SSH-09)`). It removes `Explicit Folder Picker` from `specs/made-ui-ux.md`; its tree, keyboard, refresh and close guarantees are carried into `Open Workspace Screen` and `Open Workspace Folder Navigation`. Each MODIFIED requirement below replaces the realized requirement of the same name in full, scenarios included; text not named in the proposal's change list is verbatim. SSH-04, SSH-06, SSH-07, SSH-08, SSH-10 and SSH-11 are not modified: their guarantees apply unchanged, and the terminology rule of `Open Workspace Screen` and of SSH-01 maps their references to rows, the `+` menu and remote launch choices onto the new screen.

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

Made MUST use the implemented Calm Editor visual system for the native shell: dark layered surfaces, one calm mint-green accent, bundled Ubuntu-Light proportional and Hack monospace UI typography with bundled DejaVu Sans Mono and a bundled Iosevka symbols fallback for the terminal grid, a 4/8-point spacing rhythm, 8–10px card radii, 4px window gutters, 1px borders, and opaque preview, diagnostics, tooltip, toast, and other non-terminal reading surfaces, with the Open workspace card as the documented conditional-glass exception. The accent MUST be `ACCENT #8BD5A7`, with `ACCENT_HOVER #A3E3BA`, `ACCENT_PRESSED #72BF8E`, `ACCENT_TEXT #08140D`, `ACCENT_TINT #26352C`, `FOCUS #B8F0C9`; it applies to the wordmark `M`, primary accent actions, focus rings, busy spinners, and the explorer splitter while dragging, but MUST NOT be used as a workspace-tab outline. The terminal cursor MUST use the neutral terminal foreground `#D7DDE7` rather than the accent; when the pane is focused and the terminal reports a blinking cursor style it MUST blink with a ~530 ms on/off phase, while a steady reported style or an unfocused pane MUST keep the cursor steady and visible. The no-native/fallback clear color MUST use `BG_CLEAR` alpha 215/255, while an actually applied native backdrop keeps `BG_CLEAR_NATIVE` alpha 135/255. Workspace tabs use opaque `BG_TAB_SELECTED #1B231F` and `BG_TAB_HOVER #151C18` fills over the translucent black `BG_TAB_IDLE`; only the selected tab paints a 1px inside stroke, including when that selected tab is hovered: the dark tab hue darkened by 12 at alpha 128, the same strength as the previous lighter step but not a light edge, and not `BORDER_CONTROL` or another contrasting gray. Inactive tabs paint no stroke at rest and no stroke on hover. That stroke is not `ACCENT`, `FOCUS`, or a selection cue. Card borders remain neutral and the tab close hover remains `ERROR`. The full-height explorer card/header MUST use the documented conditional glass: when a native backdrop was actually applied (macOS HudWindow vibrancy or Windows 11 Acrylic), they use `BG_CARD_GLASS_NATIVE` 24/255 and `BG_HEADER_GLASS_NATIVE` 32/255 so the native material reads through; when no native backdrop is applied (disabled, unsupported, or failed), they use the darker fallback `BG_CARD_GLASS` 188/255 and `BG_HEADER_GLASS` 194/255 for legibility without blur. Windows MUST use the same applied/fallback selection. The painted soft shadow and 1px top highlight remain. The terminal card fill MUST use `BG_TERMINAL_GLASS` alpha 160/255 over the `BG_TERMINAL` RGB (11/13/16) only while the terminal is on its primary screen (no alternate-screen TUI), and it MUST be the sole glass layer: the terminal body and terminal-owned path-line/file-slot chrome MUST NOT paint another translucent fill. When the terminal is in alternate-screen mode (the VT `ALT_SCREEN` signal used by OpenCode, vim, and less), the terminal card fill, terminal-owned path-line/file-slot chrome, and body MUST use fully opaque `BG_TERMINAL` 255/255 so TUI cells remain on an opaque surface. The optional session strip/header remains opaque `BG_HEADER`. Preview, diagnostics, tooltip, and toast remain opaque. The Open workspace card MUST use the same conditional glass as the explorer card: `BG_CARD_GLASS_NATIVE` 24/255 when a native backdrop was actually applied and the fallback `BG_CARD_GLASS` 188/255 otherwise, with the painted soft shadow and 1px top highlight; its text fields, folder and worktree rows, pills, notices and buttons keep their own opaque fills and the reading-surface contrast of `Reading-surface contrast`. The workspace header MUST float directly over the window clear or native backdrop with no persistent band fill, border, shadow, or top highlight; the wordmark, workspace tabs, and icon actions are painted directly on that backdrop. A native macOS/Windows backdrop MAY layer behind the window only through the platform boundary; when unavailable, disabled, or unsupported, the in-app explorer glass MUST remain and the reason MUST be honest. Except for the documented primary-screen terminal glass and the Open workspace card glass, the UI MUST NOT introduce gradients or glows, multiple accents, accent-colored status, opacity on reading surfaces, or text below 10px. The no-webview runtime constraint is owned by the core Responsiveness and Lightness requirement.

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
- Then while the primary screen is active the terminal card fill uses `BG_TERMINAL_GLASS` alpha 160/255 over the `BG_TERMINAL` RGB (11/13/16) and is the only glass layer: the terminal body and terminal-owned path-line/file-slot chrome paint no second translucent fill; while the terminal reports the VT `ALT_SCREEN` mode, the terminal card fill, path-line/file-slot chrome, and body use fully opaque `BG_TERMINAL` 255/255; the optional session strip/header remains opaque `BG_HEADER`; the surface follows that mode signal rather than title/content heuristics and returns to the single glass layer when the alternate screen exits; and preview, diagnostics, tooltip, and toast remain opaque while the Open workspace card follows its conditional glass

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

## REMOVED Requirements

### Requirement: Explicit Folder Picker
**Rationale:** The folder picker is replaced by the Open workspace screen. Its card, Close and Escape guarantees move to `Open Workspace Screen` (its Back action is dropped as redundant with Close, user decision); its tree, selection, keyboard, refresh, re-root, 1000-entry cap, double-click and unreadable-child guarantees move unchanged to `Open Workspace Folder Navigation`, except that rows become 24px high instead of 32px, matching the main Explorer tree, and the filled disclosure triangle becomes a thin stroked chevron (both user decisions); the path chip becomes the editable address bar; and the `Use current folder` and `Open selected folder` actions are replaced by the launch bar's `Terminal` button (and Cmd/Ctrl+Enter), which opens the same launch target with the same confirmation semantics.
