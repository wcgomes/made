# CI Platform Quirks

Distilled from the 2026-10-27/28 cross-platform CI stabilization. These are non-obvious harness and product behaviors; read the linked tests/source before changing them, and do not weaken product behavior to satisfy a harness.

## Windows ConPTY

- conhost and PowerShell emit a cursor-position DSR (`ESC[6n`) at startup and block until the terminal replies `ESC[1;1R`; answer every query, including ones that arrive after commands are queued, or the shell never reaches a prompt. Scan with a carry buffer across reads so a query split between reads is not missed.
- Whole-line harness input uses CRLF on Windows; the product's physical-Enter encoding is one `\r`. Keep the test submission distinct from the product encoding.
- Coverage: [`pty.rs`](../../crates/made-terminal/tests/pty.rs) `answer_console_dsr`/`settle_console`, [`shell_integration.rs`](../../crates/made-platform/tests/shell_integration.rs), [`contract.rs`](../../crates/made-platform/tests/contract.rs).

## Windows GUI subsystem

- `made-app` `main.rs` carries `#![cfg_attr(windows, windows_subsystem = "windows")]`, the one accepted OS `cfg` outside `made-platform` (a crate-root attribute). CLI actions and `--smoke-test` attach to the parent console; GUI runs detach inside the `run_native` creator closure, after nulling console-backed std handles.
- Every non-PTY console child must go through `made_platform::hide_console_window` (`CREATE_NO_WINDOW`) or it pops a console window.
- Interactive cmd/PowerShell do not wait for `made --version` or `--update` output; piped and CI invocations do.
- Source: [`main.rs`](../../crates/made-app/src/main.rs), [`lib.rs`](../../crates/made-platform/src/lib.rs).

## macOS bash 3.2 prompt hooks

- macOS `/bin/bash` 3.2 executes only element 0 of a `PROMPT_COMMAND` array. The shell hook appends natively on bash 5.1+, but flattens existing hooks through `IFS=$'\n'` and `"${PROMPT_COMMAND[*]}"` on older shells so every user hook runs before `__made_cwd`.
- Regression: [`pty.rs`](../../crates/made-terminal/tests/pty.rs) `bash_prompt_command_array_keeps_every_hook_in_order` asserts two hooks run in order; set `MADE_TEST_BASH32` to exercise a 3.2 binary locally. Source: [`shell_integration.rs`](../../crates/made-platform/src/shell_integration.rs).

## macOS bash 3.2 non-ASCII input

- bash 3.2 readline drops 8-bit input without a UTF-8 locale. Any test that spawns `/bin/bash` 3.2 and types non-ASCII text must pin a UTF-8 locale and isolate `INPUTRC`/`HOME`, or it fails on a locale-less runner. Product rule: [terminal-first](../runtime/terminal-first.md) (Local PTY locale).
- Known open case: 2 tests in [`shell_integration.rs`](../../crates/made-platform/tests/shell_integration.rs) still fail without a runner locale (not fixed yet).

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
- The 10k-row selection guard (median of three p95 batches) is 16 ms on Windows debug runners and 5 ms on Linux and macOS arm64. macOS x86_64 has no timing assertion and only reports the p95 to stderr, because the hosted `macos-15-intel` runner swung from about 5.5 to 16 to 26 ms on unchanged code; the no-rebuild assertion still guards every platform. The product contract remains frame p95 below 16.667 ms. Treat the test guard as runner headroom, not the product budget.
- Sources: [`ui_model.rs`](../../crates/made-app/tests/ui_model.rs) `hover_until_text`, [`explorer_view.rs`](../../crates/made-app/src/explorer_view.rs), [Performance](../performance.md).

## Terminal view selection tests

- Selection and copy tests in `terminal_view.rs` must not run the user's real shell. Tests drain PTY output into the grid only through the first frame's resize pump (`TerminalPane::resize` pumps, then returns early on an unchanged size), so whether shell startup output lands in the grid depends on whether it arrives before that first frame; macOS `/bin/bash`'s leading blank line before its zsh notice can then blank the selected cells. An empty selection copies nothing (`copy_selection` returns `None`); that is correct product behavior, not the bug.
- Use the `spawn_quiet_pane` test helper: on unix it runs `/bin/sh` with `stty -echo`, waits for a `ready` marker, then clears screen and scrollback before the test feeds text. Windows still uses the native shell.
- In made-terminal (`alacritty_terminal` 0.26), ED 2 (`ESC[2J`) moves the screen into scrollback and a later taller resize pulls those lines back into view; send ED 3 (`ESC[3J`) as well to blank a test grid before a resize.
- Source/Test: [`terminal_view.rs`](../../crates/made-app/src/terminal_view.rs) `resize`, `copy_selection`, `spawn_quiet_pane`.

## Windows watched-root deletion

- Recursive deletion can fail with a sharing violation while a watcher handle is open even though `notify` uses `FILE_SHARE_DELETE`. The unavailable-root test renames the root on Windows and deletes recursively on Unix; this simulates loss of the watched path without weakening product cleanup.
- Test: [`workspace_tabs.rs`](../../crates/made-app/tests/workspace_tabs.rs).

## Windows CRLF checkouts

- Windows CI runners check out files with CRLF line endings because the repository has no `.gitattributes`. Tests that scan source text for multi-line literals or `\n`-joined patterns must normalize `\r\n` or match line by line.
- The integration neutrality scans do the latter through a line-based `production_part`. Source: [`integration_modules.rs`](../../crates/made-app/tests/integration_modules.rs).

## CI invocation

- CI runs `cargo test --workspace --locked --no-fail-fast`, so all failing test binaries report in one run; fix the aggregate rather than assuming the first failure is the only one.
- `made-app` tests run on Windows only in the `windows-2025` entry of the `ci.yml` `cargo test --workspace` matrix. [`ci-windows.yml`](../../.github/workflows/ci-windows.yml) runs targeted package tests (platform contract, PTY, the hook crates) plus smoke, never `made-app` tests.
- Clippy runs only on Linux (`ubuntu-24.04` lint job). Locally on macOS, `cargo clippy --workspace --all-targets --locked -- -D warnings` fails with dead-code errors in [`ui_model.rs`](../../crates/made-app/tests/ui_model.rs) for items used only under `#[cfg(target_os = "linux")]`; that is a host difference, not a regression.
- Workflow: [`ci.yml`](../../.github/workflows/ci.yml).
