# Proposal: Remote Claude Code parity in SSH workspaces

## Intent
Realized SSH-11 limits remote Claude Code to launch plus OSC 7501 status and explicitly excludes the session-only plugin, hook records, ownership, explorer follow, hook-derived title and the local Claude Code-over-Grok precedence, so a remote Claude Code tab never follows its session directory and its diagnostics row can never reach ready. The user states that remote parity was always the expected behavior and chose full parity: remote Claude Code in an SSH workspace must meet the local Claude Code contract, the way SSH-10 makes remote Grok Build mirror the local Grok contract.

## Scope
Active change `ssh-claude-code-parity`, user-confirmed, full parity chosen. No other active change exists under `specs/changes/`. It modifies realized contracts in three domains:
- `specs/ssh-remote-workspaces.md`:
  - `Remote Claude Code launch and status (SSH-11)`: replaced by a parity contract. The session-only plugin and native hook are staged on the server with the companion; `CLAUDE_CODE_PLUGIN_DIRS` and the spool binding are set for remote shell tabs and `Claude Code SSH` launches; the spool is ingested on the server with process-ancestry ownership and relayed to the client over the SSH transport; remote explorer follow includes `/cd` and agent-view through a triggered remote `claude agents --json`; the hook-derived session title and status fallback apply; local precedence (Claude Code over Grok) applies remotely; and the remote diagnostics row can reach ready.
  - `Remote OpenCode session and cwd follow (SSH-06)`: the fixed staged set grows from 10 to 14 artifacts (adds the `claude-code-made` manifest, README, `hooks/hooks.json` and the target-built native hook); staging consent text gains the Claude Code package.
  - `Remote security and private state (SSH-08)`: Claude Code is added to the protected personal configuration, the remote `claude agents --json` listing becomes an authorized remote command, and remote Claude Code ingestion is bounded to the app-owned spool and listing fields.
  - `Local preservation and truthful parity evidence (SSH-09)`: diagnostics include remote Claude Code status; remote claude is optional; no interpreter is a prerequisite for the remote Claude Code hook.
  - `Remote Grok Build session follow and tab parity (SSH-10)`: remote Grok yields root and presentation to a remote Claude Code session as locally.
- `specs/claude-code-session-follow.md`:
  - `Session-only Claude Code plugin`: no longer forbids SSH tabs or remote processes; remote loading is owned by SSH-11 and uses only the server-staged plugin.
  - `Claude Code privacy and observation boundary`: the observation and privacy boundary applies on the server too, and only metadata fields may cross the SSH transport.
- `specs/agent-integration-architecture.md`:
  - `Declared precedence (AIA-05)`: "remote root sources and the remote presentation rule stay as they are" becomes "remote order equals the local order".

Unchanged and intentionally kept: SSH-01 (detection and the `Claude Code SSH` row already defer to SSH-11), SSH-02 path validation, SSH-07 survival/resnapshot rules, protocol major 1, all local Claude Code requirements other than the two above, all Grok and OpenCode local contracts, and AIA-01 (see conflicts).

## Non-goals
- Any change to local Claude Code behavior, hook event semantics, record fields, status mapping or diagnostics wording for local tabs.
- Any write to, or read of, the remote user's Claude Code configuration or state (`~/.claude/**`), `claude plugin install`, or any `claude` subcommand other than `claude agents --json`.
- Installing claude on the server, or making claude a remote prerequisite.
- Windows remotes and Linux musl (unsupported per SSH-09).
- Claude Code started inside tmux or other processes outside the tab's managed process tree (not claimed, as locally).
- Remote listing triggers beyond the local trigger set; a periodic poll.
- Wiki and realized `specs/*.md` edits inside this change; they are follow-ups after `verify` and at archive (tasks §6).

## Approach
Mirror how SSH-10 extends the local Grok contract: SSH-11 states that remote Claude Code mirrors the local Claude Code contract for the covered flows and then states only the remote deltas (server-side staging, server-side environment composition, server-side ingestion and ownership, relay over the authenticated SSH transport, epoch and reattach rules, remote validation, remote diagnostics sourcing). The plugin and its hook are staged under the existing SSH-06 private version directory and verified with the other artifacts, so no new remote write location is introduced. The companion side owns everything that needs live server process ancestry (spool ingestion, ownership, liveness, the listing run); the client keeps presentation, precedence and explorer root, as it does for remote Grok. `design.md` is not created: the flow is the local flow plus a relay, and the clarified decisions fit as short entries plus tasks (overflow rule reapplied after `clarify`).

Conflicts and overlaps flagged, not resolved here:
- `wiki/integration/claude-code.md` §SSH says remote session and explorer follow are "deferred", while realized SSH-11 says they are excluded ("MUST be limited to"); this change follows the wiki's framing and the user's statement. On archive the wiki §SSH, the `Where.` bullet ("never ... SSH launches"), the precedence bullet ("Remotely the SSH-10 order is kept ... recorded follow-up") and "the companion `Capabilities` struct is unchanged ... gains only an additive `LaunchTarget::Claude`" become stale.
- `wiki/integration/claude-code.md` §Spool places the local spool under the per-user app temp (`<tmp>/claude/<pid>-<id>/`), while realized `Claude Code session events` says "Made's private per-user application directory". The two wordings differ (temp vs application directory); the clarified remote spool is per-session supervisor runtime state, so the remote wording does not depend on resolving this local mismatch, which stays a wiki/spec wording conflict.
- `wiki/integration/grok.md` §SSH states "The remote stage set is 10 artifacts"; it becomes 14. It also links the archived SSH contract rather than the realized `specs/ssh-remote-workspaces.md`.
- `wiki/integration/grok.md` §SSH records that Made's Grok installer needs its own per-server/version consent after companion staging; that separate consent exists because it writes into the user's Grok home. Claude Code staging writes only app-owned files, which is why reuse of SSH-06 consent is the recommendation below.
- Realized AIA-01 (`Behavior preservation`) says the agent-integration reorganization kept "remote root sources and remote presentation unchanged" and its scenario pins "Grok Build under SSH-10 before Claude Code under SSH-11". It is scoped to that reorganization ("The reorganization MUST NOT change"), so it is not modified; a test that encodes that scenario literally will need updating under this change, which is allowed because it is a deliberate contract change, not a reorganization step.
- Realized AIA-07 says "remote targets receive no host integration environment and start no host process". This change keeps that: the remote environment is composed on the server from server-staged paths; no host path or host value is sent.
- The `## Purpose` of `specs/ssh-remote-workspaces.md` ("remote Claude Code launch and status") and of `specs/claude-code-session-follow.md` ("Remote Claude Code launch and status is owned by SSH-11") are not requirements and cannot be carried by this delta; on archive they should be reworded to "remote Claude Code session follow and tab parity".
- Realized AIA-10 (`Shared hook library`) scenario `Packaged hooks unchanged` says companion stage sets keep "the same executables, names, modes and checksums-file entries". Like AIA-01, it is scoped to the reorganization and is not modified; adding the Claude Code hook to each companion stage set is this change's deliberate contract change under SSH-06.
- Realized REL-03 lists the per-target `remote/<target>/made-remote` companion but does not enumerate the per-target remote stage set (the Grok hook is already in it without being named); adding the per-target Claude Code hook does not contradict REL-03 but extends the packaged per-target remote payload.

## Decision index
- **Parity contract in SSH-11, local contract by reference:** SSH-11 says remote Claude Code mirrors `specs/claude-code-session-follow.md` for the listed flows and states only remote deltas, like SSH-10; this avoids duplicating ~600 lines and keeps one source of truth for event semantics.
- **Requirement name kept:** SSH-11 keeps its realized name so archive is a clean MODIFIED replacement; renaming to "session follow and tab parity" is cosmetic and can be done at archive if the user prefers.
- **Staging location:** the Claude Code package lives in the same private, verified SSH-06 version directory as the companion and `grok-made`; no new remote write location. Stage set 10 → 14.
- **Server-side ownership:** spool ingestion, live PID ancestry, liveness and the listing run happen on the server because only the server sees remote processes; the client receives only metadata-level session state, scoped to its tab and epoch.
- **Session state survives detach:** because SSH-07 keeps remote Claude Code running across transport loss, its owned-session state is kept on the server while the managed session lives and is delivered as a fresh snapshot on reattach; previous-epoch relay data is discarded (mirrors SSH-10 snapshot rule).
- **Remote budgets:** remote follow uses the SSH-09 two-second budget instead of the local ~250 ms / ~1 s, consistent with remote Grok.
- **Precedence:** remote root OpenCode > Claude Code > Grok > remote shell cwd > launch directory; remote presentation OpenCode connected > Claude Code present > Grok > server-name label; AIA-05 updated accordingly.
- **Consent model:** the existing SSH-06 per-server/version companion staging consent authorizes the Claude Code package; there is no separate Claude Code consent. The package is app-owned and loaded session-only, unlike SSH-10's Grok-home install.
- **Writes outside app-owned storage:** none. Made never reads or writes `~/.claude/**` on the server, and SSH-10's Grok install remains the single write outside app-owned storage.
- **Listing trigger source and worker scope:** the client detects triggers (submitted line, title change while protocol status is live, session becoming owned) and sends a bounded trigger with no input bytes or title text; one listing worker per server user serves every remote tab of that client on that server.
- **Remote spool location and cleanup:** per-session runtime state owned by the tab's supervisor; removed on owned-session teardown and lease expiry; swept at helper start only when the owning supervisor is gone; no removal claimed while unreachable.
- **Remote shell-typed claude gets the plugin:** yes, every supported remote shell tab receives the plugin entry and spool binding.
- **Remote hooks-unavailable sentence:** `Claude Code session-status hooks are unavailable over SSH.` is retired; remote rows use the local sentences, and remote-only reasons appear only in the sectioned detail.
- **Shared pure Claude Code helpers (non-normative):** `made-app` already depends on `made-remote`, so the pure pieces both sides need (record parsing, `CLAUDE_CODE_PLUGIN_DIRS` composition, listing parsing and title normalization) move into a `made-remote` Claude Code module and the local integration module delegates to them, with local behavior and tests unchanged. This avoids a second copy like the Grok companion module.
- **Hooks availability signal (non-normative):** the companion reports whether the staged Claude Code package is available for the tab inside its Claude Code state reply; a tab attached to an older supervisor that does not know the operation yields "hooks unavailable". `Capabilities` stays unchanged, and the per-attach `Capabilities` request added by commit 54c0bf6 is reused, not duplicated.
- **Published listing cleanup:** the app-owned published `claude agents --json` result is removed when the last supervisor of the server user that can run the listing ends and swept at every helper start when none runs, mirroring the spool rule; recorded as one sentence in SSH-11.
- **Accepted residual, same-version older supervisor:** the client sends the Claude Code operations only to a supervisor whose `helper_version` (pre-release/build suffixes ignored) is at least its own, so a supervisor of the same version started from older artifacts is not detected; accepted on the user's decision because every release bumps the version. Revisit if a same-version companion update ships outside a release.
- **No design.md:** the overflow rule was reapplied after clarify and is still not met. Every decision fits as a short entry plus tasks, and the per-server-user listing worker is a single coordination point described in tasks.

## Clarifications
- **Q:** Consent model for staging the Claude Code plugin and hook on the server: reuse the SSH-06 companion staging consent (a), or add a separate per-server/version Claude Code consent like Grok's installer (b)?
  **A:** (a). The SSH-06 staging consent covers the Claude Code package, and a refused staging consent writes nothing, as SSH-06 defines. Recorded in SSH-11, SSH-06 and the `Staging consent declined` scenario.
- **Q:** Does every supported remote shell tab receive the plugin entry and spool binding, so shell-typed remote `claude` loads the plugin?
  **A:** Yes. Recorded in SSH-11.
- **Q:** Is any remote write outside app-owned storage allowed for Claude Code parity?
  **A:** No. Made never reads or writes `~/.claude/**` or any remote Claude Code configuration or state. Recorded in SSH-11 and SSH-08.
- **Q:** Remote spool location and cleanup?
  **A:** Per-session runtime state owned by the tab's supervisor, removed on owned-session teardown and lease expiry, and swept at helper start only for spools whose supervisor is gone. Recorded in SSH-11.
- **Q:** Remote listing trigger source and worker scope?
  **A:** The client detects triggers and sends a bounded trigger with no input bytes; one worker per server user. Recorded in SSH-11.
- **Q:** Retire the remote sentence `Claude Code session-status hooks are unavailable over SSH.`?
  **A:** Yes. The local sentence is used, and remote-only reasons go in the sectioned detail. Recorded in SSH-11.
