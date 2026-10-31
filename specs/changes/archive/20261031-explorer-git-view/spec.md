# Change Spec: explorer-git-view

## ADDED Requirements

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
