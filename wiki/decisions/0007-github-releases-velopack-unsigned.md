# 0007: GitHub Releases, Velopack, Unsigned Builds

**Status:** Accepted; `v0.1.0` published.

## Context

Made shipped only unsigned debug development artifacts that were not installers and could not update. End users need one download place, a simple per-user install per OS with nothing else to install, and an app that keeps itself current.

## Decision

- **Channel:** GitHub Releases only, produced by a pushed stable `v*` tag. Targets: Windows x64 setup, macOS arm64 and x64 (`.pkg` and `.zip` each), Linux x64 AppImage. Installs and updates need no administrator rights.
- **Toolchain:** Velopack (`velopack =1.2.161`, `vpk` 1.2.161) for installers and the update client. Made supplies its own update source and does its own download and verification in `made-platform`, because Velopack's built-in sources follow redirects to any host, send an install identifier, and never check the embedded version.
- **Unsigned:** no signing or notarization. Gatekeeper and SmartScreen warn on first run and possibly again after an update; macOS is ad-hoc signed only so `codesign --verify` passes. Trust model: [Security](../security.md#update-trust-model).
- **Update policy:** checks at launch and every 24 hours (opt-out in About), single stable channel, background download, apply and restart only after confirmation in About; a wordmark dot signals a ready update. Non-installed builds never update.
- **Single version source:** `[workspace.package].version` in `Cargo.toml`. `scripts/bump-version.sh X.Y.Z` rewrites every other site (plugin manifests, `PLUGIN_VERSION`, README example, lockfiles) and runs `.github/scripts/check-versions.py`; `ci.yml` fails on drift and `release-gate.py` refuses disagreement. Stamping only at staging was rejected because the repository plugin copies are consumed directly by development builds.
- **Approval gate:** publish waits for the protected `release` environment; settings in [Release and CI](../operations/release-ci.md#repository-settings).
- **Linux:** the AppImage runtime is replaced with uruntime v0.8.1 (`APPIMAGE_EXTRACT_AND_RUN=1`) so no libfuse or `fusermount` is needed, with a private per-user extraction target. Baseline glibc 2.35 (built on `ubuntu-22.04`). Windows binaries use `+crt-static`.
- **Upgrade verification:** no test repository; `upgrade-check.yml` installs the previous published release and updates it through the real in-app path.

## Consequences

- Anyone able to publish a release on the repository can ship an update; hardening is procedural plus the environment and rulesets.
- macOS may re-request privacy permissions (TCC) after an update because the ad-hoc signature changes per build.
- Windows apply is two renames, not atomic; power-loss behavior is unverified.
- Prereleases, other channels (Homebrew, winget), and other Linux formats are out of scope.

## Evidence

[`specs/changes/add-release-auto-update/proposal.md`](../../specs/changes/add-release-auto-update/proposal.md), [`tests/e2e/release_validation.md`](../../tests/e2e/release_validation.md), [release record](../records/20261106-v0-1-0-release.md).
