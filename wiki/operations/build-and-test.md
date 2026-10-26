# Build and Test

Local development uses the Cargo workspace plus the TypeScript plugin. The Phase 0 gate harness has been removed. The binary target is `made-app`; the product/CLI name is `made`, and an installed build is expected to be named `made`. See [`Cargo.toml`](../../Cargo.toml) and [`README.md`](../../README.md).

## Prerequisites

| Tool | Requirement |
|---|---|
| Rust | Stable via `rustup`; `rust-toolchain.toml` selects stable with `clippy` and `rustfmt`. |
| Node.js | 24.x for the plugin. |
| OpenCode | Optional v2.1.4+ native distribution. Only used when the user runs `opencode`; WSL-hosted OpenCode is out of scope. |
| git | Optional read-only branch/worktree metadata; missing git degrades metadata only. |
| Display/GPU | Real X11/Wayland desktop, native macOS/Windows, working wgpu backend. CI installs X11/Wayland/EGL/GL/Vulkan development packages on Linux. |
| Shells | bash verified; zsh/fish/PowerShell source/unit coverage; cmd/unknown explicit fallback. |

## Build and run

```bash
cargo check --workspace
cargo build --workspace

# Explicit workspace (native window)
cargo run -p made-app -- /path/to/workspace

# No argument: platform-resolved home workspace
cargo run -p made-app

# Explicit folder picker
cargo run -p made-app -- --picker

# Pickers and explicit paths: explicit path wins
cargo run -p made-app -- --workspace /path/to/workspace --picker

# CI-style smoke: create window, pump frames, print marker, exit
cargo run -p made-app -- --smoke-test

# Smoke with a workspace and duration
cargo run -p made-app -- --smoke-test 2500 /path/to/workspace
```

`--smoke-test` without a workspace stays on the picker surface and starts no workbench. With a workspace it starts the real workbench and can read real app paths/config; isolate user state for developer smoke:

```bash
root="$(mktemp -d)"
mkdir -p "$root/home" "$root/tmp"
HOME="$root/home" \
XDG_CONFIG_HOME="$root/home/.config" \
XDG_DATA_HOME="$root/home/.local/share" \
XDG_STATE_HOME="$root/home/.local/state" \
XDG_CACHE_HOME="$root/home/.cache" \
TMPDIR="$root/tmp" \
  cargo run -p made-app -- --smoke-test /path/to/workspace
```

For X11 under Xvfb, unset `WAYLAND_DISPLAY`; use `LIBGL_ALWAYS_SOFTWARE=1` and `WINIT_UNIX_BACKEND=x11` for software rendering.

## Tests

```bash
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings

# Focused app suites
cargo test -p made-app --test ui_model
cargo test -p made-app --test workspace_tabs
cargo test -p made-app --test launch_workspace
cargo test -p made-app --test session_follow
cargo test -p made-app --test read_only_ui
cargo test -p made-app --test performance

# Real OpenCode TUI through the PTY when installed; skips otherwise
cargo test -p made-app --test opencode_tui -- --nocapture
```

Plugin:

```bash
cd plugins/opencode-made
npm install
npm test          # node --test tests/lifecycle.test.ts
npm run typecheck # tsc --noEmit
```

Phase 0 gate (historical; Linux, OpenCode v2, Node 24, tmux):

The spike harness was removed after the gate passed. Outcomes are preserved in the [Phase 0 gate record](../records/20261025-phase-0-gate.md), with integration details in [OpenCode bridge](../integration/opencode.md). The gate was integration evidence, not an app UI/revision pass.

## Environment switches

| Variable / switch | Effect |
|---|---|
| `MADE_NATIVE_BACKDROP=auto` | Default (also unset/unrecognized): attempt native macOS vibrancy/Windows 11 Mica once on the first UI frame. Linux/unsupported falls back to in-app glass. |
| `MADE_NATIVE_BACKDROP=off` | Skip the native attempt; use in-app glass with `applied=false` and the disabled reason. Useful for deterministic headless runs. |
| `MADE_SMOKE_TEST_MS` | Enables smoke mode with a duration when the CLI flag is absent (minimum 50 ms). |
| `MADE_PLUGIN_DIR` | Development-only plugin directory override. Release overrides must canonicalize inside the install root. |
| `TRANSLUCENT_WINDOW` | Code constant in [`theme.rs`](../../crates/made-app/src/theme.rs); `false` makes the window opaque and disables transparency. |
| `MADE_SHELL` | Test-only wrapper used by E2E harnesses to run a fake/real OpenCode under the Made PTY. |

Headless example with native backdrop disabled:

```bash
MADE_NATIVE_BACKDROP=off \
DISPLAY=127.0.0.1:99 LIBGL_ALWAYS_SOFTWARE=1 WINIT_UNIX_BACKEND=x11 \
  target/debug/made-app --smoke-test 2500 /path/to/workspace
```

## Display dependencies on Linux

CI installs `libxkbcommon-dev`, `libwayland-dev`, `libx11-dev`, `libegl1-mesa-dev`, `libgl1-mesa-dev`, and `libvulkan-dev`; smoke jobs also use Xvfb or headless Weston and a software Vulkan/GL backend. Xvfb has no window manager and cannot validate native input, IME, clipboard, or OpenCode TUI interaction.

## Related

- [Test strategy](../testing/strategy.md)
- [Platform matrix](../testing/platform-matrix.md)
- [Release and CI](release-ci.md)
- [Architecture](../architecture.md)
