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

## Known residuals

- REL-09, Linux AppImage: the extraction guard repairs an interrupted or damaged extraction only while its own `AppRun` is intact. If extracting was cut short inside `AppRun` itself, the runtime reuses the non-empty directory and runs the truncated guard, so that build does not start until `~/.cache/made` (or that build's `appimage-*` directory) is removed. A new build extracts to a different directory and is unaffected. UNVERIFIED by a test: the case needs a kill at that exact point of extraction.

## Iteration releases (task 9.2)

Iteration releases are temporary test releases cut under the rollout plan; their notes say so, and the coordinator
deletes them before the final `v0.1.0` re-release.

| Tag | Release | release.yml run | Installed smoke | Upgrade check | Notes |
|---|---|---|---|---|---|
| `v0.1.0` | https://github.com/wcgomes/made/releases/tag/v0.1.0 | [42073343557](https://github.com/wcgomes/made/actions/runs/42073343557) (success) | `CI-release` pass on all seven: Windows setup (`windows-2025`), macOS arm64 `.pkg` and `.zip` (`macos-15`), macOS x64 `.pkg` and `.zip` (`macos-15-intel`), Linux AppImage (`ubuntu-22.04`, `ubuntu-24.04`) | First-release exemption reported (upgrade jobs skipped) | 15 assets: 6 end-user downloads, 4 full packages, 4 feeds, `SHA256SUMS` (14 entries, `sha256sum -c` OK); published as latest, not a prerelease; notes start with the temporary-release line; `releases/latest/download/releases.linux-x64.json` redirects `github.com` → `github.com/wcgomes/made/releases/download/v0.1.0/` → `release-assets.githubusercontent.com` |
| `v0.1.1` | https://github.com/wcgomes/made/releases/tag/v0.1.1 | [42076755597](https://github.com/wcgomes/made/actions/runs/42076755597) (failure: upgrade check only) | `CI-release` pass on all seven | `CI-upgrade` 0.1.0 → 0.1.1 pass for Windows setup (on/off), macOS arm64 and x64 `.zip` (on/off) and Linux AppImage (on/off), including the Windows hand-off to the apply helper. macOS `.pkg` (arm64 and x64, on/off) failed: Velopack's package opens Made after installing, so the command-line apply correctly refused with "Made is running from this install (process 1180)", exited non-zero and changed nothing (`--version` stayed 0.1.0, app data preserved). Remedy: the check now closes that first-run window, and the fix ships as a newer release | The release itself was unaffected and not modified. The v0.1.0 gate re-run on the published tag ([42073343557](https://github.com/wcgomes/made/actions/runs/42073343557), attempt 2) refused with "already published" and "not greater than the newest published stable release v0.1.0"; nothing was built and the 15 assets are unchanged |
| `v0.1.2` | https://github.com/wcgomes/made/releases/tag/v0.1.2 | [42080975498](https://github.com/wcgomes/made/actions/runs/42080975498) (success) | `CI-release` pass on all seven | `CI-upgrade` 0.1.1 → 0.1.2 pass on all twelve: Windows setup, macOS arm64 `.pkg` and `.zip`, macOS x64 `.pkg` and `.zip`, Linux AppImage (`ubuntu-24.04`), each with automatic checks seeded on and off; `--update` downloaded and verified 0.1.2 without changing the install, `--update --apply` (Windows through the apply helper) installed it, `--version` reports 0.1.2, `preferences.json` and `ssh-servers.json` byte-identical, REL-03 layout complete, smoke line `update_repo=wcgomes/made update_seam=0` | Remedies the v0.1.1 upgrade-check failure with a newer release (REL-12 "Failed upgrade check") |
| `v0.1.3` | none (tag only) | [42087188425](https://github.com/wcgomes/made/actions/runs/42087188425) (cancelled) | not reached | not reached | Cancelled after the glibc 2.31 companion job hung for 30 minutes in `reset_discards_late_correlated_replies_but_keeps_attach_snapshot`: a lock-order deadlock in that test (fixed in 82ad8db), and the companion steps now time out after 30 minutes. Nothing was published |
| `v0.1.4` | none (tag only) | [42091593472](https://github.com/wcgomes/made/actions/runs/42091593472) (cancelled) | not reached | not reached | Superseded by v0.1.5 after branch CI on its head found two more test races (zsh report, ETXTBSY fixtures); nothing was published |
| `v0.1.5` | https://github.com/wcgomes/made/releases/tag/v0.1.5 | [42093551889](https://github.com/wcgomes/made/actions/runs/42093551889) (failure: one upgrade job) | `CI-release` pass on all seven | `CI-upgrade` 0.1.2 → 0.1.5 pass on 11 of 12. macOS arm64 `.pkg` (automatic checks on) found nothing newer: it ran 17 s after publishing, while `releases/latest/download/` still redirected to v0.1.2, so 0.1.2 correctly reported up to date and `--apply` had nothing to install. Remedy: the check now waits, bounded, until the `latest` feed names the new tag, shipped in v0.1.6 | Release assets unaffected and not modified |
| `v0.1.6` | https://github.com/wcgomes/made/releases/tag/v0.1.6 | [42098104113](https://github.com/wcgomes/made/actions/runs/42098104113) (success) | `CI-release` pass on all seven | `CI-upgrade` 0.1.5 → 0.1.6 pass on all twelve (Windows setup, macOS arm64/x64 `.pkg` and `.zip`, Linux AppImage; automatic checks on and off) | Branch CI green on the same head (cb80181); `SHA256SUMS` (14 entries) matches GitHub's asset digests. Remedies the v0.1.5 failure (REL-12 "Failed upgrade check") |
| `v0.1.7` | https://github.com/wcgomes/made/releases/tag/v0.1.7 | [42116369959](https://github.com/wcgomes/made/actions/runs/42116369959) (failure: upgrade check harness only) | `CI-release` pass on all seven | `CI-upgrade` 0.1.6 → 0.1.7: all twelve jobs failed checks that need the previous version to be 0.1.7 or later. All twelve failed "the next `--update` reports the recorded apply outcome" (0.1.6, which ran the apply, predates the outcome record); both Windows jobs also failed "the hand-off says the apply is not complete yet" (0.1.6 wording). One macOS `.pkg` job also ran while `releases/latest/download/` still named v0.1.6, so it found nothing to install. Remedy (aaf782a): those checks apply only when the previous release records outcomes (0.1.7 or later), and the check waits, bounded at 10 minutes, until the installed client is offered the new release | Release assets unaffected and not modified |
| `v0.1.8` | https://github.com/wcgomes/made/releases/tag/v0.1.8 | [42122383704](https://github.com/wcgomes/made/actions/runs/42122383704) (success) | `CI-release` pass on all seven | `CI-upgrade` 0.1.7 → 0.1.8 pass on all twelve (Windows setup, macOS arm64/x64 `.pkg` and `.zip`, Linux AppImage; automatic checks on and off). Windows printed "Handed off to helper process …: Made 0.1.8 is being installed and the apply is not complete yet", and the next `--update` printed "Last command-line apply: installed Made 0.1.8" | Branch CI green on the same head (2fe1073; Linux display smoke needed one re-run after an `apt` mirror stall). The publish gate re-ran on the draft, `latest` was set explicitly and the newest-release check passed; `SHA256SUMS` (14 entries) matches GitHub's asset digests |
| `v0.1.9` | https://github.com/wcgomes/made/releases/tag/v0.1.9 | [42130151982](https://github.com/wcgomes/made/actions/runs/42130151982) (success, attempt 2) | `CI-release` pass on all seven; the AppImage check on Ubuntu 22.04 and 24.04 found the target `~/.cache/made/appimage-0.1.9-3d891ab9899b3928` (per user, version and build), refused a target owned by another user, a group/other-writable target and a symlink, and replaced an extraction with a file cut short and one with most files missing, then started | `CI-upgrade` 0.1.8 → 0.1.9 pass on all twelve (Windows setup, macOS arm64/x64 `.pkg` and `.zip`, Linux AppImage; automatic checks on and off) | Branch CI green on the same head (65b7f4a). Attempt 1 stopped before anything was built or published: the macOS x64 companion job's tests passed but its artifact upload failed with `ENOTFOUND` (runner network outage); only the failed jobs were re-run |

## Second release from the final `v0.1.0` (task 9.3)

UNVERIFIED: the final `v0.1.0` re-release and its successor are dispatched separately after review.
