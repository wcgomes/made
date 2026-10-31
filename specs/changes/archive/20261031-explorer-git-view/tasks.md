# Tasks: explorer-git-view

Readiness: `analyze` must pass before implementation; review runs `verify` against the persisted spec.

This file records work that shipped before plan and analyze. The checked tasks and checkpoints are that record only. They do not mean that analyze or verify passed. The shipped header shows a branch or an 8-character commit id, not the words "Detached HEAD". A linked worktree shows a small monochrome icon instead of " · ". A non-linked checkout shows no worktree icon. The checked section 8 and 9 lines record the rejected matching-name layouts: omitting the folder name, then painting the icon before the head text. Section 10 records the matching-name header. Checked letter-color lines record the painted letters. Section 11 records the replacement of those letters with the file-tree icon and a right-side status dot, and anchors the status-dot tooltip at the dot. That checked section does not mean that analyze or verify passed. Section 12 records the ring colors. Section 13 records the shipped hover-only fill and is not the current fill contract. Section 15 records the current fill rule. That checked section does not mean that analyze or verify passed. Sections 2, 3, and 5 record the rejected single header icon. Naming that icon Explorer while the git view was open is rejected. Those checked lines are not the current icon contract. Section 14 records the shipped two-icon header. Its folders glyph is not the current icon contract. Section 16 records a shipped bare total and is not the current count contract. Section 17 records the colon-and-hyphen format and is not the current count contract. Section 18 records the parenthetical count format. The unchecked files-mark follow-up does not mean that analyze or verify passed.

## 1. Closed read-only status

- [x] 1.1 Group closed read-only status into Merge Changes, Staged Changes, Changes, and Untracked, assign U/A/M/D/C/R, omit ignored paths and empty sections, mark folders, and keep only changes under the explorer root (path: crates/made-app/src/worktree.rs)
- [x] 1.2 Lock that shipped grouping, ignored omission, subdirectory scope, conflict section, and read-only command set (path: crates/made-app/tests/worktree.rs)

## Checkpoint

- [x] verify Non-empty sections stay in order
- [x] verify Empty sections are omitted
- [x] verify Partially staged file appears twice
- [x] verify Ignored path is omitted
- [x] verify Changes outside the explorer root are omitted
- [x] shipped type change and copy used the painted letters

## 2. Explorer source-control surface

- [x] 2.1 Add the Source Control glyph used by the header icon-only toggle (path: crates/made-app/src/theme.rs)
- [x] 2.2 Paint the header toggle, keep the EXPLORER title in the file tree, reserve the root name short of the icon, show the branch or an 8-character commit id rather than the words "Detached HEAD", render grouped rows and letter colors, paint the folder dot without file-tree letters, open the existing preview for a file only, and keep the view choice in memory on the pane (path: crates/made-app/src/explorer_view.rs)

## Checkpoint

- [x] shipped known worktree showed one header icon
- [x] shipped header toggle switched the explorer body; naming that icon Explorer in the git view is rejected
- [x] verify Attached branch without a linked label
- [x] verify Detached HEAD shows a short commit id
- [x] verify Distinct linked-worktree label
- [x] shipped repeated linked label showed the branch once and omitted the folder name
- [x] shipped detached linked worktree showed the words "Detached HEAD"
- [x] shipped clean repository showed "No changes" and no change section
- [x] shipped letter and color identified the change
- [x] shipped file tree had no status letters
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

- [x] shipped unknown folder showed no header icon
- [x] verify Pending read leaves the file tree available
- [x] verify Failed read leaves the file tree available
- [x] shipped missing git left the file tree usable and showed no header icon
- [x] verify Each workspace keeps its own choice
- [x] verify Hide and restore keeps the choice
- [x] shipped leaving a worktree returned the body to the file tree and removed the header icon

## 5. Git-view header

- [x] 5.1 Show `REPOS` in the git-view header in the same section-label treatment as `EXPLORER`, with the existing head line in place of the root name, and do not repeat that line in the git-view body; keep the file-tree title `EXPLORER` plus the root name (path: crates/made-app/src/explorer_view.rs)

## Checkpoint

- [x] verify Git view header shows REPOS and the head line
- [x] shipped single header icon was named Explorer while the git view was open; that naming is rejected

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

## 12. Source-control ring colors

- [x] 12.1 Color the source-control toggle's three rings independently from open changes: the highest ring `SUCCESS` only while a new change is open, where a new change is an untracked path or a staged add, the vertically middle ring `WARNING` only while a modified change is open, and the lowest ring `ERROR` only while a deleted change is open; leave a ring in the original color when its type is not open, and leave all three rings original for a clean repository; keep the stem and curve in the original color; do not light a ring for a rename; light only the lowest ring in `ERROR` for a conflict; keep those ring colors when the icon is selected or at rest so the selected treatment does not replace them; do not recolor the linked-worktree icon (path: crates/made-app/src/theme.rs)

## Checkpoint

- [x] verify Source-control rings follow open changes
- [x] verify Rename and conflict use the existing rings
- [x] verify Linked-worktree icon stays uncolored
- [x] verify Selected treatment keeps the ring colors

## 13. Colored ring fill on hover

- [x] 13.1 Shipped a hover-only center fill for a source-control ring that uses `SUCCESS`, `WARNING`, or `ERROR`, left an original-color ring hollow, and left the stem and curve unfilled; that record is not the current fill contract (path: crates/made-app/src/theme.rs)

## Checkpoint

- [x] shipped hover-only ring fill; not verification of the current fill scenarios

## 15. Selected fill keeps colored rings filled

- [x] 15.1 Keep a source-control ring that uses `SUCCESS`, `WARNING`, or `ERROR` filled while the icon is hovered or has the selected fill, leave an original-color ring hollow, return every ring to hollow only when the icon is neither hovered nor selected, and leave the stem and curve unfilled (path: crates/made-app/src/theme.rs)

## Checkpoint

- [x] verify Hover fills a colored ring
- [x] verify Selected fill keeps a colored ring filled

## 14. Two header icons

- [x] 14.1 For a known git worktree, show two stable-named icons: an icon named "Explorer" immediately left of the source-control icon named "Source Control", which stays rightmost; do not swap those names; give only the active view's icon the existing selected fill; do not switch when the active icon is activated; switch only the explorer body when the other icon is activated; clip the head line and the root name before both; keep ring colors only on the source-control icon in both views and do not recolor the Explorer icon; show neither icon when the folder is not a known git worktree. The shipped glyph was a folder; that glyph is not the current icon contract (path: crates/made-app/src/explorer_view.rs)

## Checkpoint

- [x] verify Known worktree shows both icons
- [x] verify The other icon switches only the explorer body
- [x] verify Unknown folder has neither icon
- [x] verify Ring colors stay on the source-control icon
- [x] verify Missing git does not block the file tree
- [x] verify Leaving a worktree returns to the file tree

## 16. Section and tooltip counts

- [x] 16.1 Shipped a row count on the same line as each visible REPOS section title, omitted empty sections without a zero, and added a bare total to the source-control tooltip when changes were open, leaving that tooltip only "Source Control" when none were open; that record is not the current count contract (path: crates/made-app/src/explorer_view.rs)

## Checkpoint

- [x] shipped section counts used a bare total; not verification of the current count scenarios

## 17. Section count format

- [x] 17.1 Shipped each visible section title as "{title}: {count}", and a source-control tooltip of exactly "Source Control" when no changes were open, or "Source Control" followed by " - {title}: {count}" for each visible section in order when changes were open, while the accessible name stayed exactly "Source Control"; that record is not the current count contract (path: crates/made-app/src/explorer_view.rs)

## Checkpoint

- [x] shipped section counts used the colon-and-hyphen format; not verification of the current count scenarios

## 18. Parenthetical section counts

- [x] 18.1 Show each visible section title as "{title} ({count})", and set the source-control tooltip to exactly "Source Control" when no changes are open, or "Source Control - " plus each visible section as "{title} ({count})" joined by " · " when changes are open, while the accessible name stays exactly "Source Control" (path: crates/made-app/src/explorer_view.rs)

## Checkpoint

- [x] verify Visible sections show their row counts
- [x] verify Tooltip lists visible section counts
- [x] verify Tooltip uses parentheses and a middle dot
- [x] verify Clean repository

## 19. Explorer files mark

- [x] 19.1 Draw the Explorer header icon as two offset rounded sheets, the back sheet up-left and the front sheet down-right, not a folder and not a tree, while its name stays "Explorer" (path: crates/made-app/src/theme.rs)

## Checkpoint

- [x] verify Explorer header icon is a files mark
