# Tasks: explorer-git-view

Readiness: `analyze` must pass before implementation; review runs `verify` against the persisted spec.

This file records work that shipped before plan and analyze. The checked tasks and checkpoints are that record only. They do not mean that analyze or verify passed. The shipped header shows a branch or an 8-character commit id, not the words "Detached HEAD". A linked worktree shows a small monochrome icon instead of " · ". A non-linked checkout shows no worktree icon. The checked section 8 and 9 lines record the rejected matching-name layouts: omitting the folder name, then painting the icon before the head text. Section 10 records the matching-name header. Checked letter-color lines record the painted letters. The unchecked follow-up replaces those letters with the file-tree icon and a right-side status dot, and anchors the status-dot tooltip at the dot. That follow-up does not mean that analyze or verify passed.

## 1. Closed read-only status

- [x] 1.1 Group closed read-only status into Merge Changes, Staged Changes, Changes, and Untracked, assign U/A/M/D/C/R, omit ignored paths and empty sections, mark folders, and keep only changes under the explorer root (path: crates/made-app/src/worktree.rs)
- [x] 1.2 Lock that shipped grouping, ignored omission, subdirectory scope, conflict section, and read-only command set (path: crates/made-app/tests/worktree.rs)

## Checkpoint

- [x] verify Non-empty sections stay in order
- [x] verify Empty sections are omitted
- [x] verify Partially staged file appears twice
- [x] verify Ignored path is omitted
- [x] verify Changes outside the explorer root are omitted
- [x] verify Type change and copy use the existing letters

## 2. Explorer source-control surface

- [x] 2.1 Add the Source Control glyph used by the header icon-only toggle (path: crates/made-app/src/theme.rs)
- [x] 2.2 Paint the header toggle, keep the EXPLORER title in the file tree, reserve the root name short of the icon, show the branch or an 8-character commit id rather than the words "Detached HEAD", render grouped rows and letter colors, paint the folder dot without file-tree letters, open the existing preview for a file only, and keep the view choice in memory on the pane (path: crates/made-app/src/explorer_view.rs)

## Checkpoint

- [x] verify Known worktree shows the icon
- [x] verify Toggle switches only the explorer body
- [x] verify Attached branch without a linked label
- [x] verify Detached HEAD shows a short commit id
- [x] verify Distinct linked-worktree label
- [x] shipped repeated linked label showed the branch once and omitted the folder name
- [x] shipped detached linked worktree showed the words "Detached HEAD"
- [x] verify Clean repository
- [x] verify Letter and color identify the change
- [x] verify File tree has no status letters
- [x] verify Single-kind folder dot
- [x] verify Mixed-kind folder dot
- [x] verify Folder without changes has no dot
- [x] verify File activation opens the existing preview
- [x] verify Directory activation does not open a preview
- [x] verify No git write is offered
- [x] verify Banned explorer actions stay absent
- [x] verify Restart does not restore the choice

## 3. Per-workspace status chrome

- [x] 3.1 Show the icon only for a known worktree, push status chrome for that workspace, and keep a pending, failed, or missing read from blocking the file tree (path: crates/made-app/src/workbench.rs)

## Checkpoint

- [x] verify Unknown folder has no icon
- [x] verify Pending read leaves the file tree available
- [x] verify Failed read leaves the file tree available
- [x] verify Missing git does not block the file tree
- [x] verify Each workspace keeps its own choice
- [x] verify Hide and restore keeps the choice
- [x] verify Leaving a worktree returns to the file tree

## 5. Git-view header

- [x] 5.1 Show `REPOS` in the git-view header in the same section-label treatment as `EXPLORER`, with the existing head line in place of the root name, and do not repeat that line in the git-view body; keep the file-tree title `EXPLORER` plus the root name, and keep the icon tooltip and accessible name "Source Control" in the file tree and "Explorer" in the git view (path: crates/made-app/src/explorer_view.rs)

## Checkpoint

- [x] verify Git view header shows REPOS and the head line

## 4. File-row change dot

- [x] 4.1 Paint one right-side dot and kind tooltip on a changed file that is already a tree row, using the folder dot's color and tooltip rules and no status letter, without adding a row for a deleted path that is not in the tree (path: crates/made-app/src/explorer_view.rs)

## Checkpoint

- [x] verify Changed file row shows one dot
- [x] verify Deleted path not in the tree has no row

## 6. Detached commit id

- [x] 6.1 Show an 8-character commit id in the git-view header when HEAD is detached, instead of the words "Detached HEAD"; show that commit's subject on hover, including when the line is truncated, or the id when the subject is empty; do not put " · " before the linked-worktree name; leave an attached branch name unchanged (path: crates/made-app/src/explorer_view.rs)

## Checkpoint

- [x] verify Detached HEAD shows a short commit id
- [x] verify Empty commit subject still shows the id
- [x] verify Distinct linked-worktree label
- [x] shipped repeated linked label showed the branch once and omitted the folder name
- [x] verify Detached linked worktree
- [x] shipped repeated linked label showed the commit id once and omitted the folder name
- [x] verify Attached branch without a linked label

## 7. Linked-worktree icon

- [x] 7.1 Place a small monochrome icon immediately before a shown linked-worktree name in the REPOS header, remove the " · " before that name, and on hover of the icon or the name say it is a linked worktree and include the name, without replacing the commit-subject hover on the commit id; show no worktree icon and no worktree name on a non-linked checkout; leave the attached branch name and the detached 8-character commit id otherwise unchanged (path: crates/made-app/src/explorer_view.rs)

## Checkpoint

- [x] verify Linked worktree name has an icon
- [x] verify Distinct linked-worktree label
- [x] verify Detached linked worktree
- [x] verify Attached branch without a linked label
- [x] shipped repeated linked label showed the branch once and omitted the folder name
- [x] shipped repeated linked label showed the commit id once and omitted the folder name

## 8. Linked icon when the name repeats

- [x] 8.1 Keep the small monochrome linked-worktree icon when the folder name equals the visible branch or the 8-character commit id, omit that duplicate name, and say it is a linked worktree on hover of the icon, including the folder name; show no worktree icon and no worktree name on a non-linked checkout (path: crates/made-app/src/explorer_view.rs)

## Checkpoint

- [x] shipped repeated name showed the branch or id once, kept the icon, and showed no icon on a non-linked checkout

## 9. Icon before a repeated head text

- [x] 9.1 Paint the linked-worktree icon before the branch or 8-character commit id when the folder name equals that text and is omitted, and keep head text, then icon, then folder name when the name is distinct (path: crates/made-app/src/explorer_view.rs)

## Checkpoint

- [x] shipped repeated name painted the icon before the head text

## 10. Matching folder name replaces the head

- [x] 10.1 When a linked-worktree folder name equals the branch or 8-character commit id, show the icon and that folder name only, not the branch or id and not an icon-before-head layout; on hover of the icon or the name, say it is a linked worktree and include the folder name; keep head text, then icon, then folder name when the name differs (path: crates/made-app/src/explorer_view.rs)

## Checkpoint

- [x] verify Repeated linked label is not duplicated
- [x] verify Repeated linked label matches the visible id
- [x] verify Repeated worktree name still shows the icon
- [x] verify Matching folder name replaces the head text

## 11. Git-view icon and status dot

- [x] 11.1 Show the file-tree file or folder icon immediately before each git-view path, replace the painted status letter with one right-side kind dot, and anchor that dot's tooltip at the dot in both the git view and the file tree (path: crates/made-app/src/explorer_view.rs)

## Checkpoint

- [x] verify Git-view row shows an icon and a status dot
- [x] verify Kind tooltip replaces the painted letter
- [x] verify Status-dot tooltip is anchored at the dot
