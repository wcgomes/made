# Proposal: Read-only explorer git view

## Intent

The shipped explorer can switch a known git worktree to a read-only source-control view, but the realized UI contract does not record that outcome. This change adds that contract as a delta on `made-ui-ux` so the live spec matches what the view already shows, without treating the wiki as the contract. It also refines the file-tree mark, moves the git-view head line into a `REPOS` header, shows a detached HEAD as an 8-character commit id with the commit subject on hover, marks a linked worktree with a small monochrome icon, and when the folder name matches the branch or commit id shows the icon and that folder name only.

## Scope

This is a new active change, `explorer-git-view`, against the realized `specs/made-ui-ux.md`. `specs/changes/` has no other active change; archived changes are not edited. The delta ADDs the git-view outcomes: icon visibility and names, the file-tree title `EXPLORER` plus the root name, the git-view title `REPOS` with the head line in the header and not repeated in the body, the branch name or an 8-character commit id when detached, the commit subject on hover, a small monochrome icon on a linked worktree, including when the folder name matches the branch or commit id and the header then shows the icon and that folder name only, grouped sections, empty-section omission, "No changes", the git-view file or folder icon and right-side status dot instead of a painted letter, the right-side change dot and kind tooltip on a changed folder and on a changed file that is already a tree row, mixed-kind `ACCENT`, file activation of the existing read-only preview, directory activation that does not open a preview, a failed or missing status read that does not block the file tree, changes limited to the explorer root, read-only git, and a per-workspace in-memory view choice. No realized requirement is modified or removed. The existing bans on Wider, Narrower, Reload, Hide, text action buttons, and a routine refresh control remain in force.

## Non-goals

- Stage, unstage, discard, commit, checkout, or any other git write.
- A commit box, a flat change list, or a diff editor.
- A Reload button, or any new Wider, Narrower, Hide, or routine refresh control.
- Status letters on file-tree rows or painted status letters on git-view rows.
- A fabricated row for a deleted path that is not already in the file tree.
- Changing the file-icon theme or file-tree entry icons.
- Keyboard navigation of git-view rows, which is not shipped and was not confirmed.
- `SOURCE CONTROL` as the git-view header title.
- The words "Detached HEAD" as the visible head line.
- A dirty-worktree asterisk.
- A worktree icon on a non-linked checkout.
- A second copy of the branch or commit id, or an icon-before-head layout, when the linked-worktree folder name already equals that text.
- Unifying Diagnostics worktree phrasing with the git-view label.
- Archiving this change, or adding `design.md`.
- Edits to `specs/made-ui-ux.md`, `specs/explorer-file-icons.md`, or `wiki/`.

## Approach

Record the already shipped view as observable outcomes, refine the file-row dot in the same change, move the git-view head line into the header under the title `REPOS`, replace the visible "Detached HEAD" words with an 8-character commit id, and mark a linked worktree with a small monochrome icon instead of " · ", and, when that folder name matches the branch or commit id, shows the icon and the folder name only instead of a separate icon-before-head layout. Each product choice fits in the decision index, so `design.md` is not created: there is no alternatives essay, interacting flow, or migration, rollback, or security expansion that the index cannot carry. Normative guarantees stay in `spec.md`. Implementation constraints, including how a status snapshot is read, stay out of the spec. This refinement does not change the application and does not claim that analyze or verify passed.

Conflicts are recorded here and are not silently preferred. The wiki UI pages already describe this view, and they cite `specs/changes/archive/20261029-made-ui-ux-v1/spec.md` as the normative UI contract. The live contract is `specs/made-ui-ux.md` plus this active delta. Those wiki pages are not copied into the spec and are not edited. `wiki/architecture.md` states that status paths are canonicalized because macOS `/var` and `/private/var` would otherwise drop every entry. That is an implementation constraint. The spec states only that changes under the explorer root are shown. `specs/explorer-file-icons.md` still requires a theme icon on every regular file and directory row in the file tree. Git-view rows are a separate surface. Each shows the same file or folder icon used in the file tree and one right-side status dot, not a letter. This delta does not modify that specification. Diagnostics uses a lowercase "detached HEAD" form inside a worktree-kind label. This delta does not unify that phrasing with the git view. The git view shows an 8-character commit id instead of the words "Detached HEAD". `wiki/ui/layout-and-controls.md` still describes those words and is not edited. The realized Keyboard Focus requirement still says every persistent interactive control must be keyboard reachable, and that the explorer tree must remain keyboard operable. Shipped git-view rows are pointer-activated and are not keyboard stops. This delta does not weaken that requirement and does not add an unshipped keyboard surface. The realized accent sentence lists current accent uses and is not exclusive, so a mixed-kind dot using `ACCENT` does not make that sentence false; the use is added, not modified. The realized explorer row presentation bans an accent side bar; the right-side change dot is not that side bar and does not modify that requirement. The wiki says a failed read shows "Git status unavailable" without saying that this copy appears only when the git view is open and no status snapshot is available. The spec contracts the shipped condition. `wiki/ui/layout-and-controls.md` says a painted folder dot sits on the right of the row and does not mention a file-row dot. This delta requires that same right-side dot on a changed file that is already a tree row. That page is not edited. `wiki/ui/layout-and-controls.md` also says the git view keeps the `EXPLORER` title and shows the head line in the body. This delta now requires `REPOS` in that header and does not repeat the head line in the body. That page is not edited.

## Decision index

- **New retrospective delta:** create `explorer-git-view` against `made-ui-ux`; do not edit the realized spec, the file-icon spec, the wiki, or archived changes until a later confirmed archive.
- **Grouped sections, not a flat list or a commit box:** show only the non-empty sections Merge Changes, Staged Changes, Changes, and Untracked, in that order.
- **Existing-row change dot:** a changed file that is already a tree row shows one right-side dot and a kind tooltip, the same treatment as a changed folder; file-tree rows still have no status letter; deleted paths that are not in the tree do not get a fabricated row.
- **Dot color:** a single-kind dot uses that kind's semantic color; a mixed-kind dot uses `ACCENT`, preserving the one-accent rule rather than adding a second accent. The same rule applies to a changed folder and to a changed file that is already a tree row.
- **Existing preview:** activating a git-view file opens the existing full-left read-only preview; activating a directory does not.
- **Icon and names:** show the icon only for a known git worktree; its tooltip and accessible name are "Source Control" in the file tree and "Explorer" in the git view; the icon switches the body and does not hide the explorer.
- **Git-view header title:** the user confirmed `REPOS`, not `SOURCE CONTROL`, because the longer title would crowd the branch name. The file-tree header stays `EXPLORER` plus the root name. While the git view is showing, the title is `REPOS` in the same section-label treatment, and the existing head line replaces the root name.
- **Head label:** a non-linked attached HEAD shows the branch name. A non-linked detached HEAD shows an 8-character commit id, not the words "Detached HEAD". Hover shows the commit subject, or the id if the subject is empty, including when the line is truncated. When a linked-worktree folder name differs from that text, show head text, then icon, then folder name. Do not repeat the line in the body.
- **Linked-worktree icon:** when the folder name differs from the branch or commit id, the text beside that head text is the linked-worktree name, not a branch. The user confirmed a small monochrome icon immediately before that name. The icon replaces the " · " separator. Hovering the icon or the name says it is a linked worktree and includes the name. A non-linked checkout shows no worktree icon and no worktree name. A shown commit id still uses the 8-character rule. No dirty asterisk.
- **Matching folder name replaces the head:** the user rejected the icon-before-head layout. When the folder name equals the branch or the 8-character id, the header shows the icon and that folder name only. It does not also show the branch or id. When the name differs, the order stays head text, then icon, then folder name. Hovering the icon or the name says it is a linked worktree and includes the folder name.
- **Detached id, VS Code style:** the user confirmed a short id such as `fb16d0ee`, with the commit text on hover. They did not ask for a dirty asterisk. A later confirmation adds a monochrome icon for a linked worktree. When the folder name equals the id, the header shows the icon and that folder name only, not a second copy of the id. A non-linked detached HEAD still shows the id. Diagnostics phrasing stays unchanged.
- **Git-view row mark:** the user confirmed this replaces painted letters. A git-view row shows the same file or folder icon used in the file tree, immediately before the path, and one right-side status dot. The dot uses the existing kind colors. Its tooltip names New, Modified, Deleted, Renamed, or Conflict, and that tooltip is the non-color cue. A staged add and an untracked path both use New; the section title already separates them. A status-dot tooltip, in the file tree and in the git view, is anchored at the dot, not at the far side of the row. Clicking a file still opens the existing preview. Clicking a directory still does not.
- **Partial staging and ignored paths:** a partially staged file appears in both Staged Changes and Changes; ignored paths are not listed.
- **Clean and failed reads:** a clean repository shows "No changes"; a failed or missing read does not block the file tree.
- **Explorer-root guarantee:** show changes under the explorer root; do not put the macOS canonicalize call in the spec.
- **Read-only:** no stage, unstage, discard, commit, checkout, or other git write, and no new banned chrome.
- **View choice:** remember the file-tree or git-view choice per open workspace in memory; do not restore it after restart.
- **No design overflow:** the index is sufficient, so `design.md` is omitted.
- **Conflicts stay flagged:** wiki pages are descriptive and are not edited, including the folder-dot sentence that does not yet mention a file-row dot, the sentence that keeps the git-view title as `EXPLORER`, and the sentence that still shows the words "Detached HEAD"; the accent list is not rewritten; file-icon and Diagnostics contracts are not edited; the keyboard gap is not silently closed.

## Clarifications

- **Q:** Should this be a new change `explorer-git-view`, as a delta on `made-ui-ux`, retrospective to the shipped view?
  **A:** Yes. It records the shipped view as a delta on `made-ui-ux`. The file-row dot stays in this change as one follow-up task. This revision does not implement, archive, or claim that analyze or verify passed.
- **Q:** Should the git view be a flat list, or include a commit box?
  **A:** Neither. Changes are grouped as Merge Changes, Staged Changes, Changes, and Untracked. There is no commit box.
- **Q:** How should the file tree show changes?
  **A:** A changed folder shows one right-side dot and a kind tooltip. A changed file that is already a tree row shows the same dot and tooltip. File-tree rows do not show status letters. Deleted paths that are not in the tree do not get a fabricated row.
- **Q:** Which color marks a mixed-kind dot on a folder or on a changed file that is already a tree row?
  **A:** `ACCENT`. A single-kind dot uses that kind's semantic color.
- **Q:** What does activating a git-view row do?
  **A:** A file opens the existing read-only preview. A directory does not.
- **Q:** Should a changed file that is already a tree row show the status dot, not only a changed folder?
  **A:** Yes. It shows one right-side dot and a kind tooltip. One kind uses that kind's semantic color. More than one kind uses `ACCENT`. The tooltip names the present kinds. File-tree rows still have no status letter. A deleted path that is not in the tree does not get a fabricated row.
- **Q:** Should a detached HEAD show the words "Detached HEAD"?
  **A:** No. The user chose the VS Code-style 8-character commit id, such as `fb16d0ee`, with the commit subject on hover. An empty subject still shows the id. A distinct linked-worktree name is still shown when it differs from the visible id, compared with that id rather than those words. A later confirmation replaces the " · " before that name with a small monochrome icon. No dirty asterisk. Diagnostics phrasing is not unified.
- **Q:** Is the header text beside a detached commit id a branch or a worktree?
  **A:** It is the linked-worktree name, not a branch. When that name is shown, a small monochrome icon sits immediately before it and replaces the " · " separator. Hovering the icon or the name says it is a linked worktree and includes the name. A non-linked checkout shows no worktree icon and no worktree name. The attached branch and the detached commit id are otherwise unchanged. No dirty asterisk.
- **Q:** Should the linked-worktree icon disappear when the folder name equals the branch or the commit id?
  **A:** No. The icon stays. A later confirmation shows that folder name instead of the branch or id, rather than omitting the name or painting the icon before the head text. Hovering the icon or the name says it is a linked worktree and includes the folder name. A non-linked checkout still shows the branch or id and no worktree icon.
- **Q:** Where does the icon sit when the duplicate folder name is omitted?
  **A:** That layout is rejected. The header shows the icon and the folder name only, not the branch or id as well, and not a separate icon-before-head order. When the folder name differs, the order stays head text, then icon, then folder name.
- **Q:** Should a matching folder name paint the icon before the branch or commit id?
  **A:** No. The user rejected that special case. Show the icon and that folder name instead of the branch or id. The distinct-name order is unchanged.
- **Q:** Should a git-view row keep the painted status letter?
  **A:** No. The user rejected the letter before the path. The row shows the same file or folder icon used in the file tree, immediately before the path, and one right-side status dot. The dot tooltip names the kind and is anchored at the dot, in the git view and in the file tree. A staged add and an untracked path both use New. Clicking a file still opens the existing preview. Clicking a directory still does not.
- **Q:** Should the git-view header title be `SOURCE CONTROL`?
  **A:** No. The user rejected that label because it would crowd the branch name. The title is `REPOS`, in the same section-label treatment as `EXPLORER`. The existing head line moves into the header where the root name sits in the file tree. The git-view body does not repeat it. The file-tree header stays `EXPLORER` plus the root name. The icon names stay "Source Control" and "Explorer", and the icon still does not hide the explorer.
