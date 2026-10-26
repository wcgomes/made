# Release and CI

Made v1 produces **unsigned development builds**, not a production release. There is no signing, notarization, installer, package repository, or auto-update path. CI is configured to prove compilation, contract tests, smoke markers, and artifact assembly; it does not certify native UX. See [`.github/workflows`](../../.github/workflows) and the [platform matrix](../testing/platform-matrix.md).

## Workflows

| Workflow | Trigger | Coverage |
|---|---|---|
| [`ci.yml`](../../.github/workflows/ci.yml) | Push to main, PR, manual | Format/clippy; workspace build/test on macOS arm64/x64, Linux x64, Windows x64; plugin tests/typecheck. |
| [`ci-artifacts.yml`](../../.github/workflows/ci-artifacts.yml) | Push to main, manual | Debug development artifact packaging for every target. |
| [`ci-linux-display.yml`](../../.github/workflows/ci-linux-display.yml) | Push/PR/manual | `made-app --smoke-test` under Xvfb X11 and headless Weston Wayland; uploads logs. |
| [`ci-macos.yml`](../../.github/workflows/ci-macos.yml) | Push/PR/manual | macOS arm64/x64 build, platform contract tests, app smoke, attempted native OpenCode install; TUI interaction manual. |
| [`ci-windows.yml`](../../.github/workflows/ci-windows.yml) | Push/PR/manual | Windows build, platform/PTY contract tests, app smoke, attempted native OpenCode install; TUI interaction manual. |

CI jobs deliberately record “manual validation required” instead of claiming native behavior they cannot exercise.

## Artifacts

| Artifact | Contents |
|---|---|
| `made-macos-arm64`, `made-macos-x64`, `made-linux-x64`, `made-windows-x64` | `dist/made-app` (or `.exe`) from the build/test matrix; 7-day retention. |
| `made-development-<target>` | Debug `made-app`, README, Cargo metadata/toolchain, `plugins/opencode-made`, `version.json`; 14-day retention. |
| `made-linux-x11-smoke-logs`, `made-linux-wayland-smoke-logs` | Xvfb/Weston smoke logs. |
| `made-macos-arm64-smoke-logs`, `made-macos-x64-smoke-logs`, `made-windows-smoke-logs` | Platform smoke/OpenCode-install logs. |

The development artifact stages the plugin at `<artifact_root>/plugins/opencode-made`, matching release `AppPaths` resolution. It removes `node_modules` and includes no `/plugin` directory.

## Release limits

- Artifacts are unsigned; a local macOS build may be quarantined by Gatekeeper.
- No notarization, installer, package repository, or auto-update payload.
- The packaged/staged layout has **not been launched end-to-end**.
- CI runs/artifacts had **not been observed** from this workspace at the 2026-10-26 snapshot.
- CI does not validate native window managers/compositors, native input/IME/clipboard, native shell hooks, Windows Job Object cleanup, native OpenCode TUI interaction, native macOS/Windows backdrop rendering, or hardware-GPU pacing.
- WSL-hosted OpenCode is out of scope; Windows support means native OpenCode/ConPTY.

## Deterministic/headless runs

Use `MADE_NATIVE_BACKDROP=off` to skip the native material attempt and keep in-app glass with an honest disabled reason. Use `--smoke-test` with an isolated `HOME`/`XDG_*`/`TMPDIR`; the folder-less smoke keeps the picker surface and starts no workbench. On Linux X11 under Xvfb, unset `WAYLAND_DISPLAY` and use a software backend.

```bash
MADE_NATIVE_BACKDROP=off \
DISPLAY=127.0.0.1:99 LIBGL_ALWAYS_SOFTWARE=1 WINIT_UNIX_BACKEND=x11 \
  target/debug/made-app --smoke-test 2500 /path/to/workspace
```

## Release-blocking boundary

Do not treat a green CI build as v1 UX validation. The exit test is every spec scenario with passing evidence summarized in the [v1 evidence record](../records/20261026-v1-evidence.md); native Linux, macOS, Windows, real shells, live OpenCode switching, native input, packaged launch, and observed artifacts remain open. See [Platform matrix](../testing/platform-matrix.md) and [v1 evidence](../records/20261026-v1-evidence.md).

Related: [Build and test](build-and-test.md), [Testing strategy](../testing/strategy.md), [v1 evidence](../records/20261026-v1-evidence.md).
