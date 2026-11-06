# Release and CI

Made produces **unsigned development builds** on every push and **unsigned, installable, self-updating releases** from `v*` tags (see [Releases](#releases)). There is no signing, notarization, or package repository. CI proves compilation, contract/PTY tests, smoke markers, and artifact assembly; it does not certify native UX. The five CI workflows were observed green on Linux, macOS arm64/x64, and Windows x64 at the 2026-10-28 snapshot; `release.yml` and `upgrade-check.yml` are covered by the [release record](../records/20261106-v0-1-0-release.md); see [`.github/workflows`](../../.github/workflows) and the [platform matrix](../testing/platform-matrix.md).

## Workflows

| Workflow | Trigger | Coverage |
|---|---|---|
| [`ci.yml`](../../.github/workflows/ci.yml) | Push to main, PR, manual | Format/clippy; workspace build/test on macOS arm64/x64, Linux x64, Windows x64; plugin tests/typecheck. |
| [`ci-artifacts.yml`](../../.github/workflows/ci-artifacts.yml) | Push to main, manual | Debug development artifact packaging for every target. |
| [`ci-linux-display.yml`](../../.github/workflows/ci-linux-display.yml) | Push/PR/manual | `made-app --smoke-test` under Xvfb X11 and headless Weston Wayland; uploads logs. |
| [`ci-macos.yml`](../../.github/workflows/ci-macos.yml) | Push/PR/manual | macOS arm64/x64 build, platform contract tests, app smoke, attempted native OpenCode install; TUI interaction manual. |
| [`ci-windows.yml`](../../.github/workflows/ci-windows.yml) | Push/PR/manual | Windows build, platform/PTY contract tests, app smoke, attempted native OpenCode install; TUI interaction manual. |
| [`release.yml`](../../.github/workflows/release.yml) | `v*` tag push | Gate, release build and Velopack pack per target, installed smoke, publish (protected `release` environment), post-publish upgrade check. |
| [`upgrade-check.yml`](../../.github/workflows/upgrade-check.yml) | Manual, called by `release.yml` | Installs the previous published release per target/form and updates it in place to the latest. |

CI jobs deliberately record “manual validation required” instead of claiming native behavior they cannot exercise.

## Artifacts

| Artifact | Contents |
|---|---|
| `made-macos-arm64`, `made-macos-x64`, `made-linux-x64`, `made-windows-x64` | `dist/made-app` (or `.exe`) from the build/test matrix; 7-day retention. |
| `made-development-<target>` | Debug `made-app`, README, Cargo metadata/toolchain, `plugins/opencode-made`, `version.json`; 14-day retention. |
| `made-linux-x11-smoke-logs`, `made-linux-wayland-smoke-logs` | Xvfb/Weston smoke logs. |
| `made-macos-arm64-smoke-logs`, `made-macos-x64-smoke-logs`, `made-windows-smoke-logs` | Platform smoke/OpenCode-install logs. |

The development artifact stages the plugin at `<artifact_root>/plugins/opencode-made`, matching release `AppPaths` resolution. It removes `node_modules` and includes no `/plugin` directory. CI assembled these artifacts at the 2026-10-28 snapshot, but the staged layout has not been launched end-to-end.

## Release limits

- Artifacts and releases are unsigned (macOS bundles are ad-hoc signed only); Gatekeeper and SmartScreen warn on first run. Because the ad-hoc signature changes with every build, macOS may re-request privacy permissions (TCC) after an update.
- No notarization or package repository; GitHub Releases is the only channel.
- The packaged/staged layout has **not been launched end-to-end**.
- CI is observed green on Linux, macOS arm64/x64, and Windows x64 at the 2026-10-28 snapshot; artifacts are assembled but remain unlaunched, and green CI is not native UX evidence.
- CI does not validate native window managers/compositors, native input/IME/clipboard, native shell hooks, Windows Job Object cleanup, native OpenCode TUI interaction, native macOS/Windows backdrop rendering, or hardware-GPU pacing.
- WSL-hosted OpenCode is out of scope; Windows support means native OpenCode/ConPTY.

## Deterministic/headless runs

Use `MADE_NATIVE_BACKDROP=off` to skip the native material attempt and keep in-app glass with an honest disabled reason. Use `--smoke-test` with an isolated `HOME`/`XDG_*`/`TMPDIR`; the folder-less smoke keeps the picker surface and starts no workbench. On Linux X11 under Xvfb, unset `WAYLAND_DISPLAY` and use a software backend.

```bash
MADE_NATIVE_BACKDROP=off \
DISPLAY=127.0.0.1:99 LIBGL_ALWAYS_SOFTWARE=1 WINIT_UNIX_BACKEND=x11 \
  target/debug/made-app --smoke-test 2500 /path/to/workspace
```

## Releases

Releases are produced only by pushing a `v<major>.<minor>.<patch>` tag; branch pushes and pull requests never create or modify a release. `[workspace.package].version` in `Cargo.toml` is the single version source, and `ci.yml` fails on any drift of the other sites.

### Bump and release

`main` accepts changes only through pull requests, so a release bump is a pull request too:

1. Create a branch from `main` and run `scripts/bump-version.sh X.Y.Z` (stable versions only). It rewrites the workspace version, `plugins/opencode-made/package.json` and `package-lock.json` (through `npm version`), the plugin's `PLUGIN_VERSION`, `plugins/grok-made/plugin.json` and the README `appVersion` example, refreshes `Cargo.lock` through cargo and runs `.github/scripts/check-versions.py`.
2. Commit, push the branch and open a pull request against `main`; merge it once its checks are green.
3. On the updated `main`, tag the merge commit and push the tag: `git tag vX.Y.Z && git push origin vX.Y.Z`.
4. CI then runs, in order: the version gate (`release-gate.py`: refuses prerelease or build tags, a version not greater than the newest published stable release, an already published tag, or any version-site disagreement); the SSH companions and the release build and Velopack pack for Windows x64, macOS arm64, macOS x64 and Linux x64 with the glibc 2.35, `NEEDED`, import and minimum-OS checks; the installed-layout smoke of every target and macOS form; then the publish job waits for approval in the `release` environment.
5. Approve the pending `release` deployment (the run's **Review deployments** button). Publish then removes any stale draft, creates the draft with every asset and `SHA256SUMS` and notes from `.github/release-notes.md`, publishes it with `make_latest=true`, and runs the post-publish upgrade check from the previous release. A failure before publish leaves at most an unpublished draft.
6. A failed upgrade check is remedied by publishing a fixed, newer release. The failing release is never modified or replaced.

If the gate refuses the tag, read its message, fix the sites with the bump script (through a pull request) or choose a higher version, then delete the refused tag locally and remotely (`git tag -d vX.Y.Z && git push origin :refs/tags/vX.Y.Z`) before tagging again. Never re-tag a version that was published.

### Repository settings

All three are configured (done after `v0.1.0` was published):

- The `release` environment requires approval by the maintainer (`wcgomes`) as required reviewer and accepts deployments only from `v*` tags.
- Ruleset "main: PR only" (id 26664378): a pull request is required with 0 approvals, and force-push and deletion are blocked.
- Ruleset "release tags" (id 26664379): creating, updating and deleting `v*` tags is restricted; repository admins bypass. Rulesets cannot key on release state, so never re-tagging a published version stays a procedure.

Optional, manual, not enabled: GitHub immutable releases (**Settings › General › Releases**). It may be enabled now that `v0.1.0` is published.

### Rollout cleanup (done)

The outcome is in the [2026-11-06 release record](../records/20261106-v0-1-0-release.md). The temporary iteration releases cut during implementation (`v0.1.0` to `v0.1.9`) and their tags were deleted, and `v0.1.0` was released again from the final `main`. Machines that installed an iteration release need Made uninstalled first (`0.1.0` is lower than the installed version), `grok plugin uninstall grok-made --confirm` run locally and on each SSH server where Made installed it, and Made's staged remote version directories removed on each SSH server.

## Release-blocking boundary

Do not treat a green CI build as UX validation. The exit test is every spec scenario with passing evidence summarized in the [evidence record](../records/20261026-v1-evidence.md); native Linux, macOS, Windows, real shells, live OpenCode switching, native input, and packaged launch remain open. See [Platform matrix](../testing/platform-matrix.md) and [CI platform quirks](../testing/ci-platform-quirks.md).

Related: [Build and test](build-and-test.md), [Testing strategy](../testing/strategy.md), [CI platform quirks](../testing/ci-platform-quirks.md), [v1 evidence](../records/20261026-v1-evidence.md).
