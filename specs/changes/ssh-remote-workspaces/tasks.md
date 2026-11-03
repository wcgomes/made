# Tasks: ssh-remote-workspaces

Readiness: `analyze` must pass before implementation; review runs `verify` against the persisted spec.

Contract ref: `specs/changes/ssh-remote-workspaces/`. Phase 1 checkboxes are completed and stay checked. Later checkboxes stay unchecked until that behavior is actually done. Owners are work-package roles, not permission to spawn agents in this specification session. `[P]` means disjoint-file parallel work after the named dependency, not concurrent edits to shared files. New paths are intentional deliverables; existing paths were checked against current source. Every test-first task includes observing a behavioral assertion failure after minimal compiling scaffolding, before implementing its paired behavior; missing modules alone are not red-test evidence. A serially adds leaf-module exports/dependency wiring as owners make compiling files available; phase 3 audits/completes that wiring rather than preventing phase 2 test execution. Missing toolchain/target/runtime access is reported as unrun, never a pass. In-progress tmux-based drafts are not accepted behavior and must be replaced before their tasks can be checked.

## Ownership and frozen interfaces
| Unit / owner | Exclusive files | Inputs / outputs / dependency |
|---|---|---|
| A — contract/integration owner | All Cargo manifests and module declarations; `made-remote/src/protocol.rs`; `made-terminal/src/transport.rs`; `made-bridge/src/state.rs`; shared app Workbench/UI integration files in phase 4 | Freeze WireEnvelope/RemotePath/Operation/Error, POSIX snapshot validation, terminal/root/session command adapter seams first, then add only the authorized `TerminalOutput.done` field. A is the sole writer of shared wiring even while other units add new leaf files. |
| B — platform/SSH owner | `made-platform/src/ssh.rs`, `ssh_askpass.rs`, `shell_integration.rs`, `detach.rs`, `bin/made-askpass.rs`; platform SSH and detach tests; `made-remote/src/client.rs` | Uses A interfaces. Produces cancellable authenticated OpenSSH stdio/prompt stream, private staging, cwd-report sink, SSH-session detach, and remote protocol client. Platform public re-exports are integrated by A only. |
| C — remote-runtime owner | `made-remote/src/main.rs`, `supervisor.rs`, `filesystem.rs`, `integration.rs`, `clipboard.rs`, `git.rs`; companion runtime tests; `made-bridge/src/remote_rpc.rs` | Uses A protocol, platform PTY/detach/paths, explorer guards and BridgeHandle. Produces consented bootstrap/runtime, native PTY survival/lease, bounded VT resnapshot, guarded operations, and normalized session events. Does not call tmux. B/C may work concurrently against frozen interfaces; actual SSH proof waits for both. |
| D — desktop adapter owner | `made-app/src/ssh_servers.rs`, `remote_workspace.rs`; dedicated remote app model tests | Uses A protocol/terminal interfaces with fakes until B/C join. Produces registry and remote controller with authority/epoch/generation gating, no host fs reads. Existing Workbench/UI files remain A-owned, not parallel D edits. |
| E — test/package owner | `tests/ssh/`; new companion build workflow; packaging workflow only in phase 5 | Test harness can be authored in parallel after A. Real tests depend on B/C/D and integration. Packaging workflow edits wait until A has finished manifest wiring; no product writes from verification tasks. |

Wire interface: protocol major 1, <=256 KiB frames, <=64 KiB chunks, <=16 nonterminal requests and <=256 queued data chunks per tab, ten-second RPC deadlines, explicit cancellation. Envelope binds random tab capability + server authority + epoch + request id + root generation; RemotePath is lossless POSIX bytes, not host PathBuf. Closed operations and upload/terminal separation are specified in design.md. TerminalTransport supplies poll/write/resize/status/detach; explicit remote Close is distinct. RootResolver and SessionCommandSink are asynchronous authority-aware ports; local behavior stays default. Remote snapshots reuse bridge SessionSnapshot with explicit POSIX lexical validation and no host filesystem resolution. These phase-1 signatures stay frozen except the authorized `TerminalOutput` payload field below. Compatible implementation-local names may differ, not guarantees.

Native-supervisor delta:
- A follows `protocol.rs`, `made-terminal/src/transport.rs`, `made-bridge/src/state.rs`, and `made-platform/src/lib.rs` only as registrar, plus the desktop input gate in `terminal_view.rs`. Do not change `PROTOCOL_MAJOR`, add an `Operation` variant, or change `WireEnvelope`, `TerminalTransport` methods, `PathSemantics`, or `SessionSnapshot`. The only authorized payload change is `TerminalOutput { data: Chunk, done: bool }`, required, with no serde default. Keep `Capabilities.tmux`; producers encode false and nobody branches on it. Do not add a tmux operation or a VT-snapshot verb. Register `detach` in task 3.1. Do not edit `emulation.rs` to reach alacritty's private inactive grid.
- B follows `ssh.rs`, new `detach.rs`, and `client.rs`. Probe and stage scripts must not name, execute, or version-check tmux. Detach starts a new session, ignores SIGHUP, closes inherited stdio, and uses a private runtime directory. Unsupported remote families fail preflight; they do not fall back to tmux. When enqueueing terminal bytes, preserve `TerminalOutput.done`; do not collapse it into `PtyEvent::Data` and do not treat an empty poll as completion.
- C follows `main.rs` and `supervisor.rs` plus the listed runtime tests. The supervisor owns `PtySession`, the loopback bridge, watchers, and a `TerminalCore` with a 10,000-line bound. After `Attached`, it emits the resnapshot on `TerminalOutput` and sets `done: true` on the last resnapshot frame before any live byte. Later live frames use `done: false`. No tmux binary, socket, config, probe, or fallback.
- E follows `tests/ssh/harness.py` and `tests/ssh/parity.py`. Prerequisites exclude tmux. Do not install tmux or Rust on the remote or in a development container. A missing companion binary is unrun, not a pass.
- D has no wire-struct delta, must not add a tmux gate, and must not mark the terminal current before `TerminalOutput` `done: true`.

## 1. Contract seams and test-first foundation — A, sequential
- [x] 1.1 Write identity/path/frame/size/version/request/epoch/generation and hostile-response tests; observe failures before codec implementation. (path: crates/made-remote/tests/protocol.rs)
- [x] 1.2 Register the new UI-free companion member only; preserve existing members. (path: Cargo.toml)
- [x] 1.3 Define library/binary package dependencies on platform/explorer/bridge/terminal and serialization, no GUI/app dependency. (path: crates/made-remote/Cargo.toml)
- [x] 1.4 Define/export protocol and library module boundaries with redacted types; stage subsequent module registration through A. (path: crates/made-remote/src/lib.rs)
- [x] 1.5 Implement frozen bounded codec/closed operation DTOs and remote POSIX path type to pass 1.1, including fragmented/malformed frames and upload totals. (path: crates/made-remote/src/protocol.rs)
- [x] 1.6 Write remote POSIX versus native local snapshot validation tests on Windows/Unix; observe failures. (path: crates/made-bridge/tests/remote_paths.rs)
- [x] 1.7 Add explicit path-semantics validation without changing native-local default or protocol major 1 plugin fields. (path: crates/made-bridge/src/state.rs)
- [x] 1.8 Write transport-adapter conformance tests for local PTY parity, detach versus kill, bounded input and disconnect discard; observe failures. (path: crates/made-terminal/tests/transport.rs)
- [x] 1.9 Add narrow TerminalTransport with a local PtyProcess adapter; preserve existing process/drop semantics. (path: crates/made-terminal/src/transport.rs)
- [x] 1.10 Register the terminal transport module; publish no remote product policy here. (path: crates/made-terminal/src/lib.rs)
- [x] 1.11 Verify the headless platform build currently fails the no-desktop-dependency check, then introduce default-on desktop feature flags for clipboard/PNG/native backdrop without changing desktop defaults. (path: crates/made-platform/Cargo.toml)
- [x] 1.12 Gate desktop-only facilities and supply compatible unsupported headless APIs as needed; prove PTY/path/private-file/shell facilities build without desktop features and desktop tests still compile. (path: crates/made-platform/src/lib.rs)
- [x] 1.13 Disable platform desktop defaults on the terminal dependency edge so the companion cannot re-enable GUI/clipboard dependencies transitively; app host builds still explicitly enable defaults. (path: crates/made-terminal/Cargo.toml)

## Checkpoint
- [x] A freezes the interfaces above and verifies 1.1/1.6/1.8 pass; dependency graph remains acyclic and only platform contains OS-specific code. Local terminal/bridge tests still pass. This covers SSH-02/07/08 routing/path constraints; runtime acceptance is not yet established.

## 2. Disjoint leaf implementation — B/C/D/E parallel after phase 1
### B — host SSH and prompts
- [ ] 2.1 Write failing isolated-SSH tests for aliases/ProxyJump, unknown/changed keys, agent/key/encrypted-key/password/repeated MFA, cancellation, redaction, missing OpenSSH/prompt capability, and successful probe/stage with tmux absent or present and never executed. [P] (path: crates/made-platform/tests/ssh.rs)
- [ ] 2.2 Implement cancellable OpenSSH argv/stdio/bootstrap/staging process capability, safe fixed script/data separation, app-owned trust file and no forwarding/config mutation; remove every tmux probe and version gate from the fixed scripts; pass 2.1. (path: crates/made-platform/src/ssh.rs)
- [ ] 2.3 Write failing private IPC/request-bound askpass tests, including one-use answers and cancelled/stale requests. (path: crates/made-platform/tests/ssh_askpass.rs)
- [ ] 2.4 Implement OS-isolated private prompt IPC and prompt-response lifecycle; no network listener or answer logging/persistence. (path: crates/made-platform/src/ssh_askpass.rs)
- [ ] 2.5 Add packaged askpass executable using only the private IPC contract; pass 2.3 with the real OpenSSH subprocess. (path: crates/made-platform/src/bin/made-askpass.rs)
- [ ] 2.6 Extend existing shell-hook tests with cwd reporting while detached and unchanged profiles/hooks; observe failures. (path: crates/made-platform/tests/shell_integration.rs)
- [ ] 2.7 Add bounded nonblocking launch-local cwd-report sink for bash/zsh/fish, independent of the visible terminal byte stream and of any user multiplexer; preserve unsupported-shell fallback and existing local hooks. (path: crates/made-platform/src/shell_integration.rs)
- [ ] 2.8 Write failing async-client tests for priority queues, deadlines, reconnect/new epoch, nonreplayed input/focus/upload, malformed replies, and preservation of `TerminalOutput.done` across split resnapshot frames. (path: crates/made-remote/tests/client.rs)
- [ ] 2.9 After 2.33, implement remote protocol client over the platform SSH stream with explicit attach/detach/close and bounded cancellation; pass 2.8 without dropping `done` or treating an empty poll as resnapshot completion. (path: crates/made-remote/src/client.rs)

### C — remote service and guarded operations
- [ ] 2.10 Write failing native-supervisor tests with tmux absent: same shell/TUI PID after frontend loss, exclusive attach/epoch, modes/mouse/alternate-screen/primary-scrollback resnapshot without historical replay, `done: false` prefixes, one completing `done: true` before any live byte, later live `done: false`, lease/heartbeat, expiry and scoped close. [P] (path: crates/made-remote/tests/supervisor.rs)
- [ ] 2.11 After 2.31 and 2.33, implement the detached supervisor/lease and PTY owner by calling platform detach and `PtySession`, with bounded `TerminalCore` state, generated `TerminalOutput` resnapshot ended by `done: true`, and fixed child-exec wrapper; pass 2.10 without invoking tmux, implementing OS detach in made-remote, or killing unrelated sessions/shared OpenCode services. (path: crates/made-remote/src/supervisor.rs)
- [ ] 2.12 Write failing auth/loopback/cross-tab/epoch tests for remote supervisor RPC before state disclosure. (path: crates/made-bridge/tests/remote_rpc.rs)
- [ ] 2.13 Implement authenticated bounded remote RPC loopback binding inside made-bridge only; pass 2.12. (path: crates/made-bridge/src/remote_rpc.rs)
- [ ] 2.14 Write failing helper tests for lazy directory/load/watch/rescan, caps/bytes paths, preview classification, traversal/symlink/nonregular rejection and stale generations under a trusted server. (path: crates/made-remote/tests/filesystem.rs)
- [ ] 2.15 Implement filesystem/root/watch/preview operations by reusing PathGuard, TreeModel, PreviewLoader and FsWatcher; no alternate relaxed guards or host interpretations. (path: crates/made-remote/src/filesystem.rs)
- [ ] 2.16 Write failing shared Git tests for closed eight-operation allowlist, branch/linked/detached/status grouping inputs, ignored/subroot filtering, timeout/output caps and no mutations. (path: crates/made-remote/tests/git.rs)
- [ ] 2.17 Move the existing Git implementation into the shared UI-free library, add bounded process/output handling and preserve its public API for the later app re-export; pass 2.16. (path: crates/made-remote/src/git.rs)
- [ ] 2.18 Write failing integration tests for persistent remote BridgeHandle/Host/auth, focus/state relay, direct/manual OpenCode env and immutable remote config including V2/legacy macOS resolution. (path: crates/made-remote/tests/integration.rs)
- [ ] 2.19 Implement remote config/ConfigGuard and plugin BridgeOptions composition plus state/focus relay, keeping plugin URL actual remote loopback and remote config/env only; pass 2.18. (path: crates/made-remote/src/integration.rs)
- [ ] 2.20 Write failing PNG chunk/commit/path, 128 MiB caps, privacy/symlink, seven-day/64-file/current-image retention, partial-transfer/close/disconnect/deletion-failure tests. (path: crates/made-remote/tests/clipboard.rs)
- [ ] 2.21 Implement private remote PNG upload/cleanup using platform private-file operations; no workspace writes, no automatic retry or deletion of successful images on tab close; pass 2.20. (path: crates/made-remote/src/clipboard.rs)
- [ ] 2.22 Write failing capability/bootstrap/consent/target/hash/partial-stage tests, including tmux-absent and user-installed-tmux treated identically, `Capabilities.tmux` encoded false and ignored, read-only probing, and no binary installation without approval. (path: crates/made-remote/tests/bootstrap.rs)
- [ ] 2.23 Implement companion probe/stdio frontend/detached-supervisor/child-exec entry modes, version negotiation, private layout and consented atomic staging; do not search for or require tmux; pass 2.22. (path: crates/made-remote/src/main.rs)

### D — desktop controllers without shared UI edits
- [ ] 2.24 Write failing registry tests for restart, add/edit/remove, no credentials, corrupt/versioned data and live-tab immutable authority. [P] (path: crates/made-app/tests/ssh_servers.rs)
- [ ] 2.25 Implement private schema-v1 metadata/consent store and cancellable test/detection state with no credential persistence; pass 2.24. (path: crates/made-app/src/ssh_servers.rs)
- [ ] 2.26 Write failing remote controller tests for same path on host/two servers, remote selection, local-fs spy rejection, root precedence, pending generation/epoch results, lifecycle and session focus. (path: crates/made-app/tests/remote_workspace.rs)
- [ ] 2.27 Implement authority-aware remote controller, asynchronous root resolution, explorer/session projections, transport-vs-integration states and reconnect resnapshot; do not branch on `Capabilities.tmux` or mark the terminal current before `done: true`; pass 2.26. (path: crates/made-app/src/remote_workspace.rs)

### E — controlled harness authoring
- [ ] 2.28 Write isolated account/config/known-host/fixture provisioning and cleanup harness with explicit prerequisites and fault-profile setup/teardown; prerequisites exclude tmux, and setup must not install tmux or Rust; no personal servers/keys. [P] (path: tests/ssh/harness.py)
- [ ] 2.29 Add scenario driver for real PTY/Git/OpenCode mutation, PID/session observation, modes/mouse/alternate-screen resnapshot, input closed until `done: true`, prompt cancellation, SSH kill/loss/latency, hostile replies and evidence labeling on a tmux-absent server; first confirm missing product behavior fails. (path: tests/ssh/parity.py)
- [ ] 2.30 Write failing tests that platform detach survives parent hangup, keeps the PTY child alive, and does not invoke tmux. [P] (path: crates/made-platform/tests/detach.rs)
- [ ] 2.31 Implement SSH-session detach for the companion supervisor and pass 2.30; unsupported families fail preflight with no multiplexer fallback. (path: crates/made-platform/src/detach.rs)
- [ ] 2.32 Write failing codec tests that `TerminalOutput` is `{ data: Chunk, done: bool }`, the bit is required, empty `done: false` is rejected, empty `done: true` is a valid terminator, major stays 1, and no new operation exists. [P] (path: crates/made-remote/tests/protocol.rs)
- [ ] 2.33 Implement that payload on the existing verb, keep `is_terminal` true, and do not route the bit as the `Attach`/`Open` reply; pass 2.32. (path: crates/made-remote/src/protocol.rs)

## Checkpoint
- [ ] Per owner, observe behavioral test failures before implementing each behavior and retain commands/results. B/C/D files are disjoint; C's internal service files can be split further only after protocol freeze and without concurrent shared module edits. 2.9 and 2.11 wait for 2.33. 2.11 also waits for 2.31 and must not grow an OS detach implementation. Fakes validate boundaries, not acceptance. All new public module/manifest wiring is serialized through A as needed for compiling tests, with phase 3 completing the first joined build.

## 3. Wiring and first real survive/reattach slice — A with B/C, sequential gate
- [ ] 3.1 Register platform SSH/prompt/cwd/detach capabilities; preserve platform leaf dependency rule and do not move OS detach calls into made-remote. (path: crates/made-platform/src/lib.rs)
- [ ] 3.2 Declare askpass binary/dependencies without adding UI/domain imports to platform. (path: crates/made-platform/Cargo.toml)
- [ ] 3.3 Register remote RPC bridge module without weakening plugin listener checks. (path: crates/made-bridge/src/lib.rs)
- [ ] 3.4 Register completed companion modules/client exports and ensure the binary dependency graph is UI-free. (path: crates/made-remote/src/lib.rs)
- [ ] 3.5 Add app dependency on remote library for typed controllers/shared Git. (path: crates/made-app/Cargo.toml)
- [ ] 3.6 Register app SSH/remote controller modules. (path: crates/made-app/src/lib.rs)
- [ ] 3.7 Replace app Git implementation with compatible re-exports; existing worktree/scope-boundary tests must remain passing without duplicated allowlists. (path: crates/made-app/src/worktree.rs)
- [ ] 3.8 Update lockfile only for required new dependencies and verify locked builds. (path: Cargo.lock)
- [ ] 3.9 Execute the real reference Linux SSH/native-supervisor/shell/OpenCode vertical slice, with tmux absent, E owning any harness edits: consent/probe/start, resize/mouse/paste/Unicode, 60-second frontend loss, same PID/session reattach, modes/mouse/alternate-screen resnapshot ended by `done: true` before live bytes, input still closed on an empty poll, no input or historical-byte replay, and scoped close. Add regression cases for any failures before fixes; do not proceed on a terminal-only, auto-restart, or tmux substitute, and do not install tmux or Rust to make the slice runnable. (path: tests/ssh/parity.py)

## Checkpoint
- [ ] Gates SSH-01 provisioning/auth basics, SSH-05 real TUI fidelity and SSH-07 survival/lifecycle on the reference pair with tmux absent. If platform detach, prompt support, or the VT resnapshot cannot satisfy these outcomes, stop and revise design/spec/tasks then rerun analyze; do not hide a technical blocker behind UI completion or a tmux fallback.

## 4. Desktop integration test-first — A, sequenced shared-file owner
- [ ] 4.1 Extend local/remote app tab tests with remote add/cancel/close/last-tab/hidden-tab, remote picker and header reachability at both sizes; observe failures. (path: crates/made-app/tests/workspace_tabs.rs)
- [ ] 4.2 Add remote launch/selection lifecycle to AppModel without making startup wait for SSH; immutable saved-record snapshots per live tab. (path: crates/made-app/src/app.rs)
- [ ] 4.3 Add local/remote TerminalTarget variants and labels, without host executable/cwd substitution. (path: crates/made-app/src/tui.rs)
- [ ] 4.4 Extend follow tests for asynchronous remote root resolution, POSIX reports on Windows, precedence/fallback/epochs and no host fs; observe failures before 4.5. (path: crates/made-app/tests/session_follow.rs)
- [ ] 4.5 Introduce authority-aware root resolution in the existing coordinator, preserving all local behavior and unavailable-root semantics. (path: crates/made-app/src/session_follow.rs)
- [ ] 4.6 Add command-sink adapter for remote focus/state requests with existing correlation/timeout/cancellation behavior. (path: crates/made-app/src/session_focus.rs)
- [ ] 4.7 Extend terminal UI tests for remote transport, no input while disconnected, both image chords, commit-once IME and unchanged local clipboard semantics; observe failures. (path: crates/made-app/tests/ui_model.rs)
- [ ] 4.8 Adapt TerminalPane to TerminalTransport; reuse core/encoder/renderer; route remote image upload/path paste, hints and failures; clear pending input on detach and, on reattach, reset then feed the supervisor resnapshot. Reopen input only after the applied `done: true` frame, never because `poll` is empty, and do not treat a later `done: false` frame as more resnapshot. (path: crates/made-app/src/terminal_view.rs)
- [ ] 4.9 Extend explorer UI tests for remote bytes keys, selection/expansion, icons, preview actions/live changes, Git view and no host reads; observe failures. (path: crates/made-app/tests/read_only_ui.rs)
- [ ] 4.10 Select local or remote explorer data controller, retaining shared row/preview/Git presentation and keyboard/hide/restore/resize behavior. (path: crates/made-app/src/explorer_view.rs)
- [ ] 4.11 Integrate per-tab remote runtime ownership, authority-aware root/session/explorer/Git sinks, independent transport/integration diagnostics, heartbeat and bounded close; no direct remote I/O on the UI thread. (path: crates/made-app/src/workbench.rs)
- [ ] 4.12 Add remote server/directory selection path without changing local picker confirmation/cancel semantics. (path: crates/made-app/src/workspace_picker.rs)
- [ ] 4.13 Add Servidores SSH management/auth/consent cards and adjacent header action, `+` remote choices independent of host TUI detection, full cluster/overflow budget and contextual failure/reconnect notices. (path: crates/made-app/src/ui.rs)
- [ ] 4.14 Add only required SSH glyph/control tokens using the existing Calm Editor interaction system. (path: crates/made-app/src/theme.rs)
- [ ] 4.15 Extend diagnostics tests for authority, remote root/source/capability, transport-vs-plugin health, version/cleanup-unconfirmed status and secret/content redaction; observe failures. (path: crates/made-app/tests/diagnostics.rs)
- [ ] 4.16 Expose the tested remote fields in diagnostics without routine technical chrome or violating OpenCode-active path-free tooltips. (path: crates/made-app/src/diagnostics.rs)

## Checkpoint
- [ ] SSH-01 through SSH-09 model/regression tests pass with inherited local suites. Shared desktop files are sequential to avoid conflicting Workbench/UI ownership. No local cwd/session/file validation is weakened to accept remote strings. User action errors stay visible, routine diagnostics stay in About.

## 5. Native builds, packaging and end-to-end verification — E after integration
- [ ] 5.1 Add four headless companion target builds (glibc >=2.31 x64/arm64 and macOS >=12 x64/arm64), hash/plugin artifact manifest, target-runtime tests and askpass host binaries; no GUI dependency in companion. (path: .github/workflows/ci-ssh-remote.yml)
- [ ] 5.2 Assemble every desktop package with all remote target binaries/plugin/manifest plus its native askpass executable; missing/mismatched artifacts fail packaging, not runtime download. (path: .github/workflows/ci-artifacts.yml)
- [ ] 5.3 Execute real native client/server matrix with tmux absent, both window sizes, all auth methods, hooks/config hashes, remote explorer/Git/live preview, both image chords, same-session loss/reattach/expiry/close, loopback scans, compromised-response containment and network profiles; label unavailable combinations unverified and block full release acceptance. Do not install tmux or Rust as part of the run. (path: tests/ssh/parity.py)
- [ ] 5.4 Run cargo workspace tests, fmt/clippy/target/package checks, existing plugin Node 24 tests/typecheck, and local UI regression under concurrent stalled remote work; attach scenario-level results inline in the review/handoff rather than creating unrelated report files. (path: tests/ssh/harness.py)

## Checkpoint and scenario coverage
| Persisted requirements/scenarios | Test/implementation tasks |
|---|---|
| SSH-01 entry/overflow, per-server detection, failed probes/vanished executable, metadata/auth prompts | 2.1–2.5, 2.22–2.25, 3.9, 4.1–4.3, 4.12–4.14, 5.3 |
| SSH-02 identical paths on host/two servers, remote picker, invalid/unavailable directories | 1.1–1.7, 2.14–2.15, 2.26–2.27, 4.4–4.5, 4.9–4.12, 5.3 |
| SSH-03 browse/terminal roundtrip, live changes/late results, safe preview/listing caps, icons/copy/refresh/layout | 2.14–2.15, 2.26–2.29, 4.7–4.14, 5.3 |
| SSH-04 remote grouped Git/worktree/detached state, unavailable/stale Git and read-only allowlist | 2.16–2.17, 3.7, 4.9–4.11, 5.3 |
| SSH-05 real TUI fidelity, separate default/fallback image scenarios, failure/retention/private permissions | 1.8–1.10, 2.10–2.11, 2.20–2.21, 3.9, 4.7–4.8, 5.3 |
| SSH-06 precedence/focus, integration-only failure, consented staging/manual OpenCode/config preservation, tmux-absent and user-tmux treated identically | 1.6–1.7, 2.1–2.2, 2.6–2.7, 2.18–2.19, 2.22–2.23, 4.4–4.6, 4.11, 5.3 |
| SSH-07 same-server close isolation, offline local release/lease, same-process reconnect/no replay, modes/mouse/alt-screen resnapshot, explicit TerminalOutput done bit, empty-poll does not reopen input, tmux-irrelevant survival, lost/expired session | 1.8–1.10, 2.8–2.13, 2.26–2.33, 3.9, 4.1, 4.8, 4.11, 5.3 |
| SSH-08 unknown/changed host, trusted-helper containment/injection, hostile forged responses, network/config/credential audit, no tmux execution | 1.1–1.7, 2.1–2.5, 2.8–2.23, 2.28–2.31, 5.3–5.4 |
| SSH-09 local under stalled server, measured controlled end-to-end evidence, target/network/dependency matrix without tmux or a remote Rust toolchain | 2.28–2.31, 3.1, 3.9, 4.1–4.16, 5.1–5.4 |
| Modified Security: unauthenticated rejection, non-loopback bind, config unchanged, traversal, app-owned plugin, read-only preview | 1.6–1.7, 2.12–2.23, 5.3–5.4; inherited local bridge/config/path/preview tests retained |
| Modified Layout: new SSH inventory/order/new-tab behavior without losing local chrome | 4.1–4.3, 4.12–4.14, 5.3; inherited responsive/focus/tab tests retained |

- [ ] Review runs `verify` against this persisted spec and maps every scenario to passing tests, including inherited local guarantees, not merely the nine requirement summaries. Leave unresolved product failures unchecked; request contract evolution/analyze again before changing observable outcomes. No archive is authorized.

## Analyze result (resnapshot-delimiter rerun)
**PASS — ready to resume affected implementation, not product verified.** The delimiter correction stays inside the approved no-replay outcome. It adds a required `done` bit to the existing `TerminalOutput` verb and does not add an operation, change `PROTOCOL_MAJOR`, or restore a tmux dependency. Zero clarification markers remain. Proposal, spec, design, and tasks agree on the frame order, the input gate, and who writes the field. Each of the 38 persisted Given/When/Then scenarios is covered by the mapping above, including the split-frame and missing-completion scenarios. All 75 executable tasks have a unique id and one path. Phase 1's 13 tasks and its checkpoint stay checked; 2.32 and 2.33 are unchecked. `git diff --check` passes. No product code, wiki, workflow, real SSH/TUI proof, or cargo suite ran in this spec-only rerun.

This rerun is not blocked. `PtyEvent::Data` still has no chunk meaning, so completion cannot be inferred from an empty poll; B must preserve the bit and A must gate input on it. `Attach` still completes only with `Attached`. A missing `done` is malformed, not an implicit end. The phase-3 checkpoint still stops if the round-trip cannot restore modes, mouse, alternate screen, and bounded primary scrollback without replay. That stop is not permission to install or call tmux. Verification and release acceptance remain pending execution evidence.
