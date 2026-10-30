# Change Spec: explorer-git-view

## ADDED Requirements

### Requirement: Source Control view toggle

When the explorer root is a known git worktree, the explorer header MUST show an icon-only control that switches the explorer body between the file tree and the git view. The control MUST be absent when the folder is not a known git worktree. Its tooltip and accessible name MUST be "Source Control" while the file tree is showing and "Explorer" while the git view is showing. While the git view is showing, the icon MUST use the existing selected icon treatment. While the file tree is showing, the header title MUST be `EXPLORER` and the header MUST show the root name. While the git view is showing, the header title MUST be `REPOS` in the same section-label treatment as `EXPLORER`, the header MUST show the head line in place of the root name, and the root name MUST NOT be shown. Activating the icon MUST NOT hide the explorer, MUST NOT change the left surface, and MUST NOT replace the header Explorer toggle, which remains the hide and restore control. The icon MUST remain an icon-only action with a visible focus state and MUST NOT be a text button.

#### Scenario: Known worktree shows the icon

- Given the explorer root is a known git worktree and the file tree is showing
- When the explorer header is inspected
- Then an icon-only control is present, its tooltip and accessible name are "Source Control", the header title is `EXPLORER`, and no text action button is shown

#### Scenario: Toggle switches only the explorer body

- Given the icon is visible and the file tree is showing
- When the user activates the icon, then activates it again
- Then the body becomes the git view, the header title becomes `REPOS` with the head line in place of the root name, and the icon name becomes "Explorer" with the selected treatment; activating it again returns the body to the file tree, restores the `EXPLORER` title and the root name, and returns the icon name to "Source Control"; the explorer stays visible, the left surface is unchanged, the git-view body does not repeat the head line, and the header Explorer toggle still hides and restores the explorer

#### Scenario: Unknown folder has no icon

- Given the explorer root is not a known git worktree
- When the explorer is shown
- Then the icon is absent, the body is the file tree, and the header title remains `EXPLORER`

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
- Then the git-view header title is `REPOS` in the same section-label treatment as `EXPLORER`, the header shows the branch head line and does not show the root name, the git-view body does not repeat the head line, the file tree restores the `EXPLORER` title and the root name, and the icon tooltip and accessible name are "Explorer" while the git view is showing and "Source Control" after the return

### Requirement: Grouped changes

The git view MUST present changes in the sections "Merge Changes", "Staged Changes", "Changes", and "Untracked", in that order. A section with no rows MUST be omitted. The view MUST NOT present the changes as one flat list and MUST NOT show a commit box. A repository with no listed changes MUST show "No changes". Ignored paths MUST NOT be listed and MUST NOT mark a folder or a file row. A path that is both staged and unstaged MUST appear in both "Staged Changes" and "Changes", each with that side's status dot. A conflicted path MUST appear in "Merge Changes" with a Conflict dot and MUST NOT also appear as an ordinary staged or unstaged row. Each row MUST show its path relative to the explorer root. The git view and the file-tree change dots MUST include only changes under the explorer root. A change outside that root MUST NOT be listed or marked.

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
- Then the view shows "No changes" and shows no change section

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

A failed, missing, or unfinished status read MUST NOT block, replace, or disable the file tree. The file tree MUST remain usable, and the user MUST be able to return to it. When the git view is showing and no status snapshot is available yet, the view MUST show "Reading changes…". When the git view is showing and no status snapshot is available because the read failed, the view MUST show "Git status unavailable". Those messages MUST appear in the git view, not as a blocker over the file tree. When the folder is not a known git worktree, the icon MUST be absent and the file tree MUST be shown.

#### Scenario: Pending read leaves the file tree available

- Given a known git worktree whose status snapshot is not yet available
- When the git view is showing
- Then the git view shows "Reading changes…", and activating the icon returns to a usable file tree

#### Scenario: Failed read leaves the file tree available

- Given a known git worktree whose status read failed and no status snapshot is available
- When the git view is showing, and the user then returns to the file tree
- Then the git view shows "Git status unavailable", and the file tree is usable after the return

#### Scenario: Missing git does not block the file tree

- Given the status cannot be read because the folder is not a known git worktree
- When the explorer is shown
- Then the icon is absent, the file tree is shown, and no status failure blocks the tree

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

The choice between the file tree and the git view MUST be remembered separately for each open workspace. Switching to another workspace and back, or hiding and restoring the explorer, MUST keep that workspace's choice. The choice MUST NOT be restored after that workspace is closed or after the application restarts. If the explorer root stops being a known git worktree, the body MUST return to the file tree and the icon MUST disappear.

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
- Then the icon is absent and the body is the file tree
