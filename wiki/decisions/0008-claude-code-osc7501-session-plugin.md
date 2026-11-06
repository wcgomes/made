# 0008: Claude Code via OSC 7501 and a Session-Only Plugin

**Status:** Accepted; implemented in the active change [`claude-code-integration`](../../specs/changes/claude-code-integration/spec.md). Windows and macOS hook behavior is `UNVERIFIED`.

## Context

Claude Code was excluded from the integrated-TUI catalog and showed as a plain shell tab. It now offers two channels that fit Made's terminal-first, local-only, no-config-edit model: the Program Status Protocol (OSC 7501) it emits to the hosting terminal, and plugins loaded for one session from `CLAUDE_CODE_PLUGIN_DIRS`. Neither carries everything: OSC 7501 has no directory, no reliable session identity and no session title, while hooks have no immediate "blocked" signal.

## Decision

Use two independent, fail-open channels and join them per tab inside Made. Details live in [Claude Code integration](../integration/claude-code.md).

- **Status through OSC 7501.** Made's terminal answers the capability probe for any program and consumes only session-level (no `id`) `app=claude-code` emissions. `msg` is never retained.
- **Identity and directory through a session-only plugin.** Made composes its shipped plugin into `CLAUDE_CODE_PLUGIN_DIRS` for local shell tabs and directly launched Claude Code tabs. Its native async hook `made-claude-status` appends metadata-only records to a private per-tab spool. Nothing is written to the user's Claude Code configuration, so there is nothing to uninstall.
- **Ownership by live ancestry**, never by directory or title.
- **Precedence** OpenCode > Claude Code > Grok > shell for the explorer root and tab presentation locally. Remote Claude Code yields to Grok, a recorded follow-up.
- **Security policy narrowed.** "Made does not scrape prompts/titles" becomes: Made consumes structured status-protocol metadata and reads the window title only as a sanitized tooltip, capped at 120 characters, while a `claude-code` OSC 7501 status is live in that tab. Prompts and screen contents are never read, and a title never feeds cwd, status or identity. The `Shell Directory Follow` title clause is MODIFIED to scope it to cwd discovery.
- **SSH scope.** `Claude Code SSH` launches the remote `claude` through an additive `LaunchTarget::Claude` in the companion; status arrives as OSC 7501 through the remote PTY. No remote plugin, hook, staging or write.

## Rejected alternatives

- **HTTP hooks to the OpenCode bridge:** adds a reachable listener and per-request authentication for data the command hook already writes locally.
- **Statusline command:** one user-owned slot in Claude Code settings; using it edits user config or displaces the user's statusline.
- **IDE lockfile and MCP IDE channel:** Made would impersonate an IDE and receive editor-level context.
- **Transcripts and session files:** expose prompt and model content and couple to an undocumented on-disk format.
- **`claude plugin install`:** persists into user configuration and affects Claude Code outside Made.
- **`--plugin-dir` on the direct launch:** misses a shell-typed `claude`, and the contract forbids adding arguments.
- **Title glyph parsing** (`✳` idle, `◐`/`◑` working): unstructured and version-fragile.
- **`claude agents --json`:** works, but spawns `claude` on a poll, which the metadata-only detection rule avoids. Out of contract.

## Consequences

- Each channel works alone; with both missing the tab stays a plain shell tab. Diagnostics tells which channel is missing.
- The terminal now writes a reply to a program-initiated query it previously ignored. The reply is byte-identical to the probe, so a tty that echoes input would loop; the receiver answers at most one probe per second per terminal.
- Reply ordering is a Made-side requirement: the probe reply must precede the reply to any query that follows the probe in the output stream (DA1 is the observed case).
- The hook is a second native hook beside `made-grok-status`; the "OS code only in `made-platform`" rule has a second narrow exception for its pid-chain lookup.
- Owned release layout gains `plugins/claude-code-made`; the REL-03/REL-04 reconciliation with `add-release-auto-update` is recorded in the change's proposal.
- A same-user process can forge spool records; this stays inside the existing same-user exclusion and is still subject to live ancestry.

## Evidence

Phase 0 ran 86 runs against claude 2.1.319 to find why reporting sometimes did not enable. With the probe reply written after the DA1 reply, 17 of 54 runs enabled regardless of plugin count, hook count, sync or async, entry order, a nonexistent or duplicated second entry, or no plugin. With an in-order responder 19 of 19 enabled, and a DA1-first control enabled 9 of 13. The earlier "two plugin directories never enable 7501" observation was an artifact of the spike responder's order, not a Claude Code limit.

See [`design.md`](../../specs/changes/claude-code-integration/design.md) and [`proposal.md`](../../specs/changes/claude-code-integration/proposal.md), and the [security model](../security.md).
