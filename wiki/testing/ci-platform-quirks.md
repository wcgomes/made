# CI Platform Quirks

Distilled from the 2026-10-27/28 cross-platform CI stabilization. These are non-obvious harness and product behaviors; read the linked tests/source before changing them, and do not weaken product behavior to satisfy a harness.

## Windows ConPTY

- conhost and PowerShell emit a cursor-position DSR (`ESC[6n`) at startup and block until the terminal replies `ESC[1;1R`; answer every query, including ones that arrive after commands are queued, or the shell never reaches a prompt. Scan with a carry buffer across reads so a query split between reads is not missed.
- Whole-line harness input uses CRLF on Windows; the product's physical-Enter encoding is one `\r`. Keep the test submission distinct from the product encoding.
- Coverage: [`pty.rs`](../../crates/made-terminal/tests/pty.rs) `answer_console_dsr`/`settle_console`, [`shell_integration.rs`](../../crates/made-platform/tests/shell_integration.rs), [`contract.rs`](../../crates/made-platform/tests/contract.rs).

## macOS bash 3.2 prompt hooks

- macOS `/bin/bash` 3.2 executes only element 0 of a `PROMPT_COMMAND` array. The shell hook appends natively on bash 5.1+, but flattens existing hooks through `IFS=$'\n'` and `"${PROMPT_COMMAND[*]}"` on older shells so every user hook runs before `__made_cwd`.
- Regression: [`pty.rs`](../../crates/made-terminal/tests/pty.rs) `bash_prompt_command_array_keeps_every_hook_in_order` asserts two hooks run in order; set `MADE_TEST_BASH32` to exercise a 3.2 binary locally. Source: [`shell_integration.rs`](../../crates/made-platform/src/shell_integration.rs).

## macOS FSEvents metadata

- FSEvents reports access/atime and metadata-only changes (`INODE_META_MOD`), including chmod and touch. Preview refresh ignores both, so permission-only changes do not re-read while content writes still do. This is intentional; the preview can remain stale until the next content event or explicit refresh.
- Source: [`preview_refresh.rs`](../../crates/made-explorer/src/preview_refresh.rs) `is_content_event`; tests: [`preview_refresh.rs`](../../crates/made-explorer/tests/preview_refresh.rs).

## Canonical path spelling

- `canonicalize` yields `/private/var` on macOS and `\\?\C:\...` on Windows, while watcher events may use `/var` or plain `C:\...`; normalize event paths to the canonical root before comparing, including deletion events (canonicalize the parent plus file name).
- Only the shell-launch cwd strips the Windows extended-length prefix (`\\?\UNC\` becomes `\\`, `\\?\` becomes plain), because shells reject it. Keep canonical paths for app identity and security checks.
- Sources: [`watch.rs`](../../crates/made-explorer/src/watch.rs) `normalize_event_path`, [`lib.rs`](../../crates/made-platform/src/lib.rs) `shell_compatible_cwd`.

## Bridge close drain

- Windows resets a TCP socket dropped with unread receive data; that reset can replace a flushed close frame and lose code 1009 (size) or 4409 (replaced). Finish closes by enqueueing `Disconnected`, signalling the replacement `done`, then flushing, draining input for a bounded time, and half-closing the write side before drop. `Disconnected` must precede the replacement's `Connected`.
- Source: [`server.rs`](../../crates/made-bridge/src/server.rs) `finish_close`; tests: [`protocol.rs`](../../crates/made-bridge/tests/protocol.rs) `oversized_message_closes_with_1009` and `newer_connection_replaces_older_with_4409_in_event_order`.

## Timing and budgets

- egui creates tooltip areas one pass after hover; loaded runners need retries until the exact text appears. Retry readiness, never relax the assertion.
- The 10k-row selection guard is 16 ms on Windows and macOS x86_64 debug runners and 5 ms on Linux/macOS arm64; the product contract remains frame p95 below 16.667 ms. Treat the test guard as runner headroom, not the product budget.
- Sources: [`ui_model.rs`](../../crates/made-app/tests/ui_model.rs) `hover_until_text`, [`explorer_view.rs`](../../crates/made-app/src/explorer_view.rs), [Performance](../performance.md).

## Windows watched-root deletion

- Recursive deletion can fail with a sharing violation while a watcher handle is open even though `notify` uses `FILE_SHARE_DELETE`. The unavailable-root test renames the root on Windows and deletes recursively on Unix; this simulates loss of the watched path without weakening product cleanup.
- Test: [`workspace_tabs.rs`](../../crates/made-app/tests/workspace_tabs.rs).

## CI invocation

- CI runs `cargo test --workspace --locked --no-fail-fast`, so all failing test binaries report in one run; fix the aggregate rather than assuming the first failure is the only one.
- Workflow: [`ci.yml`](../../.github/workflows/ci.yml).
