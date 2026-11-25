# Release Distribution Specification

## Purpose

This specification owns how Made is released, installed and kept up to date: tag-triggered GitHub Releases for aarch64-apple-darwin, x86_64-apple-darwin, x86_64-unknown-linux-gnu and x86_64-pc-windows-msvc, the single version source, the complete installed layout, self-contained installation on the supported OS baseline, unsigned first-run guidance, and in-app update discovery, download, integrity, apply, failure safety, opt-out and update-path continuity. The wordmark update indicator and the About Updates section are presentation owned by `specs/made-ui-ux.md`. Requirement identifiers REL-01 to REL-12 are stable for task and evidence references. "Installed build" means Made installed from a published release by the Windows installer, the macOS installer package, the macOS portable app archive extracted to a user location, or the Linux AppImage; "non-installed build" means any other build, including `cargo build`/`cargo run` output, development artifacts, update packages and raw staging trees. "Supported OS baseline" is defined in REL-04. Optional external tools are OpenCode, Grok Build, host OpenSSH and host git. Realized from the archived change `specs/changes/archive/20261125-add-release-auto-update/`; the Claude Code package clauses in REL-03 and REL-04 were handed to that archive by `specs/changes/archive/20261124-claude-code-integration/`.

## Requirements

### Requirement: Tag-triggered release with per-target assets (REL-01)

Pushing a tag named `v<version>` MUST produce one GitHub Release for that version whose assets are built with the release profile for aarch64-apple-darwin, x86_64-apple-darwin, x86_64-unknown-linux-gnu and x86_64-pc-windows-msvc. Each target MUST have its end-user downloads plus the update payload and update metadata that REL-06 to REL-09 consume: for Windows x64, an installer; for macOS arm64 and for macOS x64, separately, an installer package and a portable app archive; for Linux x64, one AppImage, which is the only Linux format. Every end-user download's file name MUST identify the version and the OS/architecture, so an end user can pick the right download without reading build details; update metadata files MAY use fixed names. Each release MUST publish a `SHA256SUMS` file listing the SHA-256 digest of every other asset in that release; it detects corruption and does not establish publisher authenticity. The release MUST become publicly visible only when every target's assets were built, packaged and passed REL-03 checks; a failure for any target MUST leave no partially published release visible to end users or update clients. Re-running the pipeline for a tag whose release is already published MUST fail rather than replace or delete published assets; re-running for a tag whose earlier run failed MUST discard that run's unpublished draft and MUST NOT publish its assets. Releases are stable versions only: a tag whose version carries a semantic-version prerelease suffix, or whose version is not greater than the newest published stable release, MUST be refused before anything is published, and each published release MUST be marked as the repository's latest release explicitly. creating, updating and deleting `v*` tags MUST be restricted to maintainers, publishing MUST pass a maintainer approval gate, and release builds MUST NOT restore build caches and MUST use third-party build steps pinned to immutable versions. Release notes MUST include the per-OS install and first-run steps required by REL-05.

#### Scenario: Tag produces a complete release

- Given the single version (REL-02) is `0.2.0` and the commit is tagged `v0.2.0`
- When the tag is pushed
- Then one GitHub Release `v0.2.0` appears with release-profile downloads for Windows x64, macOS arm64 and macOS x64 (installer package and portable archive each) and a Linux x64 AppImage, plus update data for all four targets; each download name shows `0.2.0` and its OS/architecture; and the notes contain per-OS install and first-run steps

#### Scenario: Checksums cover every asset

- Given a published release
- When a user downloads its assets and checks them against `SHA256SUMS`
- Then every asset except `SHA256SUMS` itself is listed and every digest matches

#### Scenario: One target fails

- Given a tag push where the Windows packaging step fails and the other three targets succeed
- When the pipeline finishes
- Then no release for that version is visible to end users and no update client is offered that version

#### Scenario: Re-run on a published tag

- Given release `v0.2.0` is already published
- When the pipeline is triggered again for `v0.2.0`
- Then the run fails and the published assets are unchanged

#### Scenario: Version not newer than the latest release refused

- Given `v0.3.0` is the newest published stable release
- When tag `v0.2.1` or `v0.3.0` is pushed for a new run
- Then the pipeline refuses the tag before publishing and the latest release remains `v0.3.0`

#### Scenario: Re-run after a failed run

- Given an earlier run for `v0.2.0` failed after creating an unpublished draft
- When the pipeline is re-run for `v0.2.0` and every target passes
- Then the old draft's assets are discarded, the published release contains only assets from the successful run, and it is marked latest

#### Scenario: Prerelease tag refused

- Given the single version is `0.3.0-beta.1` and the commit is tagged `v0.3.0-beta.1`
- When the tag is pushed
- Then the pipeline fails before publishing and no GitHub Release or prerelease is created

#### Scenario: Non-tag pushes publish nothing

- Given a push to main or a pull request without a `v*` tag
- When CI runs
- Then no GitHub Release is created or modified

### Requirement: Single version source and release version consistency (REL-02)

The single source of Made's version MUST be `[workspace.package].version` in the workspace `Cargo.toml`. Every other version site, namely the `version` in `plugins/opencode-made/package.json` and its tooling-maintained `plugins/opencode-made/package-lock.json`, the version constant the OpenCode plugin reports to Made, and the `version` in `plugins/grok-made/plugin.json`, MUST equal it. Bumping the version MUST require exactly one maintainer action that takes only the new version as input and updates every version site, with any `Cargo.lock` and `package-lock.json` refresh done by normal cargo and npm tooling; no other manual edit MAY be needed. CI on every push to main and every pull request MUST fail on any mismatch, naming each disagreeing site and version. The release pipeline MUST refuse a tag before publishing anything when the tag version, the workspace version or any other version site disagree, or when the version is not a valid semantic version. The installed app MUST report that same version in About and Diagnostics and to the update client, every per-target asset of the release MUST carry that same version, and release build metadata MUST identify the release profile and MUST NOT state that no installer or auto-update exists.

#### Scenario: One action bumps every version site

- Given every version site is `0.1.0`
- When the maintainer performs the bump action with `0.2.0` and makes no other edit
- Then the workspace version, both plugin manifests, the OpenCode plugin's `package-lock.json` and the plugin's reported version constant read `0.2.0`, and the CI version check passes

#### Scenario: Plugin manifest drifts in a pull request

- Given `[workspace.package].version` is `0.2.0` and a pull request changes `plugins/grok-made/plugin.json` to `0.2.1`
- When CI runs
- Then CI fails with a message naming `plugins/grok-made/plugin.json`, `0.2.1` and the workspace version `0.2.0`; a hand edit of only `[workspace.package].version` fails the same way for each site it leaves behind

#### Scenario: Tag and workspace version mismatch

- Given `[workspace.package].version` and both plugin manifests are `0.1.0`
- When tag `v0.2.0` is pushed
- Then the pipeline fails with a message naming both versions and no release is published

#### Scenario: Tag refused on plugin mismatch

- Given the tag and workspace version are `0.2.0` but `plugins/opencode-made/package.json` is `0.1.0`
- When the tag is pushed
- Then the pipeline refuses the tag, names the disagreeing manifest and publishes nothing

#### Scenario: Installed app reports the release version

- Given Made installed from release `v0.2.0` on any target
- When the user opens About
- Then the shown version is `0.2.0`, and Diagnostics reports the same version

### Requirement: Complete and launchable installed layout (REL-03)

An installed build MUST contain, at the locations the release build resolves them, the release-profile `made-app` executable, the app-owned `plugins/opencode-made` package without `node_modules`, the shipped `plugins/grok-made` package with the file set defined by `specs/changes/archive/20261123-grok-native-hook/` (realized as `Shipped Grok plugin install` in `specs/grok-session-follow.md`), which MUST include the native `made-grok-status` hook executable built for the release's target and no Python file, the shipped `plugins/claude-code-made` package with its manifest (`.claude-plugin/plugin.json`), `hooks/hooks.json`, README and the executable native hook `bin/made-claude-status` (`bin/made-claude-status.exe` on Windows) built for the release's target, the `remote/<target>/made-remote` companion for each of x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, x86_64-apple-darwin and aarch64-apple-darwin matching the assembled companion manifest hashes, the platform-specific `made-askpass` helper, and the LICENSE file. Executable files MUST keep their executable permission where the OS uses one. The installed app MUST resolve the OpenCode plugin from its installation root (not a repository or development path), find the Grok plugin, the Claude Code plugin, companions and askpass, and MUST NOT download any of them at runtime. Before a release is published, the packaged build for each target MUST be launched from its installed layout and pass the existing smoke check with the plugin, Grok hook, Claude Code hook, companion and askpass resolution confirmed, and with its update location confirmed as the production repository.

#### Scenario: Fresh install launches with all components resolved

- Given Made freshly installed from a release on each of macOS arm64, macOS x64, Linux x64 and Windows x64
- When the user launches it and opens a folder
- Then a usable terminal and explorer appear, Diagnostics reports the OpenCode plugin source as the installed location, and the Grok plugin, all four companions and askpass are found without any network download

#### Scenario: Companion integrity in the installed layout

- Given an installed build
- When its four `remote/<target>/made-remote` files are hashed
- Then each hash matches the companion manifest assembled for that release, and each companion, the askpass helper and the native `made-grok-status` and `made-claude-status` hooks is executable on Unix-like systems, and `plugins/grok-made` contains no Python file

#### Scenario: Release blocked by a broken layout

- Given a packaged target whose installed layout lacks `plugins/opencode-made/package.json`
- When the pre-publish launch check runs
- Then the check fails and REL-01 prevents the release from being published

### Requirement: Self-contained installation (REL-04)

On a supported OS baseline, an end user MUST be able to install and run Made with no prerequisite installation: no language runtime (such as Node.js, Python, .NET or Java), no package-manager dependency, no C/C++ runtime redistributable (such as the Microsoft Visual C++ Redistributable) and no separately installed installer or updater component. Everything the core app needs beyond the supported OS baseline MUST ship in the release package, and the installer and updater MUST satisfy the same rule for their own runtime needs. The core app is launch, the terminal with the user's shell, explorer, read-only preview, About and Diagnostics, and REL-06 to REL-11 updates. Optional external tools are not prerequisites: their absence MUST leave the core app usable and MUST produce only the unavailable or degraded states the realized specs already define. This rule covers the bundled plugin packages: `plugins/opencode-made`, `plugins/grok-made` and `plugins/claude-code-made`, including the Grok and Claude Code hooks, MUST NOT require anything beyond what the corresponding optional external tool itself provides; the Claude Code hook is a native executable that needs no interpreter or other prerequisite and keeps its executable bit. This depends on the now realized `Grok status hook` and `Shipped Grok plugin install` requirements in `specs/grok-session-follow.md` (archived change `specs/changes/archive/20261123-grok-native-hook/`), which replace the Python Grok hook with a native per-target executable and so remove the host python3 requirement; a release MUST NOT be published while a bundled plugin still needs a host runtime.

The supported OS baseline is: Windows 10 version 1809 or later and Windows 11, x64; macOS 12 or later on arm64 and on x64; Linux x64 with glibc 2.35 or later and a graphical desktop session. On Linux, the baseline is assumed to provide glibc, `libgcc_s.so.1`, libxkbcommon, the Wayland and X11 client libraries, EGL/GL and a Vulkan loader; Made MUST NOT depend on any other host library, and the release binaries MUST NOT require a glibc symbol version newer than 2.35. Binary inspection covers the ELF `NEEDED` entries; libraries loaded at runtime by name (libxkbcommon, Wayland, X11, EGL/GL and Vulkan) are covered by the fresh-distribution acceptance below. macOS release binaries MUST declare a minimum OS version of 12.0 or lower. Linux acceptance distributions are Ubuntu 22.04, Ubuntu 24.04 and the current Fedora release at release time. The Linux AppImage MUST launch and update without host FUSE support installed, meaning no libfuse library and no `fusermount` helper. When the AppImage extracts itself to run, the extraction target MUST be a private per-user location that other users cannot create, replace or write: a directory in the running user's home whose parent belongs to that user, named for the Made version and the image's content so that two builds of one version never share an extraction. This location is the control. As defence in depth, Made MUST NOT run from a reused extraction directory that is a symlink, is not owned by that user, or has any group or other permission bit (mode `077`) set, and MUST NOT run from an incomplete extraction, such as one cut short when extracting was interrupted; in those cases Made MUST either extract to a fresh directory owned by the user with mode `077` clear, or exit with a clear message, and an interrupted extraction MUST NOT leave Made unable to start.

#### Scenario: Fresh Windows machine

- Given a fresh Windows 10 1809 x64 machine and a fresh Windows 11 x64 machine with only the OS defaults installed (no Visual C++ Redistributable, .NET, Node.js, Python, git, OpenCode or Grok)
- When the user runs the release installer and launches Made
- Then installation and launch succeed with no missing-DLL or missing-runtime error, and the terminal, explorer and preview work

#### Scenario: Fresh macOS machine

- Given a fresh macOS 12 or later machine on arm64 and on x64 with no developer tools, Homebrew, OpenCode or Grok installed
- When the user installs Made from the release for that architecture and launches it after the REL-05 first-run steps
- Then Made launches and the terminal, explorer and preview work with no prompt to install additional software

#### Scenario: Fresh Linux desktop without FUSE

- Given fresh Ubuntu 22.04, Ubuntu 24.04 and current Fedora desktop installations, each in an X11 session and in a Wayland session, with no build packages, no libfuse or `fusermount`, OpenCode or Grok installed
- When the user downloads the AppImage, marks it executable and launches it
- Then Made launches and the terminal, explorer and preview work without installing any package

#### Scenario: AppImage extraction directory is private

- Given a pre-planted entry at the location the AppImage would reuse for extraction: a directory owned by another user, a directory owned by the user with a group or other permission bit set, and a symlink to a directory
- When the user launches the AppImage in each case
- Then Made does not run from that entry; Made either extracts to a fresh directory owned by the user with mode `077` clear and runs from it, or exits with a clear message

#### Scenario: Interrupted AppImage extraction

- Given an earlier extraction at the AppImage's extraction location that was cut short, with a file truncated or most files missing
- When the user launches the AppImage
- Then Made does not run from the incomplete extraction; it extracts afresh and starts, or exits with a clear message and starts on the next launch

#### Scenario: Dependency inspection

- Given the release binaries for each target, including the installer and updater binaries
- When their dynamic library dependencies are inspected
- Then Windows binaries depend only on DLLs provided by the supported Windows baseline, macOS binaries only on system frameworks and libraries with a declared minimum OS of 12.0 or lower, and Linux binaries' ELF `NEEDED` entries only on the listed Linux baseline libraries with no glibc symbol version newer than 2.35

#### Scenario: Optional tools absent

- Given an installed build on a machine without OpenCode, Grok, OpenSSH or git
- When the user opens a folder and uses the `+` menu, Explorer source-control view and SSH server management
- Then the core app remains usable, OpenCode and Grok rows are not offered, source-control status shows its realized failure state without blocking the file tree, and SSH reports its realized unavailable state

### Requirement: Install experience and unsigned first-run guidance (REL-05)

Each OS MUST have one documented install path that an end user can follow without a terminal, except that the Linux AppImage may need to be marked executable, in which case the notes MUST give that one step. Installation and in-app updates MUST NOT require administrator rights on any OS. On Windows, the installer MUST install for the current user, add Made to the Start menu, and let the user remove Made through the standard OS apps list. On macOS, the installer package and the portable app archive MUST each yield a Made app that launches from Finder and Spotlight. On Linux, the AppImage MUST launch as a desktop application from the downloaded file. Because builds are unsigned, the release notes and README MUST state that macOS Gatekeeper and Windows SmartScreen will warn on first run and give the exact current steps to open Made anyway, MUST state that an applied update may show the same warning again and that the same steps apply, and MUST NOT instruct users to disable Gatekeeper, SmartScreen or other OS protections globally. A repeated first-run approval after an update is accepted, documented friction and does not block a release.

#### Scenario: Windows SmartScreen first run

- Given a Windows user without administrator rights downloads the release installer in a browser
- When they run it and SmartScreen shows its unrecognized-app warning
- Then following the documented steps installs Made for that user without an administrator prompt, Made appears in the Start menu and can be removed from the OS apps list

#### Scenario: macOS Gatekeeper first run

- Given a macOS user without administrator rights downloads the installer package, and separately the portable archive, in a browser
- When they install or extract Made and first open it and Gatekeeper blocks the unsigned app
- Then following the documented per-app steps opens Made without an administrator prompt, later launches open without the warning, and no documented step disables Gatekeeper globally

#### Scenario: Linux first run

- Given a Linux user downloads the AppImage
- When they follow the documented steps
- Then Made launches from that file in a graphical session

#### Scenario: Warning after update is documented

- Given an installed build that applies an update and the OS shows its unsigned-app warning again
- When the user follows the release notes
- Then the notes describe this case and the same steps open the updated Made

### Requirement: Update discovery (REL-06)

An installed build with automatic update checks enabled, which is the default for a new install, MUST check the project's GitHub Releases at launch and then every 24 hours while running, and MUST also check when the user activates `Check for updates` in About. Made has a single stable channel: it MUST offer only a strictly newer stable version for the same OS/architecture, MUST never offer an older or equal version, and MUST never offer a GitHub prerelease. A check MUST run off the UI thread and MUST NOT delay startup, block terminal input or affect any workspace. An update check MUST send no workspace paths, file contents, user names, server records, session data or identifiers; only what an HTTPS request for public release data inherently reveals (client address, platform, architecture and installed version) MAY be exposed. Update status MUST be presented in the app only through the wordmark update indicator and the About Updates section defined by the MODIFIED `Minimal Workspace Layout` and `About Presentation` requirements; the REL-12 command-line entry's own terminal output is exempt.

#### Scenario: Newer version available

- Given Made `0.2.0` installed with automatic checks enabled and stable release `v0.3.0` published for its target
- When the launch check runs
- Then `0.3.0` is discovered and handled by REL-07 without a blocking dialog, banner or pill and without interrupting terminals

#### Scenario: Periodic check while running

- Given Made has been running for 24 hours since its last check with automatic checks enabled
- When the next scheduled check time is reached
- Then one check runs in the background

#### Scenario: No newer version

- Given Made `0.3.0` installed and `v0.3.0` is the newest stable release
- When a check runs
- Then no indicator is shown, About reports up to date, and the outcome is recorded in the bounded diagnostics log

#### Scenario: Older or prerelease versions never offered

- Given Made `0.3.0` installed, a later-published stable release `v0.2.1`, and a GitHub prerelease with a higher version
- When a check runs
- Then neither is offered

#### Scenario: Check reveals no private data

- Given an installed build with open local and SSH workspaces
- When an update check request is captured
- Then it contains no workspace paths, file names, user names, server records, session data or install identifiers

### Requirement: Background download and user-confirmed apply and restart (REL-07)

When a check discovers a newer version, Made MUST download it in the background without user action, verify it under REL-08, and only then mark it ready, which shows the wordmark update indicator and the ready state and `Restart to update` action in About. Made MUST NOT apply an update or restart without explicit user confirmation, given either in About or by invoking the REL-12 command-line entry with its explicit apply flag, and MUST NOT apply a downloaded update at quit or at launch without that confirmation. The About confirmation MUST name the target version and state that restarting closes every open workspace, terminal and running process in them, including OpenCode, Grok and SSH sessions, because workspaces are not restored. Declining MUST leave the current version running unchanged with the downloaded update kept; at the next launch Made MUST offer that kept update again without downloading it again while it is still the newest known version. If a newer version is discovered while an older one is ready, only the newest known version MUST be offered, once it is verified; until then, the older ready version stays offered. After an apply confirmed in About, Made MUST restart into the new version; the REL-12 command-line apply does not restart Made.

#### Scenario: Update downloads and becomes ready

- Given Made `0.2.0` installed and `v0.3.0` discovered
- When the background download and verification complete
- Then the wordmark shows the update indicator and About shows `0.3.0` ready with `Restart to update`, and no restart has happened

#### Scenario: User confirms update

- Given `0.3.0` is ready and two workspaces are open
- When the user activates `Restart to update` in About and confirms `Restart now` after reading that open workspaces will close
- Then Made closes its workspaces, applies the update and relaunches as `0.3.0`, and About shows `0.3.0`

#### Scenario: User declines and relaunches

- Given `0.3.0` is ready
- When the user chooses `Not now`, later quits Made normally and launches it again with no network
- Then the quit does not apply the update, Made `0.2.0` launches, and `0.3.0` is offered as ready again without a new download

#### Scenario: Newer release supersedes a ready one

- Given `0.3.0` is ready and `v0.4.0` is published
- When the next check runs and its download is verified
- Then About offers only `0.4.0`

### Requirement: Update integrity (REL-08)

Before marking an update ready or applying it, Made MUST verify that the downloaded update for its OS/architecture exactly matches the size and SHA-256 digest recorded for it in the update data of the same GitHub Release, and that the version embedded in the package equals the offered version. Made MUST use HTTPS with certificate validation for every request and MUST refuse plain HTTP. Every request, including every redirect hop, MUST go to a host on an exact allowlist limited to GitHub's release and release-asset download hosts, and requests to GitHub's web and API hosts MUST be for the project's own repository path. A mismatched, truncated, unreadable or wrongly versioned package MUST be discarded and never offered or applied. Made MUST NOT claim publisher authenticity: without code signing, integrity guarantees only that the package is the one listed by that GitHub Release, not who published it.

#### Scenario: Tampered package rejected

- Given a downloaded update whose hash differs from the release's update data
- When Made prepares to mark it ready or apply it
- Then the package is discarded, it is never offered or applied, the current version keeps running, and the failure is recorded in diagnostics

#### Scenario: Non-GitHub or HTTP source refused

- Given update data, or a redirect during a download, that points to a host outside the allowlist, to another repository's path, or to plain HTTP
- When Made attempts the request
- Then Made does not fetch from that location, nothing is applied, and the current version is unaffected

#### Scenario: Wrong embedded version rejected

- Given a downloaded package whose digest matches but whose embedded version differs from the offered version
- When Made verifies it
- Then the package is discarded and never offered or applied

### Requirement: Update failure safety (REL-09)

Update checks and updates MUST NEVER leave Made uninstalled, unlaunchable or partially updated. Offline operation, GitHub being unreachable, rate-limited or returning malformed data, no newer version, and an interrupted or failed download MUST leave the installed version launchable and fully usable, MUST NOT show an error dialog, banner, pill or wordmark indicator, and MUST be recorded in the bounded, redacted diagnostics log; the next scheduled or manual check MAY retry. The About Updates section MUST show these failures as its `About Presentation` status lines define. A failed apply, or a kill of Made or the updater at any point of apply, MUST leave either the previous version or the new version launchable on next start, never neither. Power loss is covered by the same guarantee only as far as the per-OS swap is demonstrated atomic; any OS where that is not demonstrated MUST be recorded as unverified for power loss.

#### Scenario: Launch offline

- Given an installed build with automatic checks enabled and no network
- When the user launches Made
- Then Made opens to a usable workspace in the normal startup time with no update error shown outside About, and the failed check is in the diagnostics log

#### Scenario: GitHub unreachable or malformed feed

- Given GitHub returns an error, rate limit or malformed update data
- When a check runs
- Then no update is offered, nothing blocks the user, no indicator appears, and the outcome is recorded in diagnostics

#### Scenario: Interrupted download

- Given an update download that is cut off midway
- When Made next checks or launches
- Then the partial download is never offered or applied and the installed version launches normally

#### Scenario: Interrupted apply

- Given an apply on each OS whose updater or app process is killed before the swap, during the copy, and after the swap but before relaunch
- When the user next launches Made after each kill
- Then either the previous or the new version starts and works; the installation is never left unlaunchable

### Requirement: Disabling automatic update checks (REL-10)

The user MUST be able to turn automatic update checks off and on again with the `Check for updates automatically` toggle in About; new installs start with it on. While it is off, Made MUST perform no automatic check or background download and make no update-related network request, except a single check, and the download it leads to, started by the user activating `Check for updates` or by an explicit invocation of the REL-12 command-line entry; neither exception changes the stored choice. The choice MUST persist across restarts and updates in private, user-writable app data, MUST NOT be reset by installing or updating to a newer version, and an unreadable or corrupt stored choice MUST be treated as on and reported in diagnostics.

#### Scenario: Automatic checks disabled

- Given the user has turned automatic checks off
- When Made runs and restarts over more than a day with network available and the user never activates `Check for updates`
- Then no request is made to GitHub for update data and no update is offered

#### Scenario: Manual check while disabled

- Given automatic checks are off and `v0.3.0` is published
- When the user activates `Check for updates`
- Then one check runs, `0.3.0` is downloaded and offered under REL-07, and automatic checks stay off

#### Scenario: Choice survives an update

- Given automatic checks are off and the user installs a newer release manually or applies an update
- When the new version launches
- Then automatic checks remain off

### Requirement: Non-installed builds do not self-update (REL-11)

Non-installed builds MUST NOT check for, download or apply updates, and MUST make no update-related network request. Their About Updates section MUST state that updates are not available for this build, without update controls and without presenting it as an error, and the wordmark MUST never show the update indicator. Automated runs, including `--smoke-test` and CI, MUST NOT perform update network requests, except the REL-12 upgrade check. The existing development-artifact workflow continues alongside releases, and its artifacts are non-installed builds.

#### Scenario: Developer build

- Given Made run with `cargo run` or from a development artifact
- When it runs past launch and a 24-hour interval
- Then no update request is made, no indicator is shown, and About states that updates are not available for this build

#### Scenario: Smoke test in CI

- Given the release pipeline runs the installed-layout smoke check
- When Made runs under `--smoke-test`
- Then no update network request is made

### Requirement: Update-path continuity (REL-12)

Every published release MUST be updatable in-app from the immediately previous published release on each target, starting with the second published release; the first release is exempt. An upgrade from the previous release MUST preserve the user's app data, including SSH server records, settings and the REL-10 choice, and MUST result in the REL-03 layout. This MUST be checked after publishing by an upgrade check that, on each target and for each macOS install form, installs the previous published release from the production repository's release assets, updates it through the real in-app update path to the latest published release, and confirms that the new version runs with preserved app data and the production update location; a failure MUST be remedied by publishing a newer release, and the failing published release MUST NOT be modified or replaced. The check MAY drive the update through a non-interactive command-line entry of the installed build that uses the same update client, host allowlist and verification as the About path. By default that entry MUST only check and download; it MUST apply only when invoked with an explicit apply flag, and that invocation is the confirmation REL-07 requires. It MUST refuse to apply while any Made instance of the same install is running, exiting non-zero with a clear message and changing no installed files. While applying, it MUST hold an install lock for the whole apply, and Made's GUI startup for that install MUST refuse with a clear message while the lock is held. On macOS and Linux, a command-line apply MUST exit when done without starting the GUI. On Windows, where the invoked process must exit before the swap, the command-line apply MAY complete by handing off to a helper process. In that case the install lock MUST be held continuously from the command-line entry's checks through the helper's swap with no gap, GUI startup MUST be refused for that whole period, and the entry MUST state that the apply was handed off and has not completed yet. It MUST exit 0 only if the hand-off succeeded, and on a failed hand-off it MUST exit non-zero, change no installed files and release the lock. The helper's final outcome MUST be recorded in private app data. It MUST be reported by the next command-line invocation, which reports "apply in progress" while the helper is still running, and in About's diagnostics detail as one row within the existing diagnostics sections: no new section, included in copied diagnostics, and redacted under the existing redaction rules. Neither path starts the GUI. The install lock MUST be an OS-held lock that is released when its holder exits for any reason, including a crash or kill; otherwise, a stale lock whose holder is no longer running MUST be detected and ignored, and that liveness check MUST guard against process-id reuse, for example by comparing the recorded process start time. Either way, killing the command-line entry or the helper at any point MUST NOT block the next GUI start, which MUST succeed on the previous or the new version.

#### Scenario: Upgrade from the previous release

- Given Made installed from the previous release on each target with SSH server records saved and automatic checks enabled
- When the new release is published and the user accepts the update
- Then Made relaunches as the new version with the REL-03 layout complete, the server records and settings intact, and later checks still work

#### Scenario: Post-publish upgrade check

- Given at least two published stable releases
- When the upgrade check runs for every target
- Then each target, and each macOS install form, installs the previous release from the production repository, updates through the in-app path to the latest release, and runs the latest version with preserved app data, the production update location and the seeded REL-10 choice unchanged; with only one published release the check reports the first-release exemption

#### Scenario: Failed upgrade check

- Given the upgrade check fails for the latest published release on some target
- When the maintainer remedies it
- Then the failing release's assets are unchanged, a newer release is published, and the upgrade check passes for that newer release

#### Scenario: Command-line upgrade uses the same path

- Given an installed build with a newer release published, automatic checks off, and no Made instance running
- When the command-line entry is invoked without the apply flag, then with it
- Then the first invocation only checks and downloads, the second verifies and applies under the install lock, on macOS and Linux exits when done, on Windows hands off as "Windows command-line apply hand-off" defines, never starts the GUI, and leaves the new version runnable, both use the same update client, host allowlist and verification as the About path, and automatic checks stay off; in a non-installed build the entry reports that updates are not available without any request

#### Scenario: Windows command-line apply hand-off

- Given an installed Windows build with a downloaded update and no Made instance running
- When the command-line entry is invoked with the apply flag, the user tries to start Made while the helper runs, and later invokes the command-line entry again and opens About diagnostics
- Then the entry states that the apply was handed off and has not completed and exits 0; the install lock stays held without a gap until the helper's swap ends; GUI startup is refused for that whole period; an invocation while the helper still runs reports "apply in progress"; and the next invocation after it ends, and a redacted row in an existing section of About's diagnostics detail (also present in copied diagnostics), both report the helper's final outcome

#### Scenario: Windows hand-off failure

- Given an installed Windows build with a downloaded update where the helper process cannot be started
- When the command-line entry is invoked with the apply flag
- Then it exits non-zero with a clear message, changes no installed files, releases the install lock, and a later GUI startup succeeds on the previous version

#### Scenario: Killed apply never blocks startup

- Given a command-line apply in progress, in one run with the command-line entry killed during its checks and in another with the Windows helper killed before, during and after its swap
- When the user next starts Made
- Then the lock does not block startup, because the OS released it or Made detected and ignored a stale lock whose holder is gone, and Made starts on the previous or the new version

#### Scenario: GUI startup refused during command-line apply

- Given the command-line entry is applying an update and holds the install lock
- When the user starts Made for that install
- Then startup refuses with a clear message that an update is being applied, starts no workspace, and the apply completes unaffected

#### Scenario: Command-line apply refused while Made runs

- Given an installed build with a downloaded update and a running Made instance of that install
- When the command-line entry is invoked with the apply flag
- Then it exits non-zero with a message naming the running instance, changes no installed files, and the running instance is unaffected
