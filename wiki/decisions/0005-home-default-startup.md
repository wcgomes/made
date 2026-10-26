# 0005: Home-Default Startup

**Status:** Accepted for v1.

## Context

The earlier product flow opened a folder picker before any work could start. That adds a blocking decision to a terminal-first tool and makes the common case slower. Users still need an explicit picker for choosing another folder, and automation must not start a shell in a real home directory.

## Decision

With no workspace argument, open the current user's **platform-resolved home directory** as a normal terminal + Explorer workspace.

- `--workspace <path>` or a positional folder wins over `--picker`.
- `--picker` and the Open folder action show the explicit picker.
- An invalid, missing, unreadable, or unresolvable home shows a visible non-blocking error and keeps a usable terminal in a fallback directory. It does not block on the picker or crash.
- Home resolution belongs to `made-platform`; the app maps the result to the normal workspace path.
- Closing the final tab returns to the picker, not automatically to home.
- The test-only `--smoke-test` without a folder keeps its picker/no-workbench behavior so CI never starts a home shell.

## Consequences

- No-argument startup is immediately useful and never waits for OpenCode or a picker.
- Native home resolution must be verified on macOS/Windows; current evidence is Linux tests plus an isolated-HOME Xvfb smoke.
- A failed home is a terminal-only fallback, not a new picker flow.
- Explicit paths remain the compatibility/rollback path.
- Test harnesses and developer smoke must isolate `HOME`/`XDG_*`/`TMPDIR` when a workspace is supplied.

## Evidence

[`specs/changes/opencode-session-follow-v1/spec.md`](../../specs/changes/opencode-session-follow-v1/spec.md) Workspace Launch, [core design](../../specs/changes/opencode-session-follow-v1/design.md) D19, [`crates/made-app/tests/launch_workspace.rs`](../../crates/made-app/tests/launch_workspace.rs), [`crates/made-app/src/workspace.rs`](../../crates/made-app/src/workspace.rs). Linux Xvfb evidence: `RF-1`/`RF-8` in [v1 evidence](../records/20261026-v1-evidence.md).
