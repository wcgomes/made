# 0012: Open Workspace Screen Replaces Picker, `+` Menu and SSH Window

**Status:** Accepted 2026-11-27. Contracts: the `Open Workspace *`, `Hardened read-only git reads` and `Program-neutral wording` requirements in [`specs/made-ui-ux.md`](../../specs/made-ui-ux.md), SSH-01 and SSH-12 in [`specs/ssh-remote-workspaces.md`](../../specs/ssh-remote-workspaces.md). Full decision log: the archived [proposal](../../specs/changes/archive/20261127-open-workspace-redesign/proposal.md).

## Context

Opening a workspace was split across three surfaces: a `+` menu whose rows depended on detection (Terminal, TUIs that always started in the home directory, per-server SSH rows), a folder picker reached only through the Terminal row, and a separate SSH server window in the header that users found confusing. A TUI could not start in a chosen folder or worktree.

## Decision

- One Open workspace screen is the only surface for choosing host, folder and program; it is also the `--picker`, last-tab-close and smoke surface.
- `+` always opens it directly (tooltip `Open workspace`); host management moves into its host bar and the header SSH action is removed.
- Every launch, Terminal or TUI, local or SSH, starts in the effective launch path (selected worktree, else folder).
- The screen shows git state from read-only reads, which are hardened for every caller because selecting a folder now runs git on untrusted repositories.

## Rejected alternatives

- **Keep the `+` menu and add folder choice behind each row:** two steps for every launch and the detection-dependent branch stays.
- **Keep a separate SSH management window, or a header shortcut to the screen:** two entry points for the same task.
- **A separate, simpler startup picker:** two surfaces to maintain.

## Consequences

- TUIs no longer launch in the home directory; [ADR 0001](0001-opencode-plugin-loopback-bridge.md) and [ADR 0005](0005-home-default-startup.md) read "picker"/"`+` menu" as this screen.
- Remote browsing needs a connection, so selecting an SSH host connects and opens a screen-scoped browse grant ([Security](../security.md#remote-browse-grant)).
- Hardened git reads change git-view results: dirty submodule content is hidden and repository-scoped filter drivers are neutralized ([Security](../security.md#hardened-git-reads)).
- Real-server SSH and native-backdrop visuals were left `UNVERIFIED` at archive ([Platform matrix](../testing/platform-matrix.md#open-workspace-redesign-2026-11-27)).
