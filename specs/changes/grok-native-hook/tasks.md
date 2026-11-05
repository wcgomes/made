# Tasks: grok-native-hook

Readiness: `analyze` must pass before implementation; review runs `verify` against the persisted spec.

Task IDs are stable because `specs/changes/add-release-auto-update/tasks.md` cites them; where a later ID is listed earlier, the list order is the dependency order. Tasks 2.6 through 6.3 land in a single merge to main.

## 0. Verify Grok facts (0.1 gates 1.5, 2.5, 2.6 and 6.1; 0.2 gates 3.1, 3.3 and all remote work in phases 4 and 5; 0.3 gates 1.9)
- [x] 0.1 [P] Verify how Grok on Windows resolves a hooks.json `command` of `bin/made-grok-status` against `bin/made-grok-status.exe`, using real grok on a Windows runner when available, otherwise documented Grok evidence; record the result and the chosen branch in the decision index; with neither real grok nor documentation, select the generated Windows hooks.json branch (path: specs/changes/grok-native-hook/proposal.md)
- [x] 0.2 [P] Verify on Linux and macOS that `grok plugin install <dir> --trust` copies the package into the Grok home rather than linking to `<dir>`, and whether it preserves the executable mode of `hooks/bin/<hook>`, using real grok when available, otherwise documented Grok evidence; record both results; if it links, stop and return to `clarify`; if the mode is not preserved or the result is unknown, treat it as not preserved, select the local restore fallback, and stop remote work for `clarify` on SSH-08/SSH-10 (path: specs/changes/grok-native-hook/proposal.md)
- [x] 0.3 [P] Verify on each supported platform that Grok starts the hook command directly, so the hook's parent process is the Grok process; record the result; if a platform interposes a shell, return to `clarify` before 1.9; if it cannot be determined, record it as `UNVERIFIED` for 7.3 and proceed (path: specs/changes/grok-native-hook/proposal.md)

## Checkpoint
- [x] verify 0.1 to 0.3 results are recorded and each fallback branch is selected or ruled out; no gated task starts while its gate is open

## 1. Native hook crate (test-first)
- [x] 1.1 Add `crates/made-grok-hook` to the workspace members (path: Cargo.toml)
- [x] 1.2 Create the crate manifest with the `made-grok-status` binary, a library target, workspace version and only `serde_json` plus required platform dependencies (path: crates/made-grok-hook/Cargo.toml)
- [x] 1.3 Port `status_test.py` 1:1 as process-level tests: one object with stdin left open appends `sessionId`, `cwd`, `event`, `status` and exits 0 with empty stdout/stderr within 3s; the ten event→status cases; logged `pid` equals the test process id; `NotARealHook` appends nothing (path: crates/made-grok-hook/tests/status.rs)
- [x] 1.4 [P] Port the Windows chunk-reader test 1:1: a chunk source that yields one object and then neither bytes nor EOF returns that object, and `status_of` of `NotARealHook` is none (path: crates/made-grok-hook/src/lib.rs)
- [ ] 1.5 [P] Port the hooks.json matcher test 1:1 against the repository source and, when 0.1 selected generation, against the generated Windows variant (path: crates/made-grok-hook/tests/hooks_json.rs)
- [x] 1.6 Add contract tests beyond the port: snake_case field spellings; `stopHookActive` truthiness including `0` and `0.0` as falsy; `-`/case normalization; first-match rule order; exactly seven fields with the `at` format; absent text fields as empty strings; `GROK_HOME` unset and empty both default to `~/.grok`; missing log directory created; non-object, truncated, invalid, empty and never-completing input writes nothing and exits 0 after about 2s; unwritable log exits 0 with empty stdout; invalid UTF-8 decoded as replacement characters while a valid character split across reads stays intact; concurrent runs against one log on a local filesystem produce whole, non-interleaved lines (path: crates/made-grok-hook/tests/contract.rs)
- [x] 1.7 Run `cargo test -p made-grok-hook` and confirm 1.3 to 1.6 fail for the missing implementation (path: crates/made-grok-hook/tests/status.rs)
- [x] 1.8 Implement the chunk-fed first-value decoder over the accumulated buffer, field text rules, status rules and line builder (path: crates/made-grok-hook/src/lib.rs)
- [x] 1.9 Implement the reader-thread stdin read with the ~2s deadline, parent pid per platform, a single-write append to the log path, empty stdout and exit 0 on every path (path: crates/made-grok-hook/src/main.rs)

## Checkpoint
- [ ] verify scenarios `Every registered event maps to its status`, `Line carries exactly the seven fields`, `Standard input left open`, `Malformed, empty or late input writes nothing`, `Failures never deny` and `Concurrent hook runs append whole lines` map to passing tests in `crates/made-grok-hook` on Linux, macOS and Windows CI

## 2. Plugin source and staging
IDs 2.1, 2.3 and 2.4 moved to 6.1 to 6.3 so the switch lands after packaging.
- [ ] 2.2 [P] Set the source `version` to `0.1.0`, equal to `[workspace.package].version`; 2.6 writes the workspace version into the staged copy, equal to the source under REL-02 (path: plugins/grok-made/plugin.json)
- [ ] 2.5 Add a Unix-only (`#[cfg(unix)]`) staging-script test that stages a fixture binary and a fixture source tree with its own fixture `hooks.json`, independent of 6.1's source form, and asserts the exact file set (`plugin.json`, `README.md`, `hooks/hooks.json`, `hooks/bin/<hook>`), mode 755, no test or Python file, the staged `plugin.json` version equal to the workspace version, and the Windows `hooks.json` variant when 0.1 selected generation; confirm it fails (path: crates/made-grok-hook/tests/stage_script.rs)
- [ ] 2.6 Implement `stage-grok-plugin.sh <target> <hook-binary> <out>`: copy the source files and the hook, set mode 755, write the workspace version into the staged `plugin.json` (equal to the source under REL-02), and generate the Windows `hooks.json` when 0.1 selected it (path: .github/scripts/stage-grok-plugin.sh)

## Checkpoint
- [ ] verify 2.5 passes on Linux and macOS CI

## 3. Local install and development resolution
- [ ] 3.1 Add tests: development and release builds both resolve `<install_root>/plugins/grok-made`; a development build never falls back to the repository source, which is never passed to `grok plugin install`; a missing staged copy runs no install and records hooks unavailable; and, only when 0.2 selected the fallback, the installed hook mode is restored after install; confirm they fail (path: crates/made-app/tests/grok_plugin_install.rs)
- [ ] 3.2 Add the grok-made resolution at `<install_root>/plugins/grok-made` for development and release builds, without a repository fallback (path: crates/made-platform/src/lib.rs)
- [ ] 3.3 Use that resolution in the ensure flow, treat a missing package as install failure, and apply the 0.2 mode-restore fallback only when selected (path: crates/made-app/src/grok_plugin.rs)
- [ ] 3.4 [P] Update the platform contract fixture for the staged grok-made path (path: crates/made-platform/tests/contract.rs)
- [ ] 3.5 [P] Point the manual install at the staged `<install_root>/plugins/grok-made` path and describe the native hook (path: plugins/grok-made/README.md)

## Checkpoint
- [ ] verify scenarios `Missing or older plugin is installed`, `Same version skips install`, `Newer installed copy stays`, `Development run installs only the staged copy` and `Install failure leaves hooks unavailable` map to passing tests

## 4. Remote python3 removal and stage set
- [ ] 4.1 Update tests: the probe no longer looks up python3; the staged grok-made set is the 10-artifact list with the hook at mode 700; no `status.py` or `status_test.py`; confirm failure (path: crates/made-platform/tests/ssh.rs)
- [ ] 4.2 Drop the python3 lookup from the probe and update both stage-script file lists and the 700-mode rule to the native hook (path: crates/made-platform/src/ssh.rs)
- [ ] 4.3 [P] Update the 10-entry stage list (path: crates/made-app/src/remote_session.rs)
- [ ] 4.4 Update tests: hooks become ready with an equal-or-newer install regardless of python3; frozen `Capabilities.python3` is encoded false and ignored; confirm failure (path: crates/made-remote/tests/grok.rs)
- [ ] 4.5 [P] Update the wire test for the frozen false `python3` field (path: crates/made-remote/tests/protocol.rs)
- [ ] 4.6 Remove the python3 gate and the `python3-missing` install failure, and update the stage list (path: crates/made-remote/src/grok.rs)
- [ ] 4.7 [P] Stop python3 detection and encode the field false (path: crates/made-remote/src/main.rs)
- [ ] 4.8 [P] Stop python3 detection in the supervisor (path: crates/made-remote/src/supervisor.rs)
- [ ] 4.9 [P] Document `Capabilities.python3` as frozen, false and ignored (path: crates/made-remote/src/protocol.rs)
- [ ] 4.10 [P] Stop parsing python3 from probe output and encode false (path: crates/made-app/src/ssh_servers.rs)
- [ ] 4.11 [P] Update probe-parse tests (path: crates/made-app/tests/ssh_servers.rs)
- [ ] 4.12 Update diagnostics tests so no remote state mentions python3; confirm failure (path: crates/made-app/tests/diagnostics.rs)
- [ ] 4.13 Remove `REMOTE_PYTHON3_MISSING` and the `python3-missing` branch (path: crates/made-app/src/diagnostics.rs)
- [ ] 4.14 [P] Remove the `python3-missing` reason mapping (path: crates/made-app/src/app.rs)
- [ ] 4.15 [P] Remove the python3-missing install-failure sentence (path: crates/made-app/src/ui.rs)
- [ ] 4.16 [P] Remove the python3-missing row reference (path: crates/made-app/src/workbench.rs)
- [ ] 4.17 [P] Update python3 references in workspace-tab tests (path: crates/made-app/tests/workspace_tabs.rs)
- [ ] 4.18 [P] Update python3 references in UI-model tests (path: crates/made-app/tests/ui_model.rs)

## Checkpoint
- [ ] verify scenarios `Install idempotence, refusal and failure keep honest tiers`, `Remote Grok diagnostics stay honest and hidden until revealed` and `Consented install enables hook activity` (no-python3 variant) map to passing tests, and `grep -rn python3 crates --include=*.rs` shows only the frozen field, its tests and `crates/made-platform/tests/detach.rs`

## 5. Packaging and CI
Listed in dependency order: 5.1, 5.2, 5.7, 5.8, 5.3, 5.4, 5.9, 5.10, 5.5, 5.6.
- [ ] 5.1 Accept the hook binary as an argument, stage it through `stage-grok-plugin.sh` in the 10-file list, and record it in `manifest.json` and `SHA256SUMS` (path: .github/scripts/stage-companion.sh)
- [ ] 5.2 [P] Update the 10-file list and check the hook is present, non-empty, executable and hash-matched to the companion manifest (path: .github/scripts/assemble-desktop-package.py)
- [ ] 5.7 Inside `rust:1-bullseye`, run `cargo test -p made-grok-hook`, build `made-grok-hook` in release profile, apply `check-glibc-max.sh` 2.31 and the musl-interpreter and GUI-link checks to the hook, and pass it to `stage-companion.sh` (path: .github/scripts/build-linux-companion.sh)
- [ ] 5.8 [P] Run `cargo test -p made-grok-hook`, build the hook in release profile, require minos 12.0 and no GUI framework links, and pass it to `stage-companion.sh` (path: .github/scripts/build-macos-companion.sh)
- [ ] 5.3 Use the updated build scripts for all four remote targets and replace the `status.py` mode checks with the hook (path: .github/workflows/ci-ssh-remote.yml)
- [ ] 5.4 Build the host-target hook and stage the desktop `plugins/grok-made` through `stage-grok-plugin.sh`, replacing the `status.py` checks (path: .github/workflows/ci-artifacts.yml)
- [ ] 5.9 Add an env-gated packaged-layout test: given `MADE_PACKAGED_HOOK` pointing at the staged hook and a PATH with no `python`/`python3`, spawn the hook as a child with one event and stdin left open, and assert one line, `pid` equal to the test process id, exit 0 and empty stdout; skipped when the variable is unset (path: crates/made-grok-hook/tests/packaged.rs)
- [ ] 5.10 Run 5.9 against the packaged `plugins/grok-made` on every matrix target (macOS arm64, macOS x64, Linux x64, Windows x64) with Python removed from PATH (path: .github/workflows/ci-artifacts.yml)
- [ ] 5.5 [P] Run `cargo test -p made-grok-hook` on Windows (path: .github/workflows/ci-windows.yml)
- [ ] 5.6 [P] Run `cargo test -p made-grok-hook` on both macOS targets (path: .github/workflows/ci-macos.yml)

## Checkpoint
- [ ] verify scenarios `Shipped package matches the host` and `No Python on the machine` (packaged hook, pid and no-Python run on all four local targets via 5.10), `Staged hook matches each server's target` (hook built, checked and manifest-recorded per remote target) and SSH-06 `Approved companion update is verified, swapped and reused` (10 artifacts) pass in CI

## 6. Cutover
- [ ] 6.1 Point all nine registrations at `bin/made-grok-status`, keeping events, matchers and the 5-second timeout, in the form 0.1 selected (path: plugins/grok-made/hooks/hooks.json)
- [ ] 6.2 [P] Delete the Python hook (path: plugins/grok-made/hooks/bin/status.py)
- [ ] 6.3 [P] Delete the Python test after 1.3 to 1.5 pass (path: plugins/grok-made/hooks/status_test.py)

## Checkpoint
- [ ] verify no file under `plugins/grok-made` is Python, `grep -rn 'status\.py\|status_test' .github crates plugins tests` is empty, and phases 1 to 5 still pass

## 7. Acceptance
- [ ] 7.1 Replace `grok_install_python3_missing` with a no-python3 consented install that succeeds, drop the python3 detection variants, and add a scenario that runs the installed native hook on a PATH without python3 and ingests its real line (path: tests/ssh/parity.py)
- [ ] 7.2 [P] Update harness self-tests for the removed python3 variants (path: tests/ssh/test_harness.py)
- [ ] 7.3 Run native real-grok acceptance on macOS arm64, macOS x64, Linux x64 and Windows x64 (install from the packaged layout with no Python, start a session, prompt, question, stop, end, and check dots, explorer follow and the logged pid against the Grok process) and on one remote per supported target; record per platform the evidence or an `UNVERIFIED` label per SSH-09 evidence rules, including any 0.3 `UNVERIFIED` result (path: tests/e2e/grok_native_hook_validation.md)

## Checkpoint
- [ ] verify scenarios `No Python on the machine`, `Session start and prompt move the explorer immediately`, `Question and permission show attention`, `Turn end and session end`, `Parallel sessions stay distinct`, `Consented install enables hook activity`, `Staged hook matches each server's target`, `Two tabs on one server do not leak Grok sessions` and `Disconnect, reconnect and stale Grok state` pass in the loopback grok-parity job or 7.3, with `UNVERIFIED` labels kept for combinations not executed
- [ ] verify `cargo test --workspace --locked` passes and every spec scenario maps to a passing test or a labeled `UNVERIFIED` combination

## Coverage map
- `Grok status hook`: 0.1, 0.3, 1.1 to 1.9, 5.9, 5.10, 6.1 to 6.3, 7.1, 7.3
- `Shipped Grok plugin install`: 0.1, 0.2, 2.2, 2.5, 2.6, 3.1 to 3.5, 5.4, 5.9, 5.10, 7.3
- `Remote OpenCode session and cwd follow (SSH-06)`: 4.1, 4.2, 4.3, 5.1, 5.2, 5.3, 5.7, 5.8
- `Local preservation and truthful parity evidence (SSH-09)`: 4.1, 4.2, 4.4 to 4.11, 7.1 to 7.3
- `Remote Grok Build session follow and tab parity (SSH-10)`: 0.2, 4.4, 4.6, 4.12 to 4.18, 5.1, 5.3, 5.7, 5.8, 7.1 to 7.3
