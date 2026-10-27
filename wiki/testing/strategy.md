# Test Strategy

Made's tests are layered: fast domain/model tests, real PTY/bridge integration tests, an OpenCode TUI smoke, the historical Phase 0 plugin gate, and a Linux/Xvfb UX validation pass. The **normative scenario list** is the 97-row core/UI checklist plus supplemental `UP-*` visual-polish rows in [`tests/e2e/ux_validation.md`](../../tests/e2e/ux_validation.md); results are summarized in the [v1 evidence record](../records/20261026-v1-evidence.md). Requirements remain in [`specs/changes/opencode-session-follow-v1/spec.md`](../../specs/changes/opencode-session-follow-v1/spec.md) and [`specs/changes/made-ui-ux-v1/spec.md`](../../specs/changes/made-ui-ux-v1/spec.md); do not treat this page as a substitute for the checklist or those specs.

## Layers and commands

| Layer | Command | What it covers |
|---|---|---|
| Rust workspace | `cargo test --workspace` | Unit, model, PTY, bridge, explorer, platform and app tests across all crates. Current Linux baseline: 654 passed, 0 failed, 2 ignored. |
| Format/lint | `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets -- -D warnings` | House style and warnings-as-errors. |
| App UI model | `cargo test -p made-app --test ui_model` | Mint header accent, fallback clear alpha 225, 26/24 card-edge header geometry and wordmark gaps, `+` placement, right icon order, fill-only idle/hover/selected tabs and scaled red close, faded splitter, no-outline Explorer/picker rows, picker tree expansion/keyboard/path chip/minimal scroll, Explorer animation, preview roundtrip, scrollbars, quiet chrome, Diagnostics previous-log absent/present/copy/tooltip, 720 warning containment, and startup `.1` permission states. |
| Workspace tabs | `cargo test -p made-app --test workspace_tabs` | Independent PTYs/roots/bridges, close-owner, hidden output, deleted-cwd recovery. |
| Launch/picker | `cargo test -p made-app --test launch_workspace`; `workspace_picker`; `invalid_workspace` | Home default, explicit path, picker, fallback errors. |
| Session follow/focus | `cargo test -p made-app --test session_follow`; `session_focus`; `degraded`; `diagnostics` | Root precedence, status transitions, focus timeout, diagnostics, 4 KiB message cap, 512 KiB text budget, previous-log path/copy data. |
| Bridge | `cargo test -p made-bridge` | Protocol, auth/replacement, snapshot bounds, config injection/immutability, security. |
| Explorer | `cargo test -p made-explorer` | Tree, watcher, root generation, preview guards, path guard. |
| Terminal | `cargo test -p made-terminal` | VT emulation, input encoding, PTY/ConPTY behavior. |
| Platform | `cargo test -p made-platform` | Paths, shell integration, contract tests, native backdrop, exact-threshold log rotation, lock symlink/directory/permission/timeout refusal, public rename-failure refusal, mode preservation, and barrier-synchronized append/rotation. |
| Plugin | `cd plugins/opencode-made && npm test && npm run typecheck` | Lifecycle, handshake, state bounds, focus, reconnect, redaction. Current: 29/29 plus typecheck. |
| Phase 0 gate (historical) | Harness removed after the gate | Real OpenCode v2.1.4 TUI: load, auth, tabs, worktree location, focus, reconnection, config immutability. 19/19 + 5/5; outcomes in the [Phase 0 record](../records/20261025-phase-0-gate.md). |
| Real TUI smoke | `cargo test -p made-app --test opencode_tui -- --nocapture` | PTY/TerminalCore runs a real OpenCode TUI when installed; skips otherwise. |
| App smoke | `cargo run -p made-app -- --smoke-test` | Creates the window, pumps frames, prints `made smoke: OK`, exits. |
| Session-follow E2E | `tests/e2e/run-session-follow-e2e.sh` | Real window plus synthetic plugin: bridge -> root follow -> explorer. Not real OpenCode evidence. |

## Scenario validation

[`tests/e2e/ux_validation.md`](../../tests/e2e/ux_validation.md) maps every core-spec scenario to a method (automated, Xvfb, real shell/spike, manual pending) and evidence. It has 97 pre-split scenario rows plus thirteen supplemental `UP-*` rows covering workspace launch, integration loading, shell follow, tabs, layout, session follow, worktrees, session map, preview, explorer updates, degraded state, terminal fidelity, cross-platform delivery, security, responsiveness, diagnostics, scope boundary, and the 2026-10-26/27 visual refinements.

Results and current status are in the [v1 evidence record](../records/20261026-v1-evidence.md). A row is not “passed” merely because a model test exists; native/manual rows require native evidence.

## Evidence layout

| Location | Contents |
|---|---|
| [`tests/e2e/ux_validation.md`](../../tests/e2e/ux_validation.md) | Normative 97-row scenario checklist, supplemental `UP-*` rows, and evidence method. |
| [v1 evidence record](../records/20261026-v1-evidence.md) | Baseline, verified behavior, open gaps, and performance summary. |
| [Performance](../performance.md) | Final release software measurements and method. |
| [Platform matrix](platform-matrix.md) | Build/automated coverage versus native runtime evidence. |

The wiki carries distilled outcomes only. Screenshots are ad hoc validation artifacts and are intentionally not committed or maintained here; raw logs, JSON, and historical runs are also excluded.

## Isolation rules

Every app-level run **must** isolate user state; unisolated app-model tests can write the real `~/.local/state/made/log` during shutdown. Run the workspace tests only with all of `HOME`, `USERPROFILE`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME`, `TMPDIR`, and `XDG_RUNTIME_DIR` under a scratch root:

```bash
root=/tmp/opencode/made-log-rotation-20261027/verify-home
rm -rf "$root"
mkdir -p "$root/home" "$root/tmp" "$root/runtime"
chmod 700 "$root/runtime"
HOME="$root/home" \
USERPROFILE="$root/home" \
XDG_CONFIG_HOME="$root/home/.config" \
XDG_DATA_HOME="$root/home/.local/share" \
XDG_STATE_HOME="$root/home/.local/state" \
XDG_CACHE_HOME="$root/home/.cache" \
TMPDIR="$root/tmp" \
XDG_RUNTIME_DIR="$root/runtime" \
  cargo test --workspace
```

Direct runs use the same environment:

```bash
HOME="$root/home" \
USERPROFILE="$root/home" \
XDG_CONFIG_HOME="$root/home/.config" \
XDG_DATA_HOME="$root/home/.local/share" \
XDG_STATE_HOME="$root/home/.local/state" \
XDG_CACHE_HOME="$root/home/.cache" \
TMPDIR="$root/tmp" \
XDG_RUNTIME_DIR="$root/runtime" \
  cargo run -p made-app -- --smoke-test /path/to/workspace
```

`--smoke-test` with a workspace starts the real workbench and can read real app paths/config. The folder-less form stays on the picker surface so CI does not start a shell in the user's home. `unset WAYLAND_DISPLAY` when forcing X11; set `LIBGL_ALWAYS_SOFTWARE=1` for software rendering. The per-test re-exec isolation in `ui_model`/`workspace_tabs` adds `TMPDIR`/`XDG_RUNTIME_DIR`; the top-level command must still set them for every other app test.

## Current status

- Rust: 654 passed, 0 failed, 2 ignored on Linux x64 WSL2; fmt/clippy clean.
- Plugin: 29/29 and typecheck pass.
- Phase 0: 19/19 gate plus 5/5 supporting against OpenCode v2.1.4.
- UI/behavior: Linux automated/headless evidence for launch, tabs, preview, Explorer, glass fallback, picker, bash cwd, deleted-cwd recovery, 10k startup, final software performance, bounded diagnostics-log rotation/retention and previous-log presentation, and the 2026-10-26/27 refinements (mint accent, darker 225 fallback clear, card-edge 26/24 header with 42/36 content top, right-side wordmark with 24/18 gaps, `+` after tabs, Sessions/Diagnostics/Explorer order, fill-only idle/hover/selected tabs, scaled red close, faded splitter, no-outline rows, inline picker tree expansion/keyboard/path chip/minimal scroll, fading scrollbars).
- Not validated: native Linux desktop, macOS, Windows/ConPTY, native log-lock/reparse behavior, final-exit log failure visibility (stderr only), real zsh/fish/PowerShell, live real-OpenCode switching, native IME/clipboard/mouse, hardware-GPU pacing, native backdrop rendering, packaged artifact launch, observed CI artifacts.

Related: [Platform matrix](platform-matrix.md), [Performance](../performance.md), [v1 evidence](../records/20261026-v1-evidence.md).
