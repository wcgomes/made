# Release acceptance (add-release-auto-update, tasks 9.1 to 9.3)

Contract: `specs/changes/add-release-auto-update/`. This file records, per target, what was executed against
published releases and what was not. A combination that was not executed is labelled `UNVERIFIED` with its reason;
nothing here certifies a combination that did not run.

## Evidence classes

| Class | Meaning |
|---|---|
| `CI-release` | `release.yml` on a hosted runner for a published iteration release: per-user install of the end-user download, `--version`, `--smoke-test` markers and installed-layout checks (`smoke-installed.py`) |
| `CI-upgrade` | `upgrade-check.yml` on a hosted runner: previous published release installed per-user, app data seeded, updated in place through `--update` then `--update --apply`, latest version, preserved app data, REL-03 layout and smoke markers (`upgrade-check.py`) |
| `CI-probe` | `release-probe.yml` probes (runs 42062557947, 42064109555, 42068108063), recorded in the proposal's Decision index |
| `CI-test` | automated test in `ci.yml` |
| `UNVERIFIED` | not executed; the reason is given |

Hosted runners are not fresh machines: they carry developer tools, and they run the GUI only through `--smoke-test`
(and Xvfb on Linux). Fresh-machine, interactive and first-run click-through checks therefore remain `UNVERIFIED`
until run on real machines.

## Checklist

| Check | Windows 10 1809 | Windows 11 | macOS arm64 `.pkg` | macOS arm64 `.zip` | macOS x64 `.pkg` | macOS x64 `.zip` | Ubuntu 22.04 | Ubuntu 24.04 | Fedora (VM) |
|---|---|---|---|---|---|---|---|---|---|
| Fresh machine, no admin, no optional tools: install and launch | UNVERIFIED: no fresh 1809 machine | UNVERIFIED: hosted runner is not fresh | UNVERIFIED: hosted runner is not fresh | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED: no Fedora VM |
| Per-user install of the release download and installed-layout smoke | — | see Iteration releases (`windows-2025`) | see Iteration releases | see Iteration releases | see Iteration releases | see Iteration releases | see Iteration releases | see Iteration releases | UNVERIFIED |
| Launch without libfuse or `fusermount` | — | — | — | — | — | — | CI-probe | CI-probe | UNVERIFIED |
| X11 and Wayland sessions | — | — | — | — | — | — | X11 under Xvfb only (`CI-release`); Wayland UNVERIFIED | X11 under Xvfb only; Wayland UNVERIFIED | UNVERIFIED |
| SmartScreen / Gatekeeper first-run steps | UNVERIFIED: needs an interactive browser download | UNVERIFIED | UNVERIFIED: documented from `spctl` rejection (CI-probe) | UNVERIFIED | UNVERIFIED | UNVERIFIED | — | — | — |
| Offline launch, warning after update | UNVERIFIED: interactive | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED |
| Apply kill points (before swap, during copy, after swap) | CI-probe (hosted Windows Server 2025) | CI-probe | CI-probe | CI-probe | CI-probe | CI-probe | CI-probe | CI-probe | UNVERIFIED |
| Power loss during apply | UNVERIFIED: swap is two renames (not atomic) | UNVERIFIED: same | UNVERIFIED: `renamex_np(RENAME_SWAP)` is atomic by construction, not power-cycled | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED: atomic `rename(2)` only on one filesystem | UNVERIFIED | UNVERIFIED |
| About / wordmark indicator behavior | UNVERIFIED: interactive (headless UI-model tests are `CI-test`) | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED |
| Installed-layout launch with components resolved | see Iteration releases | see Iteration releases | see Iteration releases | see Iteration releases | see Iteration releases | see Iteration releases | see Iteration releases | see Iteration releases | UNVERIFIED |

## Iteration releases (task 9.2)

Iteration releases are temporary test releases cut under the rollout plan; their notes say so, and the coordinator
deletes them before the final `v0.1.0` re-release.

| Tag | Release | release.yml run | Installed smoke | Upgrade check | Notes |
|---|---|---|---|---|---|
| `v0.1.0` | https://github.com/wcgomes/made/releases/tag/v0.1.0 | [42073343557](https://github.com/wcgomes/made/actions/runs/42073343557) (success) | `CI-release` pass on all seven: Windows setup (`windows-2025`), macOS arm64 `.pkg` and `.zip` (`macos-15`), macOS x64 `.pkg` and `.zip` (`macos-15-intel`), Linux AppImage (`ubuntu-22.04`, `ubuntu-24.04`) | First-release exemption reported (upgrade jobs skipped) | 15 assets: 6 end-user downloads, 4 full packages, 4 feeds, `SHA256SUMS` (14 entries, `sha256sum -c` OK); published as latest, not a prerelease; notes start with the temporary-release line; `releases/latest/download/releases.linux-x64.json` redirects `github.com` → `github.com/wcgomes/made/releases/download/v0.1.0/` → `release-assets.githubusercontent.com` |
| `v0.1.1` | https://github.com/wcgomes/made/releases/tag/v0.1.1 | [42076755597](https://github.com/wcgomes/made/actions/runs/42076755597) (failure: upgrade check only) | `CI-release` pass on all seven | `CI-upgrade` 0.1.0 → 0.1.1 pass for Windows setup (on/off), macOS arm64 and x64 `.zip` (on/off) and Linux AppImage (on/off), including the Windows hand-off to the apply helper. macOS `.pkg` (arm64 and x64, on/off) failed: Velopack's package opens Made after installing, so the command-line apply correctly refused with "Made is running from this install (process 1180)", exited non-zero and changed nothing (`--version` stayed 0.1.0, app data preserved). Remedy: the check now closes that first-run window, and the fix ships as a newer release | The release itself was unaffected and not modified. The v0.1.0 gate re-run on the published tag ([42073343557](https://github.com/wcgomes/made/actions/runs/42073343557), attempt 2) refused with "already published" and "not greater than the newest published stable release v0.1.0"; nothing was built and the 15 assets are unchanged |

## Second release from the final `v0.1.0` (task 9.3)

UNVERIFIED: the final `v0.1.0` re-release and its successor are dispatched separately after review.
