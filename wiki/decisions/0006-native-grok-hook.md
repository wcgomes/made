# 0006: Native Grok Status Hook

**Status:** Accepted; merged in PR #2.

## Context

The `grok-made` plugin ran its session-status hook as `hooks/bin/status.py`, so local Grok session follow and remote SSH hook activity silently depended on a Python 3 interpreter that Made neither ships nor installs. Made's goal is to install and run with nothing else pre-installed.

## Decision

Replace the Python hook with a small Rust executable, `made-grok-status` (`made-grok-status.exe` on Windows), in the workspace crate `crates/made-grok-hook`.

- Observable behavior is the Python contract: same events, status rules, seven line fields, log path (`made-grok-status.jsonl` under `$GROK_HOME`), bounded stdin read (~2 s), exit 0, empty stdout. Every `status_test.py` case was ported 1:1; `status.py` and `status_test.py` were deleted.
- The repository `plugins/grok-made` is source only. `.github/scripts/stage-grok-plugin.sh` stages the shipped package (mode 755 hook) for development runs, desktop packages, and remote companion stage sets. A development build never falls back to the repository source.
- The remote stage set shrank from 11 to 10 artifacts. The hook is built under the same glibc 2.31 / macOS 12 floors as `made-remote`.
- `Capabilities.python3` stays in protocol major 1 as a frozen field, always false and ignored. The `python3-missing` diagnostics state and the python3 PATH probe were removed.
- Windows `hooks.json` is generated at staging with the `.exe` command name, because Grok's documentation only promises `.exe` resolution.
- Python remains only as build, CI, and test tooling (`.github/scripts/*.py`); it is not a product prerequisite.

## Consequences

- No interpreter prerequisite locally or on SSH servers.
- Verified with real grok 1.0.70: `grok plugin install` copies (no symlinks) and keeps the executable mode on Linux and macOS; the packaged hook runs with Python absent from PATH on all four targets.
- Unverified: that the logged pid equals the Grok process in a real signed-in session, and Windows command resolution beyond documentation. See the [release record](../records/20261106-v0-1-0-release.md).
- A `GROK_HOME` on a network filesystem does not get the no-interleaving guarantee for log appends.

## Evidence

[`specs/changes/grok-native-hook/proposal.md`](../../specs/changes/grok-native-hook/proposal.md), [`tests/e2e/grok_native_hook_validation.md`](../../tests/e2e/grok_native_hook_validation.md), [Grok integration](../integration/grok.md).
