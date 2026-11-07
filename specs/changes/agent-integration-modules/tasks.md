# Tasks: agent-integration-modules

Readiness: `analyze` must pass before implementation; review runs `verify` against the persisted spec.

Every implementation handoff carries `Contract ref: specs/changes/agent-integration-modules/`. This is a behavior-preserving refactor, so "test-first" means: write the characterization, pin, scan or golden test; confirm it passes on today's code (baseline) or fails locally for the expected structural reason (interface or scan not yet present); commit it together with the task that makes it pass, so no commit on the branch has a failing test. Existing tests may change only imports, accessors, constructors, `include_str!` paths and added scanned sources, or as AIA-09 requires (AIA-01); a task that needs any other test edit stops and returns to the coordinator.

Gates (every checkpoint): `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --locked -j 3 -- -D warnings`; `cargo clippy -p made-platform --all-targets --locked --features update-test-seam -j 3 -- -D warnings`; `cargo test --workspace --locked --no-fail-fast -j 3 -- --test-threads=1`; `for t in .github/scripts/test_*.py; do python3 "$t" -v || exit 1; done`. Compile-green check for atomic groups: `cargo check --workspace --all-targets --locked -j 3`. Merge gate of each step: all PR CI workflows green (Linux, macOS x64 and arm64, Windows build and test, ConPTY and app smoke, Wayland/X11 smoke, SSH companions and loopback smoke). Where the `x86_64-pc-windows-gnu` target is installed, `cargo check -p made-platform --no-default-features --target x86_64-pc-windows-gnu` must show only the six pre-existing `detach.rs` warnings, and no new warning anywhere.

Conventions:
- **No lint suppression:** no `#[allow]`, `#[expect]` or `-A` flag is added anywhere; a warning is resolved by changing the code. Transitional code is designed so it triggers no lint (public accessors, `pub use` re-exports, upper-case `const` values).
- **Atomic group:** a set of tasks that cannot compile until its last task (a type used across many files changes). It is one commit or a commit series squashed before merge, and it has its own compile-green and test-green checkpoint. Groups are marked `ATOMIC`; every other task is green on its own.
- **Characterization adapter:** every phase 0 test keeps its expectations outside one block delimited by `// BEGIN characterization adapter` and `// END characterization adapter`. The adapter check is per block: only lines between the markers may differ from the phase 0 commit, and every line of the phase 0 test outside the markers is identical. In shared test files (`diagnostics.rs`, `workspace_tabs.rs`), edits outside the phase 0 block are checked separately and must be path-only (imports, accessors, constructors).
- **Identity references in phase 0 tests:** every integration-identity reference in a phase 0 test (terminal targets, integration lists, detection setup, root sources, row or field names) lives inside that test's characterization adapter block, so identity changes touch only adapter blocks. `smoke_markers.rs` has an adapter block like the other phase 0 files.
- **Remote-path markers:** remote code inside core files is delimited by `// BEGIN remote-path (AIA-02)` and `// END remote-path`; the whole files `ssh_servers.rs`, `remote_workspace.rs` and `remote_session.rs` are remote paths. Neutrality scans exempt exactly these.
- **Transitional-shim markers:** every transitional accessor or `pub use` re-export at an old path is delimited by `// BEGIN transitional-shim (AIA-11)` and `// END transitional-shim`. Neutrality scans exempt these regions until step 3 removes them; task 3.37 asserts none remain.
- **Inventory tasks:** each per-integration state group starts with a grep of that integration's field and method names across `crates/made-app/src` and `crates/made-app/tests`; the reader list found at `0b57210` is written into the task, and the implementer updates it in place if the rebased tree differs.
- **Evidence:** baselines, decisions settled by tests, CI run ids and the scenario-to-test map go to the `## Evidence` section of `proposal.md`, not to the decision index.
- **Scratch-branch checks** (revert checks) use path `n/a (PR description)` and record their result in the PR description.

## Overlap and sequencing
- PR wcgomes/made#6 (`claude-code-integration`) must merge to `main` before the step 1 PR merges; rebase this branch on `main` after it does. Phase 0 tests may be written before that.
- `grok-native-hook` is implemented but not archived; step 4 edits `crates/made-grok-hook` internals only and must keep its tests, `hooks.json` handling, staging and packaged-layout checks unchanged.
- Files shared with active changes (`add-release-auto-update`, `claude-code-integration`): `Cargo.toml`, `Cargo.lock`, `crates/made-platform/src/lib.rs`, `crates/made-app/src/{app,ui,diagnostics,workbench}.rs`. No CI workflow or script change is planned; if one turns out to be needed, stop and report.
- PR map: phase 0 + phase 1 (1a and 1b) = step 1 PR; phase 2 = step 2 PR; phase 3 = step 3 PR; phase 4 = step 4 PR; phase 5 = final rename and docs PR.

## 0. Baselines on today's code (first commits of the step 1 PR; tests only, green)
- [x] 0.1 Follow branch-table characterization behind a characterization adapter: every row of design.md "Follow policy" for Claude Code and Grok (usable, unusable, release; session channel holds, higher directory holder holds, no higher-ranked holder, nothing held), asserting update kind, rejection detail, `root_switched`, generation, effective root source and retained directories; plus the release orders OpenCode, Claude Code, Grok and OpenCode, Grok, Claude Code [P] (path: crates/made-app/tests/follow_characterization.rs)
- [x] 0.2 Launch-environment and log-order matrix behind a characterization adapter: shell, direct OpenCode, direct Grok, direct Claude Code and every remote target, with and without a shipped Claude Code plugin and a spool, first spawn and fallback-directory retry; record program, arguments, variables set, variables removed (`MADE_CLAUDE_SPOOL`, `SHELLOPTS`, `BASH_ENV`, `ENV`) and the diagnostics log lines in order; assert explicitly that the shell target has no program arguments, so typing `grok` in a shell tab gets no `dashboard` argument [P] (path: crates/made-app/tests/launch_env_characterization.rs)
- [x] 0.3 Behind a characterization adapter block added to the file: copied-report golden text for fixed snapshots (rows before detection, not on PATH, starting, ready, degraded, remote Grok and remote Claude Code rows) and row pills and sentences before and after detection, compared byte for byte [P] (path: crates/made-app/tests/diagnostics.rs)
- [x] 0.4 Behind a characterization adapter: pin the full smoke-layout markers line format as the fixed template `version={} opencode_plugin={} grok_status={} companions={}/4 askpass={} updates={} update_repo={} update_seam={}` (field names, order and separators exact, slots filled from a fixed packaged layout), with no Claude Code field [P] (path: crates/made-app/tests/smoke_markers.rs)
- [x] 0.5 Behind a characterization adapter block added to the file: tab presentation (label, mark, dots, tooltip title, accessible name) for every combination of OpenCode active, Claude Code present and Grok open, local and remote (remote Grok under SSH-10 before Claude Code); Grok-then-Claude same-tick ordering where it changes the result [P] (path: crates/made-app/tests/workspace_tabs.rs)
- [x] 0.6 Grok hook golden corpus: every event, malformed and truncated JSON, oversized input, split multibyte characters, slow and closed stdin; library-level records with a fixed clock and parent pid, binary-level records compared on every field but time and pid, file mode and exit status [P] (path: crates/made-grok-hook/tests/golden.rs)
- [x] 0.7 Claude Code hook golden corpus: the same corpus plus the 8 MiB cap and `}` gate cases, title sanitization and pid chain with an injected ancestry; spool bytes, mode and exit status [P] (path: crates/made-claude-hook/tests/golden.rs)
- [x] 0.8 Record baseline external dependency sets: `cargo tree -p made-grok-hook -e normal --prefix none` and the same for `made-claude-hook`, for a Unix and a Windows target (path: specs/changes/agent-integration-modules/proposal.md)

## Checkpoint
- [ ] gates pass on unchanged production code; 0.1 to 0.7 pass without any production edit
- [ ] verify baselines for: Branch table preserved; Launch environment matrix; Grok arguments; Copied report unchanged; Smoke markers unchanged; Rows before detection; Report order follows the list; Precedence by rank; Rank is independent of catalog order; Remote behavior unchanged; Golden corpus byte-identical; Diverged read limits kept

## 1. Step 1 (step 1 PR)

### 1a-G1 Identity (ATOMIC: not compile-green until 1.28)
No `CompatibleTui` shim: this group migrates every caller of `CompatibleTui` in `crates/made-app/src`, `crates/made-app/tests` and `crates/made-platform/tests` to `IntegrationRef`. Variants become the upper-case constants `integrations::OPENCODE`, `GROK` and `CLAUDE_CODE`; `CompatibleTui::ALL` becomes `PRODUCT` with the same catalog order; patterns on variants become comparisons of `IntegrationRef` values.
- [x] 1.1 Write pins and validation (confirm failing locally, commit with the group): every descriptor field of the product list (keys, programs `opencode`/`grok`/`claude`, labels, menu ids, scope ids and names, root-source ids, marks, ranks 100/200/300) and its order; list validation rejects duplicate keys, duplicate ranks and a session-channel owner that is not lowest-ranked; the product list holds exactly three entries (path: crates/made-app/tests/integration_modules.rs)
- [x] 1.2 Adapt detection tests to generic lookup over supplied program names with the same PATH fixtures, and paths tests to the generic plugin-package accessor; the `ALL`/`program`/`label` pins move to 1.1 (path: crates/made-platform/tests/contract.rs)
- [x] 1.3 Generic detection over program names; remove `CompatibleTui`; replace `grok_plugin_dir`/`claude_plugin_dir` and their `PathsInputs` fields with one plugin-package accessor (Claude Code composition stays here until 1b) (path: crates/made-platform/src/lib.rs)
- [x] 1.4 Interface skeleton: `Integration` with `descriptor()`, `Descriptor`, `Mark`, `Rank`, `IntegrationRef` (Eq/Hash by key), `IntegrationList`, `pub const PRODUCT` in catalog order, `pub const OPENCODE`, `GROK`, `CLAUDE_CODE`, validation; documented as compile-time, not a plugin API (path: crates/made-app/src/integrations/mod.rs)
- [x] 1.5 OpenCode descriptor and impl (identity only: mark `opencode.svg`, rank 100) (path: crates/made-app/src/integrations/opencode/mod.rs)
- [x] 1.6 Grok descriptor and impl (identity only: mark `grok.svg`, rank 300, `launch_args = ["dashboard"]`) (path: crates/made-app/src/integrations/grok/mod.rs)
- [x] 1.7 Claude Code descriptor and impl (identity only: mark `claude-code.svg`, rank 200) (path: crates/made-app/src/integrations/claude_code/mod.rs)
- [x] 1.8 `TerminalTarget::Tui(IntegrationRef)`; `pty_options` from descriptor `program` and `launch_args`; labels unchanged; remote arms wrapped in remote-path markers (path: crates/made-app/src/tui.rs)
- [x] 1.9 Remove `AgentIntegration`; keep `pub const V1_SCOPE` with its boolean fields and define `agent_integrations: PRODUCT` (field type `IntegrationList`) (path: crates/made-app/src/integration_status.rs)
- [x] 1.10 Module declarations; remove the `CompatibleTui` re-export (path: crates/made-app/src/lib.rs)
- [x] 1.11 Adapt the `PathsInputs` construction (path: crates/made-app/src/terminal_view.rs)
- [x] 1.12 Detection results mapped back to `IntegrationRef`; `compatible_tuis` options over `IntegrationRef` (phases unchanged) (path: crates/made-app/src/app.rs)
- [x] 1.13 `+` menu rows, ids and marks from descriptors (path: crates/made-app/src/ui.rs)
- [x] 1.14 Replace `CompatibleTui` variant patterns with `IntegrationRef` comparisons, mechanical and removed in G3 (path: crates/made-app/src/workbench.rs)
- [x] 1.15 Replace remaining `CompatibleTui` uses in Grok code, mechanical (path: crates/made-app/src/grok_plugin.rs)
- [x] 1.16 Path-only migration from `CompatibleTui` to `IntegrationRef` constants [P] (path: crates/made-app/tests/ui_model.rs)
- [x] 1.17 Path-only migration [P] (path: crates/made-app/tests/launch_workspace.rs)
- [x] 1.18 Path-only migration [P] (path: crates/made-app/tests/workspace_tabs.rs)
- [x] 1.19 Path-only migration [P] (path: crates/made-app/tests/claude_follow.rs)
- [x] 1.20 Path-only migration [P] (path: crates/made-app/tests/claude_live.rs)
- [x] 1.21 Path-only migration [P] (path: crates/made-app/tests/claude_privacy.rs)
- [x] 1.22 Path-only migration [P] (path: crates/made-app/tests/grok_plugin_install.rs)
- [x] 1.23 Rewrite for the new scope shape: the set assertion derives from `V1_SCOPE.agent_integrations` mapped to `(scope_id, scope_name)` and compared with `PRODUCT`; the verbatim `opencode-v2`/`OpenCode v2`, `grok-build`/`Grok Build`, `claude-code`/`Claude Code` pin stays; index asserts compare with `OPENCODE`, `GROK`, `CLAUDE_CODE`; `const` asserts unchanged; `include_str!` kept for the fixed core files, plus a recursive `std::fs::read_dir` walk of `CARGO_MANIFEST_DIR/src/integrations` asserting at least three integration modules read and scanned for forbidden surfaces (path: crates/made-app/tests/scope_boundary.rs)
- [x] 1.24 Accessor adaptation: `integration.id()` becomes `integration.descriptor().scope_id`; expected `["opencode-v2", "grok-build", "claude-code"]` and `const` asserts unchanged (path: crates/made-app/tests/degraded.rs)
- [x] 1.25 Adapter-block adaptation only (path: crates/made-app/tests/launch_env_characterization.rs)
- [x] 1.26 Identity references: none expected (the follow tests use the coordinator API, which changes in G2); verify with `grep -nE 'CompatibleTui|AgentIntegration|V1_SCOPE' crates/made-app/tests/follow_characterization.rs` returning nothing, or only lines inside the adapter block, and record the result in the PR description (path: crates/made-app/tests/follow_characterization.rs)
- [x] 1.27 Adapter-block migration of the phase 0 block's identity references (detection setup and integration lists) to `IntegrationRef`; nothing outside the block changes (path: crates/made-app/tests/diagnostics.rs)
- [x] 1.28 Adapter-block migration of identity references (detection setup) to `IntegrationRef`; nothing outside the block changes (path: crates/made-app/tests/smoke_markers.rs)

#### Checkpoint G1
- [ ] compile-green; then gates pass; adapter checks pass for `launch_env_characterization.rs`, `diagnostics.rs` and `smoke_markers.rs`, and the 1.26 grep is clean; no `CompatibleTui` identifier remains in the workspace
- [ ] verify: List order is catalog order; Product list is exact; Duplicate rank rejected; A list change trips the pin; Moved sources stay scanned; Grok arguments; Launch environment matrix

### 1a-G2 Follow (ATOMIC: not compile-green until 1.36)
- [x] 1.29 `FollowKind`, `FollowPolicy`, `OnUnusable`, `WhileOutranked` and `Integration::follow()` (path: crates/made-app/src/integrations/mod.rs)
- [x] 1.30 Ranked directory holders replacing `grok_root`/`claude_root`; generic `follow_directory` encoding each policy row in its declared check order; `release_to_fallback` and `follow_shell_cwd` by rank; `RootSource::{Shell, Agent(IntegrationRef)}` with `as_str` from `root_source_id`; `is_agent_owned` = session channel owns the root; `#[doc(hidden)] with_integrations(list)` (path: crates/made-app/src/session_follow.rs)
- [x] 1.31 Claude Code policy `{ Reject("Claude Code directory is not usable"), Retain, Retain }` (path: crates/made-app/src/integrations/claude_code/mod.rs)
- [x] 1.32 Grok policy `{ Ignore, Ignore, Retain }` (path: crates/made-app/src/integrations/grok/mod.rs)
- [x] 1.33 OpenCode `FollowKind::SessionChannel` (path: crates/made-app/src/integrations/opencode/mod.rs)
- [x] 1.34 Follow call sites use `follow_directory(entry, report)`; `RootSource` comparisons by entry (path: crates/made-app/src/workbench.rs)
- [x] 1.35 Adapter-block adaptation only (path: crates/made-app/tests/follow_characterization.rs)
- [x] 1.36 Path-only adaptation (`RootSource` variants via entries; `as_str` expectations unchanged) (path: crates/made-app/tests/session_follow.rs)

#### Checkpoint G2
- [x] compile-green; then gates pass; adapter check passes for `follow_characterization.rs`
- [x] verify: Branch table preserved; Precedence by rank; Rank is independent of catalog order; Claude Code rejects an unusable directory; Grok ignores an unusable directory silently; Claude Code retains under OpenCode; Grok is ignored under OpenCode; Grok retains under Claude Code

### 1a-G3a State skeleton (additive, each task green)
- [x] 1.37 `AppState`, `TabState`, `Slots<T>`, `EnvPolicy`, `EnvContribution`, `TabReport`, `AppInit`, `TabInit`, `TabIo`, `LaunchCtx`, `AppCtx`; `Integration::{env_policy, new_app_state, new_tab_state, polls_remote_tabs}` with defaults returning `None` or empty, so every integration still has no slot state (path: crates/made-app/src/integrations/mod.rs)
- [x] 1.38 Workbench gains `Slots<Box<dyn TabState>>` and its list, filled from `new_tab_state` (all `None` for now), and the generic loops at today's positions: `prepare_launch` and `on_spawned` in `start_terminal` and the retry, `poll` with report application and `on_follow_outcome` where `poll_grok_hooks`/`poll_grok_directory`/`poll_claude` run, the remote-tab poll in `tick_remote`, and teardown; all explicit integration code unchanged, so behavior is identical (path: crates/made-app/src/workbench.rs)
- [x] 1.39 App model gains `Slots<Box<dyn AppState>>` and its list, the `on_detection` and `poll` loops (no-ops for now), and `#[doc(hidden)] AppModel::with_integrations(list)` passing the list to every `Workbench`; explicit integration code unchanged (path: crates/made-app/src/app.rs)

#### Checkpoint G3a
- [x] gates pass; phase 0 tests unchanged
- [x] verify: Existing suites stay the acceptance gate; Launch environment matrix

### 1a-G3b Claude Code state (ATOMIC: not compile-green until 1.46)
- [x] 1.40 Inventory (re-run and update this line if the rebased tree differs). At `0b57210`: Workbench fields `claude`, `claude_plugin`, `claude_phase`; methods `poll_claude`, `prepare_claude_launch`, `open_claude_spool`, `install_claude`, `follow_claude_path`; public accessors and their readers in `src`: `claude_tab` (ui.rs), `claude_diagnostics` (ui.rs), `set_claude_phase` (app.rs), `set_remote_claude_detection` (app.rs, remote path); AppModel field `claude_phase` and `initial_claude_phase` (app.rs only); test readers of `claude_tab`: claude_live.rs, claude_privacy.rs, grok_plugin_install.rs, launch_workspace.rs, ui_model.rs, workspace_tabs.rs. Re-run at G3a (`5e03fff`): the same set, plus `claude_diagnostics` read inside `workbench.rs` (snapshot), the `RemoteTab` field `claude_detected` (read by `claude_diagnostics`, written by the probe in `workbench.rs` and by `set_remote_claude_detection`), the unit tests in `workbench.rs` (`claude`, `claude_plugin`, `poll_claude`, `with_claude_launch_env`) and in `tui.rs` (`loads_claude_plugin`) (path: specs/changes/agent-integration-modules/tasks.md)
- [x] 1.41 Claude Code `TabState`: `ClaudeRuntime`, the per-tab `claude_plugin` and `claude_phase`, and `poll_claude`, `prepare_claude_launch`, `open_claude_spool`, `install_claude` and the `follow_claude_path` bookkeeping moved wholesale (logic unchanged) as `prepare_launch`, `on_spawned`, `poll`, `on_follow_outcome`, `teardown`; lazy remote-tab creation (path: crates/made-app/src/integrations/claude_code/tab.rs)
- [x] 1.42 Claude Code `AppState`: `claude_phase`, `initial_claude_phase` and the Claude Code detection branch moved wholesale from `app.rs`, still calling the Claude Code diagnostics functions at their current path (path: crates/made-app/src/integrations/claude_code/diagnostics.rs)
- [x] 1.43 Claude Code `EnvPolicy { shell, own_direct, strip: ["MADE_CLAUDE_SPOOL"] }`, `polls_remote_tabs = true`, state constructors (path: crates/made-app/src/integrations/claude_code/mod.rs)
- [x] 1.44 Remove the Claude Code fields and methods; transitional accessors `claude_tab`, `claude_diagnostics`, `set_claude_phase` delegating to the slot, inside transitional-shim markers; `set_remote_claude_detection` delegates to the slot inside its remote-path region (path: crates/made-app/src/workbench.rs)
- [x] 1.45 Remove the AppModel Claude Code field and detection branch (now `on_detection`); Grok and OpenCode code still explicit (path: crates/made-app/src/app.rs)
- [x] 1.46 `loads_claude_plugin` callers replaced by the generic `EnvPolicy` predicate (the env code itself moves in 1b) (path: crates/made-app/src/tui.rs)

#### Checkpoint G3b
- [x] compile-green; then gates pass; adapter checks pass for every phase 0 block
- [x] verify: Launch environment matrix; Branch table preserved; Remote behavior unchanged; Existing suites stay the acceptance gate

### 1a-G3c Grok state (ATOMIC: not compile-green until 1.52)
- [x] 1.47 Inventory (re-run and update if the rebased tree differs). At `0b57210`: Workbench fields `grok_checked_at`, `grok_log_offset`, `grok_sessions`, `grok_closed`, `grok_provisional_id`, `grok_provisional_cwd`, `grok_restore_root`, `grok_file_newest_id`, `grok_diagnostics`; methods `poll_grok_hooks`, `poll_grok_directory`, `follow_grok_path`, `grok_follow_running` and the provisional restore at `workbench.rs:4290`; public accessors and readers in `src`: `set_grok_diagnostics` (app.rs, diagnostics.rs), `grok_diagnostics` (ui.rs), `grok_tab_activities` (ui.rs), `diagnostics_snapshot` (ui.rs, reads the Grok row); AppModel `grok_phase`, `grok_plugin_receiver`, `note_grok_hooks_unavailable`, `poll_grok_plugin_ensure`, `hold_grok_diagnostics` and `hold_workspace_grok_running` (test readers: workspace_tabs.rs, ui_model.rs) (path: specs/changes/agent-integration-modules/tasks.md)
- [x] 1.48 Grok `TabState`: the `grok_*` fields, `GrokLiveSession`, `poll_grok_hooks` then `poll_grok_directory` as one `poll`, `grok_follow_running`, the `follow_grok_path` bookkeeping and the provisional restore moved wholesale, still calling `grok_follow.rs` at its current path (path: crates/made-app/src/integrations/grok/tab.rs)
- [x] 1.49 Grok `AppState`: `grok_phase`, `grok_plugin_receiver`, the detection branch (`spawn_ensure`), `note_grok_hooks_unavailable`, `poll_grok_plugin_ensure` and the hold state moved wholesale from `app.rs` (phase types stay in `app.rs` until step 2) (path: crates/made-app/src/integrations/grok/diagnostics.rs)
- [x] 1.50 Grok state constructors, no env policy (path: crates/made-app/src/integrations/grok/mod.rs)
- [x] 1.51 Remove the Grok fields and methods; transitional accessors `set_grok_diagnostics`, `grok_diagnostics`, `grok_tab_activities` delegating to the slot, inside transitional-shim markers (path: crates/made-app/src/workbench.rs)
- [x] 1.52 Remove the AppModel Grok fields and detection branch; transitional `hold_grok_diagnostics` and `hold_workspace_grok_running` delegating to the slot, inside transitional-shim markers (path: crates/made-app/src/app.rs)

#### Checkpoint G3c
- [ ] compile-green; then gates pass; adapter checks pass for every phase 0 block
- [ ] verify: Branch table preserved; Grok arguments; Rows before detection; Existing suites stay the acceptance gate

### 1a-G3d OpenCode state (ATOMIC: not compile-green until 1.59)
- [x] 1.53 Inventory (re-run and update if the rebased tree differs). At `0b57210`: Workbench fields `bridge`, `config_guard`, the composed configuration and user-config capture state (`workbench.rs` ~1030-1160, 5265-5300, 5862-5885); public accessors and readers: `note_opencode_missing` (src: app.rs, integration_status.rs; tests: diagnostics.rs, degraded.rs), `smoke_summary` (app.rs); AppModel OpenCode-missing memory and `remember_opencode_missing`; staying core as session channel: `integration` (`IntegrationState`), `is_connected`, `focus_session` (path: specs/changes/agent-integration-modules/tasks.md)
- [x] 1.54 OpenCode `TabState`: `bridge`, `config_guard`, composed configuration and capture state moved into the module; the bridge-event drain runs at the session-channel tick position; the start and capture functions stay in `workbench.rs` and reach the state through the slot until step 3 (path: crates/made-app/src/integrations/opencode/tab.rs)
- [x] 1.55 OpenCode `AppState`: OpenCode-missing memory and `remember_opencode_missing` moved wholesale from `app.rs` (path: crates/made-app/src/integrations/opencode/diagnostics.rs)
- [x] 1.56 OpenCode `EnvPolicy { shell, own_direct }` for the composed configuration, state constructors (path: crates/made-app/src/integrations/opencode/mod.rs)
- [x] 1.57 Remove the OpenCode fields; the composition gating in `start_terminal` now comes only from `EnvPolicy` (the explicit target match goes); transitional `note_opencode_missing` inside transitional-shim markers (path: crates/made-app/src/workbench.rs)
- [x] 1.58 Remove the AppModel OpenCode field and detection branch; detection now runs only through `on_detection` (path: crates/made-app/src/app.rs)
- [x] 1.59 Adapter-block adaptation only, if the launch seam changed shape (path: crates/made-app/tests/launch_env_characterization.rs)

#### Checkpoint G3d
- [ ] compile-green; then gates pass; adapter checks pass for every phase 0 block
- [ ] verify: Launch environment matrix; Grok arguments; Branch table preserved; Remote behavior unchanged; Existing suites stay the acceptance gate

### 1a-G4 Presentation and diagnostics (ATOMIC: not compile-green until 1.70)
- [x] 1.60 `DiagnosticsDecl`, `RowReport`, `Presentation`; `AppState::{row, smoke_marker}`; `TabState::presentation` (path: crates/made-app/src/integrations/mod.rs)
- [x] 1.61 One neutral pill type with the same labels replacing `GrokDiagnosticsPill` and the `ClaudeDiagnosticsPill` alias; list-ordered `rows` replacing the four grok/claude fields; `to_text` iterating rows with declared keys; remote setters by key inside remote-path markers; `#[doc(hidden)]` rows builder from a list (path: crates/made-app/src/diagnostics.rs)
- [x] 1.62 Claude Code row `claude status`/`claude` with its initial state, presentation, no smoke marker (path: crates/made-app/src/integrations/claude_code/mod.rs)
- [x] 1.63 Grok row `grok status`/`grok` with `NOT_ON_PATH` initial state, presentation, marker `grok_status` (path: crates/made-app/src/integrations/grok/mod.rs)
- [x] 1.64 OpenCode `DiagnosticsDecl::Primary`, presentation (active predicate), marker `opencode_plugin` (path: crates/made-app/src/integrations/opencode/mod.rs)
- [x] 1.65 Rank-ordered branding through `presentation`; `WorkspaceTabInfo` carries the branding entry; About rows iterate the list; `#[doc(hidden)]` tab-info and About-row helpers taking a list; readers of `claude_tab`, `claude_diagnostics`, `grok_diagnostics`, `grok_tab_activities` and `diagnostics_snapshot`'s Grok row switch to list iteration; remote `+` items, server conditions, consent card and the remote branding exception inside remote-path markers (path: crates/made-app/src/ui.rs)
- [x] 1.66 Diagnostics push, repaint hash and smoke markers iterate the list; readers of `set_claude_phase` and `set_grok_diagnostics` switch to slots (path: crates/made-app/src/app.rs)
- [x] 1.67 Remove the transitional accessors whose readers moved in 1.65 and 1.66 (`claude_diagnostics`, `set_claude_phase`, `grok_diagnostics`, `set_grok_diagnostics`, `grok_tab_activities`); `claude_tab` and `note_opencode_missing` stay until step 3 because tests use them (path: crates/made-app/src/workbench.rs)
- [x] 1.68 Adapter-block adaptation of the phase 0 block; outside it, path-only adaptation of existing tests (`grok_pill`/`claude_pill` field reads become row lookups by key) (path: crates/made-app/tests/diagnostics.rs)
- [x] 1.69 Adapter-block adaptation of the phase 0 block; outside it, path-only (path: crates/made-app/tests/workspace_tabs.rs)
- [x] 1.70 Path-only adaptation (path: crates/made-app/tests/ui_model.rs)

#### Checkpoint G4
- [x] compile-green; then gates pass; per-block adapter checks pass for `diagnostics.rs` and `workspace_tabs.rs`, and their edits outside the blocks are path-only; `smoke_markers.rs` differs from phase 0 only inside its adapter block
- [x] verify: Copied report unchanged; Smoke markers unchanged; Rows before detection; Report order follows the list; Precedence by rank; Rank is independent of catalog order; Remote behavior unchanged

### 1a-G5 Neutrality, fake integration and list variation (each task green)
- [x] 1.71 Neutrality and structure scans: deny-list of legacy local-seam identifiers for Claude Code (`claude_root`, `ClaudeRuntime`, `loads_claude_plugin`, `poll_claude`, `prepare_claude_launch`, `self.claude`) in core files, exempting remote-path regions, transitional-shim regions and the files `ssh_servers.rs`, `remote_workspace.rs`, `remote_session.rs`; no core import of `integrations::<name>::` outside those exemptions; no integration folder references another's; new identifiers of this change contain no `FORBIDDEN_SURFACES` substring (for example `acp`); no direct dependency of any workspace crate provides dynamic loading (`libloading`, `dlopen`, WASM or script runtimes; transitive platform or graphics dependencies are out of scope); a PATH and environment naming another agent program leave the integration set unchanged (path: crates/made-app/tests/integration_modules.rs)
- [x] 1.72 Fake integration at rank 250 through `AppModel::with_integrations` and the hidden builders: menu rows with and without its detection; detection reaction recorded once in list order; launch arguments and variables per target; per-tab state created for declared targets, polled after the product entries, torn down once on close; follow and presentation (label, mark, dots, tooltip, accessible name) through OpenCode connect, disconnect, Claude Code live and exit with Grok reporting; About row and report lines after `claude`; smoke marker `fake_status=` after `grok_status=`; product list lacks it and no production source names it (path: crates/made-app/tests/fake_integration.rs)
- [x] 1.73 Fake variant at rank 150: owns root and presentation over Claude Code and Grok; holders release in rank order; product relative order unchanged (path: crates/made-app/tests/fake_integration.rs)
- [x] 1.74 List variation for menu and launch: product list, product plus fake, product without Claude Code; output contains exactly the list's entries (path: crates/made-app/tests/fake_integration.rs)
- [x] 1.75 List variation for follow and presentation (path: crates/made-app/tests/fake_integration.rs)
- [x] 1.76 List variation for diagnostics rows, About rows, copied report and smoke markers (path: crates/made-app/tests/fake_integration.rs)

#### Checkpoint 1a
- [ ] gates pass; all per-block adapter checks pass
- [ ] verify: One module per integration (state of all three, Claude Code seams); No cross-integration dependency; Compile-time only; Nothing outside the build adds an integration; Fake integration in the menu; Fake integration detection reaction; Fake integration launch environment; Fake integration state, polling and teardown; Fake integration follow and presentation at its rank; Fake integration above Claude Code; Fake integration diagnostics; Fake integration smoke marker; A list without an entry drops it from every seam; Fake absent from the product; A seam that ignores the list fails

### 1b Claude Code channel code relocation (each task green)
- [x] 1.77 Move `process_belongs_to_with` and helpers out of `grok_follow.rs` (path: crates/made-app/src/integrations/shared/process_ancestry.rs)
- [x] 1.78 Point Grok code at the shared helper (path: crates/made-app/src/grok_follow.rs)
- [x] 1.79 Move `claude_follow.rs` content (spool, `ClaudeTab`, ownership, liveness) importing the shared helper (path: crates/made-app/src/integrations/claude_code/follow.rs)
- [x] 1.80 Old module becomes a `pub use` re-export inside transitional-shim markers (path: crates/made-app/src/claude_follow.rs)
- [x] 1.81 Copy into the Claude Code module `CLAUDE_SPOOL_ENV`, `ClaudeLaunchEnv`, `claude_launch_env` from `tui.rs`, and `CLAUDE_CODE_PLUGIN_DIRS_ENV`, `claude_plugin_dirs_separator`, `compose_claude_plugin_dirs` and the safety check of `shipped_claude_plugin_dir` from `made-platform`; the `made-platform` originals stay until 1.85 (path: crates/made-app/src/integrations/claude_code/launch.rs)
- [x] 1.82 Remove the Claude Code env code; `pub use` re-export of `CLAUDE_SPOOL_ENV` and `ClaudeLaunchEnv` inside transitional-shim markers (path: crates/made-app/src/tui.rs)
- [x] 1.83 Composition tests copied from `made-platform/tests/contract.rs` with assertions unchanged, now exercising the Claude Code module (path: crates/made-app/tests/claude_launch.rs)
- [x] 1.84 Remove the copied composition tests (path: crates/made-platform/tests/contract.rs)
- [x] 1.85 Remove the Claude Code composition items, now unreferenced (path: crates/made-platform/src/lib.rs)
- [x] 1.86 Move `CLAUDE_GRACE`, `ClaudeDiagnosticsPhase`, `ClaudeDiagnosticsInputs`, `claude_diagnostics_row`, `remote_claude_diagnostics_row` next to the Claude Code `AppState` (path: crates/made-app/src/integrations/claude_code/diagnostics.rs)
- [x] 1.87 Remove the moved items; `pub use` re-export inside transitional-shim markers for test paths (path: crates/made-app/src/integration_status.rs)
- [x] 1.88 Privacy scan reads every file under `src/integrations/claude_code/` by a recursive `std::fs::read_dir` walk plus the two hook sources, and fails if the walk reads no file; forbidden-term list unchanged (path: crates/made-app/tests/claude_privacy.rs)
- [x] 1.89 Module declarations updated (path: crates/made-app/src/lib.rs)
- [x] 1.90 Revert check of the step 1 merge on a scratch branch, gates pass (path: n/a (PR description))

## Checkpoint
- [ ] gates pass; phase 0 tests differ only inside adapter blocks; other test diffs path-only or AIA-09; PR CI green (merge gate)
- [ ] verify: Existing suites stay the acceptance gate; Copied report unchanged; Smoke markers unchanged; Remote behavior unchanged; Source scans keep their coverage; One module per integration; No cross-integration dependency; Moved sources stay scanned; Green after each step; Revert of the latest step

## 2. Step 2: Grok Build channel code (step 2 PR; each task green)
- [x] 2.1 Extend the neutrality deny-list with Grok legacy identifiers (`grok_root`, `GrokLiveSession`, `poll_grok_`, `GrokDiagnosticsPhase`, `grok_row_for`), still exempting transitional-shim regions; confirm failing locally, commit with 2.10, the last removal (2.7, 2.8 and 2.10 remove the items) (path: crates/made-app/tests/integration_modules.rs)
- [x] 2.2 Move `grok_follow.rs` content (inventory, hook log reader, ownership) (path: crates/made-app/src/integrations/grok/follow.rs)
- [x] 2.3 Old module becomes a `pub use` re-export inside transitional-shim markers; deviation: the old `mod grok_follow;` was private and nothing outside the Grok module used it, so a re-export triggered `unused_imports` (no suppression allowed) and the file was deleted instead; 3.30 is therefore already done (path: crates/made-app/src/grok_follow.rs)
- [x] 2.4 Move `grok_plugin.rs` content (install worker, `spawn_ensure`) (path: crates/made-app/src/integrations/grok/plugin.rs)
- [x] 2.5 Old module becomes a `pub use` re-export inside transitional-shim markers; deviation as in 2.3: the file was deleted, so 3.31 is already done (path: crates/made-app/src/grok_plugin.rs)
- [x] 2.6 Move `GrokDiagnosticsPhase`, `Hold`, `RunningHold`, `grok_row_for`, `initial_grok_phase` from `app.rs` and `GrokDiagnosticsRow` from `diagnostics.rs` next to the Grok `AppState` (path: crates/made-app/src/integrations/grok/diagnostics.rs)
- [x] 2.7 Remove the moved items; `pub use` re-export inside transitional-shim markers for test paths (path: crates/made-app/src/app.rs)
- [x] 2.8 Remove `GrokDiagnosticsRow`; `pub use` re-export inside transitional-shim markers (path: crates/made-app/src/diagnostics.rs)
- [x] 2.9 `grok_mark` moved from `ui.rs` into the Grok module (path: crates/made-app/src/integrations/grok/mod.rs)
- [x] 2.10 Remove `grok_mark` and the Grok branding predicate (path: crates/made-app/src/ui.rs)
- [x] 2.11 Remote-file check: confirm `ssh_servers.rs`, `remote_workspace.rs` and `remote_session.rs` import no moved Grok item (none at `0b57210`) and record the result (path: specs/changes/agent-integration-modules/proposal.md)
- [x] 2.12 Module declarations updated (path: crates/made-app/src/lib.rs)
- [x] 2.13 Revert check of the step 2 merge on a scratch branch (path: n/a (PR description))

## Checkpoint
- [ ] gates pass; phase 0 and phase 1 tests unchanged; PR CI green (merge gate)
- [ ] verify: Existing suites stay the acceptance gate; Copied report unchanged; Smoke markers unchanged; Remote behavior unchanged; One module per integration (Claude Code, Grok Build); No cross-integration dependency; Grok ignores an unusable directory silently; Grok is ignored under OpenCode; Grok retains under Claude Code; Branch table preserved; Launch environment matrix; Grok arguments; Rows before detection; Report order follows the list; Green after each step; Revert of the latest step

## 3. Step 3: OpenCode channel code and shim removal (step 3 PR)
- [x] 3.1 Extend the neutrality deny-list with OpenCode legacy identifiers (`workspace_opencode_active`, `opencode_icon`, `remember_opencode_missing`, `note_opencode_`), still exempting transitional-shim regions; confirm failing locally, commit with 3.8 (path: crates/made-app/tests/integration_modules.rs) Done: the deny-list names `workspace_opencode_active`, `opencode_icon`, `remember_opencode_missing` and `note_opencode_`; the import scan now covers all three module paths (it failed on the `integrations::opencode` imports of `app.rs` and `workbench.rs` before 3.3 to 3.8).
- [x] 3.2 OpenCode composed-config contribution under `CONFIG_CONTENT_ENV` (path: crates/made-app/src/integrations/opencode/launch.rs) Done: `launch::contribution` and `launch::composes`; the "started with the original environment" check asks `Integration::configures_launch` (n4 of the step 1 review) instead of naming `CONFIG_CONTENT_ENV` in `workbench.rs`.
- [x] 3.3 Relocate the bridge-start wiring and user-config capture functions (`workbench.rs` ~1030-1160, 5265-5300, 5862-5885) into the OpenCode tab module (path: crates/made-app/src/integrations/opencode/tab.rs) Done as `TabState::start` (bridge start, user-configuration capture and composition) and `TabState::shutdown` / `after_shutdown` (bridge stop, capture verification, credential release); the bridge-event drain stays at the session-channel tick position in the core.
- [x] 3.4 Remove the relocated OpenCode functions (path: crates/made-app/src/workbench.rs)
- [x] 3.5 Move `SUPPORTED_OPENCODE_MAJOR`, `note_opencode_*`, `is_supported_opencode_version` (path: crates/made-app/src/integrations/opencode/diagnostics.rs)
- [x] 3.6 Remove the moved items, keeping `IntegrationState` and `ReasonCode` for the session channel (path: crates/made-app/src/integration_status.rs)
- [x] 3.7 `opencode_icon` and the active predicate in the OpenCode module (path: crates/made-app/src/integrations/opencode/mod.rs) Done: the predicate is `opencode::active`; `opencode_icon` no longer exists (the `+` menu mark comes from the descriptor since step 1).
- [x] 3.8 Remove `workspace_opencode_active` and `opencode_icon` (path: crates/made-app/src/ui.rs)
- [x] 3.9 Session channel bound to the list's `SessionChannel` entry, no OpenCode name in the coordinator (path: crates/made-app/src/session_follow.rs) Nothing left to change: `session_follow.rs` already finds the owner through `FollowKind::SessionChannel` and names no OpenCode item; the core reaches the owner through `session_channel_state`, `session_bridge`, `session_token` and `session_peer_supported`.
- [x] 3.10 Path-only import and accessor update to the modules' paths and typed helpers while the shims still exist (green), separate commit [P] (path: crates/made-app/tests/ui_model.rs)
- [x] 3.11 Path-only update [P] (path: crates/made-app/tests/workspace_tabs.rs)
- [x] 3.12 Path-only update [P] (path: crates/made-app/tests/launch_workspace.rs)
- [x] 3.13 Path-only update [P] (path: crates/made-app/tests/session_follow.rs) No change needed: the file imports no shim path.
- [x] 3.14 Path-only update [P] (path: crates/made-app/tests/diagnostics.rs)
- [x] 3.15 Path-only update [P] (path: crates/made-app/tests/degraded.rs)
- [x] 3.16 Path-only update [P] (path: crates/made-app/tests/remote_workspace.rs)
- [x] 3.17 Path-only update [P] (path: crates/made-app/tests/grok_plugin_install.rs)
- [x] 3.18 Path-only update [P] (path: crates/made-app/tests/claude_follow.rs)
- [x] 3.19 Path-only update [P] (path: crates/made-app/tests/claude_live.rs)
- [x] 3.20 Path-only update [P] (path: crates/made-app/tests/claude_privacy.rs)
- [x] 3.21 Path-only update [P] (path: crates/made-app/tests/claude_spool.rs)
- [x] 3.22 Path-only update [P] (path: crates/made-app/tests/claude_launch.rs)
- [x] 3.23 Path-only update [P] (path: crates/made-app/tests/opencode_tui.rs) No change needed: the file imports no shim path.
- [x] 3.24 Path-only update [P] (path: crates/made-app/tests/ssh_servers.rs) No change needed: the file imports no shim path.
- [x] 3.25 Adapter-block update if the block uses a re-export removed in 3.27 to 3.35, done while it still exists (green) [P] (path: crates/made-app/tests/follow_characterization.rs) No change needed: the adapter block imports no shim path.
- [x] 3.26 Adapter-block update if the block uses a re-export removed in 3.27 to 3.35, done while it still exists (green) [P] (path: crates/made-app/tests/launch_env_characterization.rs) No change needed: the adapter block imports no shim path.
- [x] 3.27 Remove the transitional `claude_tab` and `note_opencode_missing` accessors (tests already switched to the modules' paths and typed helpers in 3.10 to 3.26) (path: crates/made-app/src/workbench.rs)
- [x] 3.28 Remove every remaining `pub use` re-export shim (path: crates/made-app/src/lib.rs) Done: the `pub mod claude_follow;` declaration is gone; the other re-exports were removed with their files.
- [x] 3.29 Delete the shim file (path: crates/made-app/src/claude_follow.rs)
- [x] 3.30 Delete the shim file (already deleted in step 2, see 2.3; nothing to do) (path: crates/made-app/src/grok_follow.rs) Done in step 2.
- [x] 3.31 Delete the shim file (already deleted in step 2, see 2.5; nothing to do) (path: crates/made-app/src/grok_plugin.rs) Done in step 2.
- [x] 3.32 Remove the Claude Code diagnostics re-export (path: crates/made-app/src/integration_status.rs)
- [x] 3.33 Remove the Grok phase re-export and the transitional `hold_grok_diagnostics` and `hold_workspace_grok_running` accessors (path: crates/made-app/src/app.rs)
- [x] 3.34 Remove the Grok row re-export (path: crates/made-app/src/diagnostics.rs)
- [x] 3.35 Remove the Claude Code env re-export (path: crates/made-app/src/tui.rs)
- [x] 3.36 Confirm the fake integration tests are unchanged since step 1 and pass, and that no core file of design.md "Seam inventory" names the fake (path: crates/made-app/tests/fake_integration.rs)
- [x] 3.37 Scan asserting no `transitional-shim (AIA-11)` marker remains under `crates/made-app/src` (path: crates/made-app/tests/integration_modules.rs)
- [x] 3.38 Revert check of the step 3 merge on a scratch branch (path: n/a (PR description))

## Checkpoint
- [ ] gates pass; per-block adapter checks pass; OpenCode plugin `npm test` and `npm run typecheck` unchanged; PR CI green (merge gate)
- [ ] verify: Existing suites stay the acceptance gate; Copied report unchanged; Smoke markers unchanged; Remote behavior unchanged; Source scans keep their coverage; One module per integration (all three); No cross-integration dependency; Compile-time only; List order is catalog order; Nothing outside the build adds an integration; Product list is exact; Fake integration in the menu; Fake integration detection reaction; Fake integration launch environment; Fake integration state, polling and teardown; Fake integration follow and presentation at its rank; Fake integration above Claude Code; Fake integration diagnostics; Fake integration smoke marker; A list without an entry drops it from every seam; Fake absent from the product; Precedence by rank; Rank is independent of catalog order; Duplicate rank rejected; Claude Code retains under OpenCode; Grok is ignored under OpenCode; Branch table preserved; Launch environment matrix; Rows before detection; Report order follows the list; A seam that ignores the list fails; A list change trips the pin; Moved sources stay scanned; Green after each step; Revert of the latest step

## 4. Step 4: shared hook library (step 4 PR)
- [x] 4.1 Crate manifest: library only, `publish` from workspace, `serde_json` and the `windows-sys` Foundation/ToolHelp/Threading features under `cfg(windows)`, nothing else; the same commit adds the workspace member to `Cargo.toml` and the `Cargo.lock` entry required by `--locked` builds (path: crates/made-hook-common/Cargo.toml)
- [x] 4.2 Write unit tests for the shared items (chunk reader with deadline and poll slice, `format_timestamp`, `json_string`, `field_text`, `text_of`, panic-silent wrapper), ported from identical cases in both hooks; confirm failing locally, commit with 4.3 (path: crates/made-hook-common/tests/common.rs)
- [x] 4.3 Shared items: `Chunk`, `READ_DEADLINE`, `POLL_SLICE`, `spawn_stdin_reader`, `format_timestamp`, `json_string`, `field_text`, `text_of`, `run_silently`, Windows Toolhelp `parent_of` (path: crates/made-hook-common/src/lib.rs)
- [x] 4.4 Use and re-export the shared items, keeping `status_of`, `log_path`, `build_line`, `LOG_FILE` (path: crates/made-grok-hook/src/lib.rs)
- [x] 4.5 Use the shared reader, wrapper and Windows parent lookup; Unix `parent_pid` unchanged (path: crates/made-grok-hook/src/main.rs)
- [x] 4.6 Add the shared dependency and drop the direct `windows-sys` dependency (its only use, the Toolhelp lookup at `main.rs:88-92`, moved) (path: crates/made-grok-hook/Cargo.toml)
- [x] 4.7 Use and re-export the shared items, keeping `MAX_INPUT`, `MAX_RECORD`, `MAX_SPOOL`, `MAX_PIDS`, `MAX_TITLE_CHARS`, `Input`, `status_of`, `sanitize_title`, `build_record` (path: crates/made-claude-hook/src/lib.rs)
- [x] 4.8 Use the shared reader, wrapper and Windows parent lookup; spool, lock and Unix pid chain unchanged (path: crates/made-claude-hook/src/main.rs)
- [x] 4.9 Add the shared dependency; drop the direct `windows-sys` dependency (only use at `main.rs:256-260`, moved); `libc` stays (path: crates/made-claude-hook/Cargo.toml)
- [x] 4.10 Decide `read_event_with`: move as a parameterized shared function only if 0.6, 0.7 and both hooks' existing tests pass unchanged, else keep one copy per hook; record the outcome (path: specs/changes/agent-integration-modules/proposal.md)
- [x] 4.11 Add `made-hook-common/src/lib.rs` to the privacy scan's sources (no Grok hook source scan exists at `0b57210`, so none is extended) (path: crates/made-app/tests/claude_privacy.rs)
- [x] 4.12 Re-run the 0.8 `cargo tree` commands with the `made-hook-common` node filtered out and record that the external sets equal the baseline (path: specs/changes/agent-integration-modules/proposal.md)
- [x] 4.13 Read-only check that packaging and companion scripts need no change (they build `-p made-grok-hook` and `-p made-claude-hook` inside the workspace with `--locked`); stop and report if any does (path: .github/scripts/build-linux-companion.sh)
- [x] 4.14 Dispatch `ci-artifacts` on the branch, confirm packaged hook tests pass on all four targets, record the run id (path: specs/changes/agent-integration-modules/proposal.md)
- [x] 4.15 Revert check of the step 4 merge on a scratch branch (path: n/a (PR description))

## Checkpoint
- [ ] gates pass; `cargo test -p made-grok-hook -p made-claude-hook -p made-hook-common --locked` passes with 0.6 and 0.7 unchanged; Windows cross-check shows no new warning; PR CI and `ci-artifacts` green (merge gate)
- [ ] verify: Golden corpus byte-identical; Diverged read limits kept; Packaged hooks unchanged; Source scans keep their coverage; Existing suites stay the acceptance gate; Remote behavior unchanged; Green after each step; Revert of the latest step

## 5. Final step: scoped renames, wiki, ADR and verify (final PR)
- [ ] 5.1 Rename `push_grok_diagnostics` to a neutral name (path: crates/made-app/src/app.rs)
- [ ] 5.2 Rename `grok_status_pill_ui` to a neutral name (path: crates/made-app/src/ui.rs)
- [ ] 5.3 Rename `opencode_sessions_summary` to a neutral name (path: crates/made-app/src/ui.rs)
- [ ] 5.4 Grep audit for other core names that imply one integration while serving several; list findings in the PR without renaming beyond scope (path: n/a (PR description))
- [x] 5.5 Rule 4 names `made-hook-common` with the two hooks; rule 3 context updated; module table and ownership rows for `integrations/`, `session_follow.rs`, `integration_status.rs`; hook-crate rows' dependency columns updated for `made-hook-common` and the dropped direct `windows-sys` (path: wiki/architecture.md)
- [x] 5.6 Root precedence described by ranks and module references updated (path: wiki/runtime/terminal-first.md)
- [x] 5.7 New page "Adding an integration": the standard module structure, descriptor and trait methods, the one list line, rank choice, follow kinds and policy, launch policy, diagnostics row, assets, the fake-integration test as template, what stays outside the guarantee (remote/SSH, redaction list, packaging, spec change); links ADR 0009 (path: wiki/integration/adding-an-integration.md)
- [x] 5.8 Link the new page (path: wiki/integration/index.md)
- [x] 5.9 ADR 0009 with Status, Context, Decision, Consequences and Evidence: static trait-object list, ranks, declared follow policy, compile-time only and not a plugin system, AIA-04 exceptions including remote (path: wiki/decisions/0009-static-integration-modules.md)
- [x] 5.10 Index entry for ADR 0009 (path: wiki/decisions/index.md)
- [ ] 5.11 Map every spec.md scenario to its passing test (path: specs/changes/agent-integration-modules/proposal.md)

## Checkpoint
- [ ] gates pass; renames change no test expectation; PR CI green (merge gate)
- [ ] `verify` against spec.md passes, every scenario mapped to a passing test: Existing suites stay the acceptance gate; Copied report unchanged; Smoke markers unchanged; Remote behavior unchanged; Source scans keep their coverage; One module per integration; No cross-integration dependency; Compile-time only; List order is catalog order; Nothing outside the build adds an integration; Product list is exact; Fake integration in the menu; Fake integration detection reaction; Fake integration launch environment; Fake integration state, polling and teardown; Fake integration follow and presentation at its rank; Fake integration above Claude Code; Fake integration diagnostics; Fake integration smoke marker; A list without an entry drops it from every seam; Fake absent from the product; Precedence by rank; Rank is independent of catalog order; Duplicate rank rejected; Claude Code rejects an unusable directory; Grok ignores an unusable directory silently; Claude Code retains under OpenCode; Grok is ignored under OpenCode; Grok retains under Claude Code; Branch table preserved; Launch environment matrix; Grok arguments; Rows before detection; Report order follows the list; A seam that ignores the list fails; A list change trips the pin; Moved sources stay scanned; Golden corpus byte-identical; Diverged read limits kept; Packaged hooks unchanged; Green after each step; Revert of the latest step
- [ ] archive order noted for the archive gate: after `claude-code-integration` and `grok-native-hook` archive (their domains are the live contract this change preserves); no MODIFIED reconciliation needed
