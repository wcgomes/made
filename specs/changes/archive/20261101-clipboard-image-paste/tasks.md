# Tasks: clipboard-image-paste

Readiness: `analyze` must pass before implementation; review runs `verify` against the persisted spec.

> **Retrospective contract:** implementation and automated tests were delivered before this change folder was created. Checkboxes record the delivered state, and the scenario-to-test table below is the verification index. Native OS clipboard evidence on macOS, Linux X11/Wayland, and Windows remains pending and is intentionally unchecked.

## 1. Platform clipboard image API

- [x] 1.1 Add `ClipboardImage` with dimension/byte validation and pixel-redacted `Debug` (path: `crates/made-platform/src/lib.rs`)
- [x] 1.2 Add `Clipboard::get_image` with an `Ok(None)` default and implement it for `InMemoryClipboard` and `SystemClipboard` (path: `crates/made-platform/src/lib.rs`)
- [x] 1.3 Enable arboard `image-data` and add PNG-only `image` encoding (path: `crates/made-platform/Cargo.toml`)
- [x] 1.4 Add the bounded private PNG save helper with unique names, `0700`/`0600` on Unix, `create_new`, and `O_NOFOLLOW` (path: `crates/made-platform/src/lib.rs`)
- [x] 1.5 Cover image round-trip, redaction, validation, PNG signature/dimensions, private permissions, uniqueness, and absolute paths (path: `crates/made-platform/tests/contract.rs`)
- [x] 1.6 Add fixed retention constants and `prune_clipboard_images`, and prune best-effort after every successful save (path: `crates/made-platform/src/lib.rs`)
- [x] 1.7 Cover age-based deletion, the 64-file cap, preservation of the just-saved image, unrelated files and symlinks, and deletion failures (path: `crates/made-platform/tests/contract.rs`)

## Checkpoint

- [x] `cargo test -p made-platform --test contract clipboard_image` passes and the contract scenarios `Private image storage`, `Age-based cleanup`, `Count cap`, `Unrelated entries are preserved`, and `Cleanup failure does not block the paste` are covered

## 2. Terminal paste routing and fallback

- [x] 2.1 Add the `PasteMode` distinction and route `Cmd/Ctrl+V` to native paste and `Cmd/Ctrl+Shift+V` to path fallback (path: `crates/made-app/src/terminal_view.rs`)
- [x] 2.2 Forward `0x16` for native image paste, show the one-time fallback hint, and reset the hint on relaunch
- [x] 2.3 Save the image and paste the absolute path through the existing bracketed-paste path, refusing non-UTF-8 paths (path: `crates/made-app/src/terminal_view.rs`)
- [x] 2.4 Preserve text-only paste in both modes, prefer the image when both are present, and handle empty, unreadable, and oversized clipboards with notices and no panic (path: `crates/made-app/src/terminal_view.rs`)
- [x] 2.5 Cover key routing, native forwarding, path fallback, hint repeat/relaunch, text-only, image priority, image-read errors, empty clipboards, and oversized-image refusal with a fake platform and PTY (path: `crates/made-app/src/terminal_view.rs`)
- [x] 2.6 Run the retention prune when a workbench starts (path: `crates/made-app/src/workbench.rs`)

## Checkpoint

- [x] `cargo test -p made-app paste` and `cargo test -p made-app clipboard` pass; the `Native image paste`, `Native image paste hint`, `Path fallback`, `Text-only paste`, `Image wins over text`, `Empty or unreadable clipboard`, and `Oversized image` scenarios map to these tests

## 3. Documentation

- [x] 3.1 Document the user-visible chords and fallback behavior (path: `README.md`)
- [x] 3.2 Document the native-first flow, path fallback, and pending native evidence (path: `wiki/runtime/terminal-first.md`)
- [x] 3.3 Document private image storage, permissions, bounds, redaction, and fixed retention (path: `wiki/security.md`)
- [x] 3.4 Document the 7-day/64-file retention and opportunistic cleanup points (path: `wiki/runtime/terminal-first.md`)

## 4. Readiness and verification

- [x] 4.1 Run `analyze` against the persisted proposal, delta spec, design, and tasks; correct coverage or contradiction gaps
- [x] 4.2 Run `verify` and confirm every scenario maps to a passing automated test or an explicitly recorded evidence gap
- [x] 4.3a macOS native evidence: the user confirmed clipboard image paste works on macOS (2026-11-01).
- [ ] 4.3b Obtain Linux X11/Wayland and Windows native clipboard evidence; accepted for follow-up if issues appear.
- [x] 4.4 Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`

## Checkpoint

- [x] Automated scenarios pass and the persisted contract matches the delivered behavior
- [x] macOS native clipboard image paste confirmed by the user (2026-11-01)
- [ ] Linux X11/Wayland and Windows native clipboard evidence remains pending and is accepted for follow-up

## Scenario-to-test index

| Spec scenario | Automated evidence |
|---|---|
| Native image paste in an agent TUI | `native_image_paste_forwards_ctrl_v_and_hints_once` covers the forwarded byte and absence of a text payload; macOS user-confirmed 2026-11-01; Linux/Windows pending in 4.3b |
| Native image paste hint | `native_image_paste_forwards_ctrl_v_and_hints_once` and `relaunch_allows_the_native_image_paste_hint_again` |
| Path fallback | `path_image_paste_writes_a_bracketed_absolute_png_path` and `clipboard_image_save_is_private_unique_and_under_the_temp_directory` |
| Text-only paste remains unchanged | `text_only_clipboard_still_pastes_text_in_both_modes` |
| Image wins over text | `native_image_paste_forwards_ctrl_v_and_hints_once` and `path_image_paste_writes_a_bracketed_absolute_png_path` assert the text payload is absent |
| Empty or unreadable clipboard | `empty_and_unreadable_clipboards_show_a_notice_without_panicking` and `image_read_failure_falls_back_to_readable_text` |
| Oversized image | `path_fallback_refuses_an_oversized_image_without_writing` and `clipboard_image_validates_dimensions_and_byte_length` |
| Private image storage | `clipboard_image_save_is_private_unique_and_under_the_temp_directory` and `in_memory_clipboard_round_trips_images_and_redacts_pixels` |
| Age-based cleanup | `clipboard_image_retention_removes_images_older_than_the_limit`; workbench startup invokes the same pruner in `workbench.rs` |
| Count cap | `clipboard_image_save_prunes_to_the_retention_limit`, `clipboard_image_save_keeps_the_new_image_when_it_is_oldest`, and `clipboard_image_retention_prunes_old_and_excess_files` |
| Unrelated entries are preserved | `clipboard_image_retention_preserves_unrelated_entries` |
| Cleanup failure does not block the paste | `clipboard_image_retention_ignores_deletion_failures` (Unix) |

## Readiness (analyze)

- Status: PASS (2026-11-01).
- Normative guarantees live only in `spec.md`; `design.md` remains non-normative.
- Every requirement and scenario has task coverage, no `[NEEDS CLARIFICATION]` marker remains, and no contradiction was found between the proposal decisions, design, and tasks.
- Retrospective limitation: implementation preceded this change folder, so `analyze` could not gate execution; this is recorded rather than hidden. The retention addition was specified before its implementation.

## Verification (verify)

- Status: PASS_WITH_GAP (2026-11-01).
- `cargo fmt --all -- --check` — passed.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- `cargo test --workspace` — passed, 0 failures.
- `cargo test -p made-platform --test contract clipboard_image` — 9 passed, including age, count, just-saved, unrelated-entry, and deletion-failure coverage.
- `cargo test -p made-app paste` — 5 passed; `cargo test -p made-app clipboard` — 2 passed.
- macOS native clipboard image paste — user-confirmed 2026-11-01.
- Drift: none found between the persisted spec and the delivered code after adding the retention, relaunch-hint, and oversized-image tests. Linux X11/Wayland and Windows native clipboard evidence remains pending and is accepted for follow-up.
