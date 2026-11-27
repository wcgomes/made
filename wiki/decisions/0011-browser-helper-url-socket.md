# 0011: BROWSER Helper and Out-of-Band URL Socket

**Status:** Accepted 2026-11-27; implemented, not yet merged. Contracts: "URL Open Confirmation Card" in [`specs/made-ui-ux.md`](../../specs/made-ui-ux.md), AIA-07 in [`specs/agent-integration-architecture.md`](../../specs/agent-integration-architecture.md), SSH-05/SSH-08/SSH-11 in [`specs/ssh-remote-workspaces.md`](../../specs/ssh-remote-workspaces.md).

## Context

Agent TUIs open login and OAuth URLs on their own. A URL that a pane child, or an SSH server, can open in the host browser without a click is an untrusted action, and on SSH it opens nothing at all on the host. Made needs to intercept the request, show it, and open only on user consent, without changing the user's shell or config files.

## Decision

- Set `BROWSER` for local shell and direct TUI children to the `made-open` helper beside the executable, as a core launch contribution applied after every integration's.
- The helper validates the URL and sends `{token,url}` over a same-user Unix socket or named pipe (`MADE_URL_SOCKET`, per-launch `MADE_URL_TOKEN`). It exits 0 only on ack; any failure exits non-zero so the TUI prints the URL.
- On SSH the supervisor points `BROWSER` at a `<runtime>/made-open` symlink to `made-remote` (argv0 helper mode) and forwards `Operation::OpenUrlRequest` only to the attached client; nothing is replayed.
- The app shows an ask-before-open card and opens only on Open. Mechanism: [URL open requests](../runtime/terminal-first.md#url-open-requests).

## Rejected alternatives

- **Helper writing an OSC sequence to `/dev/tty`:** a detached `BROWSER` spawn has no controlling tty, and an in-band OSC would sit in the terminal stream and replay when a remote supervisor resnapshots.
- **`PATH` shim for `open`/`xdg-open`:** rejected by the user; it would shadow system commands for every child.

## Consequences

- Claude Code and Grok honor `BROWSER` and get the card; OpenCode calls `open`/`xdg-open` directly and still opens without one.
- A `BROWSER` assigned in `~/.bashrc`/`~/.zshrc` overrides Made's value, locally and on servers; no card appears.
- New shipped binary `made-open` in the release layout; see [Architecture](../architecture.md) and [Security](../security.md#url-open-requests).
