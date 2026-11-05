# Tasks: add-release-auto-update

Readiness: `analyze` must pass before implementation; review runs `verify` against the persisted spec.

Phase 1 probes must finish, and any fallback that changes observable behavior must be routed back into spec.md with `analyze` rerun, before Phases 3 to 7 start. Cross-change ordering with `specs/changes/grok-native-hook/` is listed under Cross-change dependencies. The only wiki edit in scope is 8.2; other descriptive knowledge is updated by a post-implementation ingest.

## 1. Early verification probes (Velopack and self-containment)
- [x] 1.1 Create a standalone probe crate with its own empty `[workspace]` table, not a workspace member, depending only on a pinned `velopack` crate version (path: tests/release-probe/Cargo.toml)
- [x] 1.2 Write the probe binary. It runs Velopack's startup hook with auto-apply-on-startup disabled; reports installed vs not-installed detection, channel and installed version; checks a feed for a newer stable version only; downloads, then applies and restarts on a flag; logs every request URL and redirect hop; and reports whether the check request can be observed at the transport layer, and otherwise the full header set it sends, including `User-Agent`. This verifies the Rust SDK API for check, download and apply. Fallbacks, applied in Phase 4:
  - no GitHub source: use an HTTPS source at `https://github.com/<owner>/<repo>/releases/latest/download/`, which excludes prereleases;
  - auto-apply on startup cannot be disabled: stop and return to the user (conflicts with REL-07);
  - no way to tell non-installed builds: gate on an installed-layout marker resolved in `made-platform`;
  - hosts or redirects cannot be restricted: implement a custom update source in `made-platform` that enforces the REL-08 allowlist on every hop;
  - no SHA-256 or embedded-version check before apply: `made-platform` performs both before marking an update ready (path: tests/release-probe/src/main.rs)
- [x] 1.3 Add a manually triggered macOS probe job on arm64 and x64 runners (path: .github/workflows/release-probe.yml).
  - Pack the probe at two versions as `.pkg` and `.zip`, in a layout mirroring REL-03 (`plugins/`, `remote/<target>/`, askpass).
  - Install the `.pkg` for the current user without admin. Separately, extract the `.zip` into `~/Applications` with the quarantine attribute set.
  - Check that the plugin can be resolved from `Contents/Resources` and that companions and askpass can be resolved next to the executable.
  - Check that `codesign --verify` passes on arm64 and that a v1-to-v2 update from a local feed relaunches. Record the exact Gatekeeper steps.
  - Fallbacks:
    - Layout: if Velopack accepts a prebuilt `.app` as pack input, build the bundle with the Resources/MacOS split ourselves. Otherwise, after `grok-native-hook` 3.2, extend `made-platform` release resolution so the plugin also resolves inside the bundle's executable directory (app-owned), and rerun `analyze`.
    - Installer: if the `.pkg` needs admin, the `.zip` into `~/Applications` becomes the documented primary path, and the `.pkg` ships only if it installs per-user. Otherwise return to the user.
    - Signature: if the arm64 signature is invalid after pack or update, ad-hoc sign (`codesign --sign -`, no identity) during packaging.
- [x] 1.4 Add a Linux AppImage probe job on `ubuntu-22.04` and `ubuntu-24.04` runners (path: .github/workflows/release-probe.yml).
  - Use VMs, not containers, because containers lack `/dev/fuse` and do not model a desktop.
  - Purge libfuse2/libfuse3 and `fusermount`, and assert they are absent.
  - Launch the probe AppImage and perform a v1-to-v2 update with the AppImage stored in a user directory.
  - Fedora is run on a VM in 9.1. Container runs are supplementary only.
  - Fallback decision:
    - (A) Replace Velopack's AppImage runtime with the static FUSE-less AppImage type-2 runtime in post-processing. Accept it only if a re-run shows launch and update still work.
    - (B) Otherwise stop and return to the user, because the remaining options (a documented FUSE prerequisite, or a non-updating tarball) change REL-04 or REL-01.
- [x] 1.5 Add a job that inspects Velopack's own installer and updater binaries against the REL-04 baseline. Fallback: any dependency outside the baseline stops and returns to the user (path: .github/workflows/release-probe.yml)
  - Windows: imports (`dumpbin /dependents`) and the requested execution level (per-user, `asInvoker`, install under `%LocalAppData%`).
  - macOS: `otool -L` and minos for the update helper.
  - Linux: ELF `NEEDED` entries inside the AppImage.
- [x] 1.6 Add a Windows job that builds `made-app.exe`, `made-askpass.exe` and `made-grok-status.exe` with `-C target-feature=+crt-static`, asserts their imports contain no `VCRUNTIME*`, `MSVCP*` or `api-ms-win-crt-*` DLLs, and runs `made-app --smoke-test`. Fallback: app-local deployment of the VC++ runtime DLLs beside those executables, under Microsoft's redistribution terms (path: .github/workflows/release-probe.yml)
- [x] 1.7 Add a channels, naming and kill-point job on each OS (path: .github/workflows/release-probe.yml).
  - Checks:
    - The four channels (`win-x64`, `osx-arm64`, `osx-x64`, `linux-x64`) produce non-colliding feed files in one release folder.
    - Renaming end-user downloads to versioned names keeps updates working.
    - A higher-versioned prerelease-style entry is ignored.
    - Killing the updater or apply process before the swap, during the copy, and after the swap but before relaunch always leaves a launchable version.
    - Record from Velopack's implementation whether each OS's swap is atomic, so that power loss can be claimed or marked unverified.
  - Fallbacks:
    - A rename breaks updates: keep Velopack's names only for files the feed references and rename the rest. If an end-user download is feed-referenced, return to the user.
    - Any kill point leaves no launchable version: this is release-blocking and returns to the user.
- [x] 1.8 Record each probe outcome (pass or chosen fallback), the confirmed Velopack crate version and API names, the observed request and redirect hosts that form the REL-08 allowlist, whether the request is observable at the transport layer or else the full header set sent, including `User-Agent`, and the per-OS swap atomicity in the Decision index. Route any behavior-changing fallback into spec.md (path: specs/changes/add-release-auto-update/proposal.md)

## Checkpoint
- [x] Every probe 1.2 to 1.7 has a recorded outcome; no "return to the user" outcome remains unresolved; `analyze` rerun if any fallback changed behavior

## 2. Single version source, bump action and license metadata
- [x] 2.1 Write failing tests for the version checker [P] (path: .github/scripts/test_check_versions.py):
  - all sites equal passes: workspace `Cargo.toml`, `plugins/opencode-made/package.json`, both root version fields of `plugins/opencode-made/package-lock.json`, `PLUGIN_VERSION` in `plugins/opencode-made/src/protocol.ts`, `plugins/grok-made/plugin.json`;
  - each site's mismatch fails, naming the site and version;
  - tag mismatch fails;
  - invalid semver fails;
  - a prerelease tag is refused.
- [x] 2.2 Implement the version checker for every site listed in 2.1 with an optional `--tag`; confirm 2.1 passes (path: .github/scripts/check-versions.py)
- [x] 2.3 Add `license = "MIT"` to `[workspace.package]` [P] (path: Cargo.toml)
- [x] 2.4 Write failing tests for the bump action on a fixture copy of the repository [P] (path: .github/scripts/test_bump_version.py):
  - `bump-version.sh 0.2.0` rewrites exactly the source sites plus the `appVersion` example in `plugins/opencode-made/README.md`, and leaves test fixtures such as `0.1.0-test` untouched;
  - `Cargo.lock` is refreshed through cargo, and `plugins/opencode-made/package-lock.json` changes through `npm version 0.2.0 --no-git-tag-version` with `npm ci` still passing;
  - invalid and prerelease versions are refused;
  - `check-versions.py` passes afterwards.
- [x] 2.5 Implement the single bump action and confirm 2.4 passes (path: scripts/bump-version.sh)
- [x] 2.6 Make the plugin lifecycle test compare `hello.pluginVersion` with the `package.json` version instead of a literal; confirm it still passes (path: plugins/opencode-made/tests/lifecycle.test.ts)
- [x] 2.7 Run the version checker and the 2.1 and 2.4 tests on every push to main and every pull request; land after `grok-native-hook` 2.2 (path: .github/workflows/ci.yml)

## Checkpoint
- [ ] verify REL-02 "One action bumps every version site" (2.4) and "Plugin manifest drifts in a pull request" (2.1 fixtures plus a CI run failing on a drift branch); `check-versions.py` passes on the repository

## 3. Shared staging and self-contained builds
- [x] 3.1 Write failing tests for shared staging. For debug and release profiles, given fake build outputs, the staged tree has (path: .github/scripts/test_stage_desktop_package.py):
  - made-app;
  - `plugins/opencode-made` without `node_modules`;
  - `<install_root>/plugins/grok-made` produced by `stage-grok-plugin.sh` from a fixture hook, with the exact `grok-native-hook` file set and its stamped version;
  - LICENSE and executable bits on Unix;
  - `version.json` whose release profile reports `profile: release` and does not report `installer: none` or `auto_update: none`.
- [x] 3.2 Extract the desktop staging from `ci-artifacts.yml`, after the `grok-native-hook` 2.6–6.3 merge, into a profile-aware script. It stages grok-made only through `.github/scripts/stage-grok-plugin.sh`, adds LICENSE and writes profile-specific metadata. Confirm 3.1 passes (path: .github/scripts/stage-desktop-package.py)
- [x] 3.3 Switch development artifacts to the shared script with the debug profile. Preserve the `grok-native-hook` 5.4 hook build and its 5.10 packaged-hook test step, and keep artifact names, retention and contents apart from the added LICENSE (path: .github/workflows/ci-artifacts.yml)
- [x] 3.4 Configure the Windows MSVC static CRT for all Windows binaries per the 1.6 outcome (path: .cargo/config.toml)
- [x] 3.5 Write failing tests for the Linux ELF `NEEDED` allowlist checker using sample `readelf -d` output: glibc libraries and `libgcc_s.so.1` are allowed, any other library fails [P] (path: .github/scripts/test_check_linux_deps.py)
- [x] 3.6 Implement the Linux allowlist checker for the REL-04 baseline, including `libgcc_s.so.1`; the glibc 2.35 floor stays with `check-glibc-max.sh`; runtime-loaded libraries are left to fresh-distribution acceptance; confirm 3.5 passes (path: .github/scripts/check-linux-deps.py)
- [x] 3.7 Write failing tests for the Windows import checker using sample `dumpbin /dependents` output for `made-app.exe`, `made-askpass.exe` and `made-grok-status.exe` [P] (path: .github/scripts/test_check_windows_imports.py)
- [x] 3.8 Implement the Windows import checker allowing only DLLs that ship with Windows 10 1809; confirm 3.7 passes (path: .github/scripts/check-windows-imports.py)
- [x] 3.9 Write failing tests for the macOS minimum-OS checker using sample `otool -l` build-version output: minos 12.0 or lower passes, higher fails [P] (path: .github/scripts/test_check_macos_minos.py)
- [x] 3.10 Implement the macOS minimum-OS checker for `made-app`, `made-askpass` and `made-grok-status`; confirm 3.9 passes (path: .github/scripts/check-macos-minos.py)
- [x] 3.11 Write failing tests that run `stage-askpass.sh` with a stub `cargo` on PATH: `PROFILE=release` selects `--release` and `target/<target>/release`, and the default stays debug [P] (path: .github/scripts/test_stage_askpass.py)
- [x] 3.12 Make askpass staging profile-aware with a debug default, so `ci-ssh-remote.yml` is unchanged; confirm 3.11 passes (path: .github/scripts/stage-askpass.sh)

## Checkpoint
- [x] verify REL-04 "Dependency inspection" for release binaries of all four targets (Linux `NEEDED` allowlist and glibc 2.35, Windows imports, macOS minos 12.0); development artifacts still build, and the `grok-native-hook` 5.10 step still runs

## 4. Update capability in made-platform
- [x] 4.1 Add the pinned `velopack` dependency confirmed in 1.8 (path: crates/made-platform/Cargo.toml)
- [x] 4.2 Write failing tests (path: crates/made-platform/tests/update.rs):
  - non-installed builds report not-applicable and make no request;
  - each target uses its own channel;
  - only strictly newer stable versions are offered;
  - HTTPS only, with an exact host allowlist (`github.com`, `api.github.com`, `objects.githubusercontent.com`, `release-assets.githubusercontent.com`, adjusted to the 1.8 observations) enforced on every redirect hop, and `github.com`/`api.github.com` URLs pinned to `/<owner>/<repo>/`;
  - packages whose SHA-256 mismatches, that are truncated, or whose embedded version differs are discarded;
  - a kept downloaded update is found at startup;
  - the startup hook never applies on its own;
  - the update repository is a fixed compile-time constant naming the production repository, with no override;
  - the tamper, redirect and allowlist cases inject their source/transport only through a test-only, non-default cargo feature;
  - the request a check actually sends, asserted at the transport layer (or against the full header set including `User-Agent` that 1.8 recorded as sent), contains only the repository path, platform, architecture and installed version, with no workspace paths, file names, user names, server records, session data or install identifiers (REL-06; chosen over a recording proxy, which would change the client's trust rules). If 1.8 found the built-in source opaque, this test applies to the custom source from the 1.2 fallback.
- [x] 4.3 Implement the update capability (startup hook, installed detection, check, background download, verification, apply-and-restart, update-repository report) using the 1.8 outcomes and fallbacks; confirm 4.2 passes (path: crates/made-platform/src/update.rs)
- [x] 4.4 Export the update module, after `grok-native-hook` 3.2 lands in the same file (path: crates/made-platform/src/lib.rs)
- [x] 4.5 Run the startup hook first in `main`, and pass smoke-test mode so updates are disabled under `--smoke-test` (path: crates/made-app/src/main.rs)

## Checkpoint
- [x] verify at the capability level: REL-08 "Tampered package rejected", "Non-GitHub or HTTP source refused" and "Wrong embedded version rejected"; REL-06 "Older or prerelease versions never offered" and "Check reveals no private data" (client-boundary request test); REL-11 "Developer build"

## 5. Update scheduling, preference and diagnostics in made-app
- [x] 5.1 Write failing tests for the update preference (path: crates/made-app/tests/update_preference.rs):
  - defaults to on;
  - persists across restart and survives replacement of app files;
  - a corrupt file reads as on and reports a diagnostic;
  - private permissions on Unix.
- [x] 5.2 Implement the preference store in private app data beside the SSH registry; confirm 5.1 passes (path: crates/made-app/src/preferences.rs)
- [x] 5.3 Write failing scheduler tests with a fake capability and clock (path: crates/made-app/tests/updates.rs):
  - a check at launch, then every 24 h, off the UI thread;
  - a manual check, and an explicit command-line entry check, runs while automatic checks are off without changing the stored choice;
  - no request when checks are disabled, the build is non-installed, or under smoke test;
  - after download, ready only after verification;
  - declining keeps the update ready and never applies it at quit or launch; relaunch re-offers it without re-download; the newest known version supersedes once verified;
  - failures go only to the log, with no indicator;
  - status-line states including failed download and failed verification.
- [x] 5.4 Implement the update scheduler and state; confirm 5.3 passes (path: crates/made-app/src/updates.rs)
- [x] 5.5 Write failing tests that update outcomes appear in the bounded, redacted diagnostics log with no paths, user names or tokens (path: crates/made-app/tests/diagnostics.rs)
- [x] 5.6 Record update outcomes in diagnostics; confirm 5.5 passes (path: crates/made-app/src/diagnostics.rs)
- [x] 5.7 Write failing tests (path: crates/made-app/tests/update_restart.rs):
  - `Restart now` closes every workspace before apply and restart;
  - `Not now` touches nothing;
  - the smoke line reports OpenCode plugin source, `made-grok-status` presence, companion and askpass resolution, `updates=disabled` and `update_repo=<owner>/<repo>` for the production repository;
  - the command-line upgrade entry by default only checks and downloads, applies only with the explicit apply flag, uses the same capability as the About path, and reports not applicable with no request in a non-installed build;
  - with the apply flag while any Made instance of the same install is running, it exits non-zero with a clear message and changes no installed files;
  - during a command-line apply the install lock is held for the whole apply, GUI startup for that install refuses with a clear message while it is held, and the apply exits without starting the GUI;
  - it works while automatic checks are off and leaves the stored choice off.
- [x] 5.8 Wire the scheduler into the app tick, the restart sequence and the smoke markers; confirm 5.7 passes (path: crates/made-app/src/app.rs)
- [x] 5.9 Register the new modules (path: crates/made-app/src/lib.rs)
- [x] 5.10 Add the non-interactive command-line upgrade entry for the REL-12 check; confirm 5.7 passes (path: crates/made-app/src/cli.rs)

## Checkpoint
- [x] verify:
  - REL-06 "Newer version available", "Periodic check while running" and "No newer version";
  - REL-07 all scenarios;
  - REL-09 "Launch offline", "GitHub unreachable or malformed feed" and "Interrupted download";
  - REL-10 all scenarios;
  - REL-11 "Smoke test in CI".

## 6. Update UI
- [x] 6.1 Write failing UI-model tests (path: crates/made-app/tests/ui_model.rs):
  - the wordmark indicator appears only when an update is ready, only on the header wordmark of an active workspace, never on the picker wordmark or in non-installed builds, and returns when a workspace becomes active;
  - the tooltip and accessibility name read `About, update ready` (Keyboard Focus and Accessibility);
  - wordmark target, gaps, reserved width and tab fitting are unchanged at 1280×800 and 720×480;
  - the About Updates status line covers every state, including failed download, failed verification and the non-installed text;
  - `Check for updates` is unavailable while busy;
  - the inline restart confirmation names the version and the closing workspaces and sessions, and `Not now` and close dismiss it;
  - the compact height fits;
  - every control has focus and an accessible name;
  - existing About, header and accessibility scenarios still pass.
- [x] 6.2 Implement the wordmark indicator, its accessible name and the About Updates section with inline confirmation; confirm 6.1 passes (path: crates/made-app/src/ui.rs)

## Checkpoint
- [x] verify:
  - MODIFIED `Minimal Workspace Layout`: "Update-ready indicator" and all existing scenarios;
  - MODIFIED `About Presentation`: "About default content", "About Updates controls in an installed build", "Restart confirmation inside About", "About in a non-installed build" and the existing About scenarios;
  - MODIFIED `Keyboard Focus and Accessibility`: "Icon actions expose accessible names" and the other existing scenarios.

## 7. Release pipeline
- [x] 7.1 Write failing tests for release asset finishing (path: .github/scripts/test_release_assets.py):
  - end-user downloads are renamed to `made-<version>-<os>-<arch>` names per target;
  - feed-referenced files are unchanged per 1.7;
  - `SHA256SUMS` lists every other asset with a correct digest.
- [x] 7.2 Implement release asset finishing; confirm 7.1 passes (path: .github/scripts/release-assets.py)
- [x] 7.3 Write the release-notes template [P] (path: .github/release-notes.md). It covers:
  - per-OS install steps;
  - the exact SmartScreen and Gatekeeper first-run steps from 1.3, and the note that they may repeat after an update;
  - the AppImage `chmod +x` step;
  - no global-disable advice;
  - `SHA256SUMS` usage, stated as corruption detection, not proof of publisher.
- [x] 7.4 Write failing tests for the release gate with a fake GitHub API (path: .github/scripts/test_release_gate.py). It refuses:
  - prerelease tags;
  - versions not greater than the latest published stable release;
  - already-published tags;
  - any REL-02 disagreement.

  It also deletes an unpublished draft left by a failed run for the same tag.
- [x] 7.5 Implement the release gate; confirm 7.4 passes (path: .github/scripts/release-gate.py)
- [x] 7.6 Write failing structure tests for the release workflows (path: .github/scripts/test_release_workflow.py):
  - job order: gate, then build-and-pack, then installed smoke, then publish, then the post-publish upgrade check;
  - publish needs every target, runs in the protected `release` environment, and alone has `contents: write`;
  - no cache restore in any release job;
  - every third-party `uses:` is pinned to a 40-hex commit SHA;
  - `make_latest` is set explicitly;
  - the upgrade-check workflow has `workflow_dispatch` and `workflow_call` triggers and read-only permissions;
  - no release build step enables the test-only update seam feature.
- [x] 7.7 Write the installed-layout smoke harness (path: .github/scripts/smoke-installed.py). Per OS:
  - install per-user (Windows installer; macOS `.pkg` and `.zip`; Linux AppImage);
  - launch `--smoke-test` and assert the 5.7 markers, including `update_repo` equal to the production repository;
  - hash the 10-artifact companion set against the manifest;
  - assert `plugins/grok-made` contains an executable `made-grok-status` and no Python file;
  - check executable bits;
  - assert the packaged binary was built without the test-only update seam feature, for example through a build-metadata marker on the smoke line;
  - fail on any miss.
- [x] 7.8 Add the version-gate job on `v*` tags, running `release-gate.py` (path: .github/workflows/release.yml)
- [x] 7.9 Add the build-and-pack job (path: .github/workflows/release.yml):
  - consume the 10-artifact companion set from `ci-ssh-remote.yml` (built by `build-linux-companion.sh`/`build-macos-companion.sh` and staged by `stage-companion.sh`);
  - build release-profile `made-app`, the host-target `made-grok-status`, and askpass through `stage-askpass.sh` with `PROFILE=release`, with `--locked` on `windows-2025` (crt-static), `macos-15` and `macos-15-intel` (`MACOSX_DEPLOYMENT_TARGET=12.0`), and `ubuntu-22.04`;
  - run the glibc 2.35, Linux `NEEDED`, Windows import and macOS minos checks;
  - stage with `stage-desktop-package.py` and assemble companions;
  - compose the macOS bundle per 1.3;
  - pack with Velopack per target channel, with the pack version read from `[workspace.package].version`;
  - finish assets with `release-assets.py`.
- [x] 7.10 Add the installed-smoke job running `smoke-installed.py` on each target's packaged output; any failure blocks publishing (path: .github/workflows/release.yml)
- [x] 7.11 Add the post-publish upgrade check, with manual and callable triggers and read-only permissions (path: .github/workflows/upgrade-check.yml):
  - per target, and on macOS for both the `.pkg` and the `.zip` install, download the previous published stable release's asset from the production repository and install it per-user;
  - run once with the REL-10 choice seeded on and once with it seeded off, along with SSH server records and settings;
  - with no GUI instance running, run the 5.10 command-line entry to download, then again with the apply flag to update to the latest published release;
  - assert `--version` reports the latest version, the app data and the seeded REL-10 value are preserved, and the smoke line shows the production `update_repo`;
  - with only one published release, report the first-release exemption and pass.
- [x] 7.12 Call the upgrade check from the release workflow after the publish job; a failure is reported for remedy by a fixed newer release and does not modify the published release (path: .github/workflows/release.yml)
- [x] 7.13 Add the publish job, which runs after every target's build and smoke succeed (path: .github/workflows/release.yml):
  - remove any stale draft for the tag;
  - create a draft and upload all assets and `SHA256SUMS`;
  - apply the notes template;
  - set `make_latest: true` and publish.

  Any failure leaves only the draft. Confirm 7.6 passes.

## Checkpoint
- [ ] verify:
  - REL-01 "Checksums cover every asset", "One target fails" (forced failure on a test tag in a fork), "Re-run on a published tag", "Version not newer than the latest release refused", "Re-run after a failed run", "Prerelease tag refused" and "Non-tag pushes publish nothing";
  - REL-02 "Tag and workspace version mismatch" and "Tag refused on plugin mismatch";
  - REL-03 "Release blocked by a broken layout";
  - REL-08 verification of a real downloaded package during 7.11;
  - REL-12 "Post-publish upgrade check" (exemption path on the first release), "Command-line upgrade uses the same path", "GUI startup refused during command-line apply" and "Command-line apply refused while Made runs" (with 5.7).

## 8. Documentation
- [x] 8.1 Replace the "no installers … auto-update" statement in the README (path: README.md) with:
  - per-OS install and first-run steps, plus the repeat-after-update note;
  - the update opt-out;
  - `SHA256SUMS` as corruption detection only;
  - a short pointer to the release procedure in `wiki/operations/release-ci.md`.
- [x] 8.2 Document the bump-and-release procedure (explicitly scoped wiki edit) (path: wiki/operations/release-ci.md):
  - run `scripts/bump-version.sh X.Y.Z`, then commit through a pull request;
  - after merge, `git tag vX.Y.Z && git push origin vX.Y.Z`;
  - what CI does: gate, build and pack, installed smoke, approval in the protected `release` environment, publish as latest, then the post-publish upgrade check; a failed upgrade check is remedied by a fixed newer release;
  - if the tag is refused: read the gate message, fix the sites with the bump script or choose a higher version, delete the refused tag locally and with `git push origin :refs/tags/vX.Y.Z` before re-tagging, and never re-tag a published version;
  - manual maintainer steps, not automated and not configured by implementers:
    - a `v*` tag ruleset restricting create, update and delete to a maintainer bypass list; published versions are never re-tagged, by procedure;
    - the protected `release` environment with required approval;
    - GitHub immutable releases, enabled only after the final `v0.1.0` re-release;
  - the one-time rollout cleanup: delete all iteration releases first, then their tags, then re-release `v0.1.0`, and follow the proposal's per-machine cleanup.

## Checkpoint
- [x] verify REL-05 documentation clauses against the README and the notes template, and that the 8.2 procedure matches `release.yml` and `bump-version.sh`

## 9. Native acceptance and first release
- [x] 9.1 Create the release acceptance checklist and record evidence there (path: tests/e2e/release_validation.md):
  - fresh Windows 10 1809 and Windows 11 without admin;
  - fresh macOS 12+ on arm64 and x64, with both `.pkg` and `.zip`;
  - Ubuntu 22.04, Ubuntu 24.04 and current Fedora (VM) in X11 and Wayland, without libfuse or `fusermount`;
  - first-run steps and optional tools absent;
  - offline launch and warning after update;
  - apply kill points on real machines, with power loss marked unverified on any OS whose swap atomicity 1.7 did not establish;
  - About/indicator behavior;
  - installed-layout launch with components resolved.
- [ ] 9.2 After user approval, push iteration tags `v0.1.0`, `v0.1.1`, ... in the production repository as implementation needs, with notes labelled as temporary test releases, and record each release URL and per-target evidence. After verification, and only once the Phase 11 checkpoint passes and the REL-01 approval gate is configured, follow the rollout plan: delete all iteration releases, then their tags, re-release `v0.1.0` from the final commit, and record its evidence (path: tests/e2e/release_validation.md)
- [ ] 9.3 At the second published release counted from the final `v0.1.0`, confirm the 7.12 upgrade check passed and verify the in-app upgrade from the first on every target with SSH records and settings preserved (path: tests/e2e/release_validation.md)

## Checkpoint
- [ ] verify:
  - REL-01 "Tag produces a complete release";
  - REL-02 "Installed app reports the release version";
  - REL-03 "Fresh install launches with all components resolved" and "Companion integrity in the installed layout";
  - REL-04 all fresh-machine scenarios and "Optional tools absent";
  - REL-05 all scenarios;
  - REL-09 "Interrupted apply";
  - REL-12 "Post-publish upgrade check", "Failed upgrade check" (if one occurs) and "Upgrade from the previous release" at the second release.

## 10. Cleanup
- [ ] 10.1 Remove the probe crate after the 1.8 outcomes are recorded and Phase 7 is green (path: tests/release-probe/Cargo.toml)
- [ ] 10.2 Remove the probe source (path: tests/release-probe/src/main.rs)
- [ ] 10.3 Remove the probe workflow (path: .github/workflows/release-probe.yml)

## Checkpoint
- [ ] No probe files, dead references or new warnings remain; `cargo clippy --workspace` and all CI workflows are green

## 11. Security-review fixes
- [ ] 11.1 Write failing tests for the Windows hand-off (path: crates/made-platform/tests/update.rs):
  - the install lock passes from the command-line entry to the helper with no gap;
  - the helper holds the lock until its swap ends;
  - a failed helper start releases the lock and changes no installed files;
  - the helper records its final outcome in private app data where the next invocation and diagnostics can read it;
  - the install lock is OS-held and released when its holder exits, or a stale lock with a dead holder is detected and ignored;
  - in the fallback, a stale lock whose process id now belongs to an unrelated live process, detected by a mismatched start time, is still treated as stale;
  - after the helper is killed before, during and after its swap, the next GUI start succeeds on the previous or new version.
- [x] 11.2 Implement the gap-free lock hand-off, the hand-off failure path and the helper outcome record; confirm 11.1 passes (path: crates/made-platform/src/update.rs)
- [x] 11.3 Write failing tests for the command-line hand-off (path: crates/made-app/tests/update_restart.rs):
  - output states that the apply was handed off and has not completed, and the exit code is 0 only on a successful hand-off and non-zero with a clear message otherwise;
  - GUI startup is refused while the hand-off lock is held;
  - an `--update` invocation while the helper still runs reports "apply in progress", and the next one after it ends reports the recorded outcome;
  - after the command-line entry is killed during its checks, the next GUI start is not blocked and succeeds on the previous version;
  - on macOS and Linux the entry still exits when the apply is done.
- [x] 11.4 Implement the hand-off messages, exit codes, the GUI refusal during the hand-off, and reporting the previous helper outcome; confirm 11.3 passes (path: crates/made-app/src/main.rs)
- [x] 11.5 Write failing tests that the last recorded helper outcome is one row inside an existing diagnostics section (no new section), appears in copied diagnostics, and is redacted under the existing rules (path: crates/made-app/tests/diagnostics.rs)
- [x] 11.6 Show the recorded helper outcome in the diagnostics detail; confirm 11.5 passes (path: crates/made-app/src/diagnostics.rs)
- [x] 11.7 Write failing tests for AppImage extraction privacy: unit cases, plus an AppImage-level runtime test that launches the built AppImage against pre-planted entries (a directory owned by another user, a user-owned directory with any `077` bit set, a symlink). None is executed from and the symlink is not followed; the outcome is a fresh private directory or a clear exit, per the M2 decision. Enforcement may need a wrapper inside the AppImage (path: .github/scripts/test_linux_appimage_runtime.py)
- [x] 11.8 Implement the private extract-and-run target per the M2 decision; confirm 11.7 passes (path: .github/scripts/linux-appimage-runtime.py)
- [x] 11.9 Fill the M2 AppImage extraction decision and confirm the hand-off record (path: specs/changes/add-release-auto-update/proposal.md)
- [x] 11.10 Make the Windows upgrade check poll `--update` until it no longer reports "apply in progress", within a bounded wait that fails the upgrade check on timeout, then assert the recorded helper outcome and the new version (path: .github/workflows/upgrade-check.yml)

## Checkpoint
- [ ] verify REL-12 "Windows command-line apply hand-off", "Windows hand-off failure", "Killed apply never blocks startup", "Command-line upgrade uses the same path" and "GUI startup refused during command-line apply"; REL-04 "AppImage extraction directory is private"; REL-12 "Post-publish upgrade check" on Windows through 11.10

## Cross-change dependencies
`specs/changes/grok-native-hook/` lands its tasks 2.6 through 6.3 in one merge to main. This change archives before or together with it, because it cites REL-02.
- **plugin.json version:** `grok-native-hook` 2.2 sets the source `plugins/grok-made/plugin.json` to `0.1.0`, and its 2.6 `stage-grok-plugin.sh` writes the staged version per REL-02. 2.7 lands after 2.2. After that, only `scripts/bump-version.sh` (2.5) changes that field.
- **Staging and packaging:** after the 2.6–6.3 merge, 3.1–3.3, 7.7 and 7.9 consume its work:
  - 2.6 `stage-grok-plugin.sh`;
  - 5.1 `stage-companion.sh`;
  - 5.2 the 10-artifact list and hook checks in `assemble-desktop-package.py`;
  - 5.7 `build-linux-companion.sh` and 5.8 `build-macos-companion.sh`;
  - 5.3 `ci-ssh-remote.yml`;
  - 5.4 and 5.10 in `ci-artifacts.yml`;
  - cutover 6.1–6.3.

  3.3 preserves its 5.4 hook build and 5.10 packaged-hook test step.
- **grok-made path:** development and release builds resolve `<install_root>/plugins/grok-made` with no repository fallback (`grok-native-hook` 3.2). This change's staging writes there for both profiles. 4.4 and the 1.3 fallback (b) edit `crates/made-platform/src/lib.rs` after its 3.2.
- **Shared files:** `grok-native-hook` lands first and this change rebases onto it:
  - `Cargo.toml` (its 1.1 member / our 2.3, 2.5);
  - `Cargo.lock` (its new crate / our 4.1);
  - `.github/workflows/ci-artifacts.yml` (5.4, 5.10 / 3.3);
  - `.github/workflows/ci-ssh-remote.yml` (5.3 / consumed by 7.9, unchanged here);
  - `.github/scripts/stage-askpass.sh` (ours, 3.12; called by its `ci-ssh-remote.yml`, default debug kept);
  - `crates/made-platform/src/lib.rs` (3.2 / 4.4);
  - `crates/made-app/src/app.rs` (4.14 / 5.8);
  - `crates/made-app/src/ui.rs` (4.15 / 6.2);
  - `crates/made-app/src/diagnostics.rs` (4.13 / 5.6);
  - `crates/made-app/tests/diagnostics.rs` (4.12 / 5.5);
  - `crates/made-app/tests/ui_model.rs` (4.18 / 6.1).
- **First release:** REL-04 requires `grok-native-hook` to be implemented before 9.2 publishes the first release.

## Coverage map
- REL-01: 1.7, 7.1, 7.2, 7.3, 7.4, 7.5, 7.6, 7.8, 7.9, 7.13, 8.2, 9.2
- REL-02: 2.1, 2.2, 2.4, 2.5, 2.6, 2.7, 3.1, 3.2, 7.4, 7.5, 8.2, 9.2
- REL-03: 1.3, 3.1, 3.2, 5.7, 5.8, 7.7, 7.9, 7.10, 9.1
- REL-04: 1.4, 1.5, 1.6, 3.4, 3.5, 3.6, 3.7, 3.8, 3.9, 3.10, 3.11, 3.12, 7.9, 9.1, 11.7, 11.8, 11.9
- REL-05: 1.3, 1.5, 7.3, 8.1, 9.1
- REL-06: 1.2, 1.7, 1.8, 4.2, 4.3, 5.3, 5.4, 6.1, 6.2
- REL-07: 1.2, 4.2, 4.3, 5.3, 5.4, 5.7, 5.8, 5.10, 6.1, 6.2
- REL-08: 1.2, 1.8, 4.2, 4.3, 5.5, 5.6, 7.11
- REL-09: 1.7, 4.3, 5.3, 5.4, 5.5, 5.6, 9.1
- REL-10: 5.1, 5.2, 5.3, 5.4, 5.7, 5.10, 6.1, 6.2, 7.11
- REL-11: 1.2, 3.3, 4.2, 4.3, 4.5, 5.3, 5.7, 5.8, 5.10, 7.7
- REL-12: 4.2, 5.7, 5.10, 7.6, 7.7, 7.11, 7.12, 8.2, 9.2, 9.3, 11.1, 11.2, 11.3, 11.4, 11.5, 11.6, 11.10
- MODIFIED Minimal Workspace Layout: 6.1, 6.2
- MODIFIED About Presentation: 5.3, 6.1, 6.2
- MODIFIED Keyboard Focus and Accessibility: 6.1, 6.2
- License metadata (Decision index): 2.3, 3.2
