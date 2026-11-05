# Change Spec: add-release-auto-update

Clarified delta for the new `release-distribution` domain, following the user's answers recorded in proposal.md Clarifications. It also MODIFIES `Minimal Workspace Layout`, `About Presentation` and `Keyboard Focus and Accessibility` in `specs/made-ui-ux.md`; each MODIFIED requirement below replaces the realized requirement of the same name in full, scenarios included. Requirement identifiers REL-01 to REL-12 are stable for task and evidence references. "Installed build" means Made installed from a published release by the Windows installer, the macOS installer package, the macOS portable app archive extracted to a user location, or the Linux AppImage; "non-installed build" means any other build, including `cargo build`/`cargo run` output, development artifacts, update packages and raw staging trees. "Supported OS baseline" is defined in REL-04. Optional external tools are OpenCode, Grok Build, host OpenSSH and host git.

## ADDED Requirements

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
An installed build MUST contain, at the locations the release build resolves them, the release-profile `made-app` executable, the app-owned `plugins/opencode-made` package without `node_modules`, the shipped `plugins/grok-made` package with the file set defined by `specs/changes/grok-native-hook/`, which MUST include the native `made-grok-status` hook executable built for the release's target and no Python file, the `remote/<target>/made-remote` companion for each of x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, x86_64-apple-darwin and aarch64-apple-darwin matching the assembled companion manifest hashes, the platform-specific `made-askpass` helper, and the LICENSE file. Executable files MUST keep their executable permission where the OS uses one. The installed app MUST resolve the OpenCode plugin from its installation root (not a repository or development path), find the Grok plugin, companions and askpass, and MUST NOT download any of them at runtime. Before a release is published, the packaged build for each target MUST be launched from its installed layout and pass the existing smoke check with the plugin, Grok hook, companion and askpass resolution confirmed, and with its update location confirmed as the production repository.

#### Scenario: Fresh install launches with all components resolved
- Given Made freshly installed from a release on each of macOS arm64, macOS x64, Linux x64 and Windows x64
- When the user launches it and opens a folder
- Then a usable terminal and explorer appear, Diagnostics reports the OpenCode plugin source as the installed location, and the Grok plugin, all four companions and askpass are found without any network download

#### Scenario: Companion integrity in the installed layout
- Given an installed build
- When its four `remote/<target>/made-remote` files are hashed
- Then each hash matches the companion manifest assembled for that release, and each companion, the askpass helper and the native `made-grok-status` hook is executable on Unix-like systems, and `plugins/grok-made` contains no Python file

#### Scenario: Release blocked by a broken layout
- Given a packaged target whose installed layout lacks `plugins/opencode-made/package.json`
- When the pre-publish launch check runs
- Then the check fails and REL-01 prevents the release from being published

### Requirement: Self-contained installation (REL-04)
On a supported OS baseline, an end user MUST be able to install and run Made with no prerequisite installation: no language runtime (such as Node.js, Python, .NET or Java), no package-manager dependency, no C/C++ runtime redistributable (such as the Microsoft Visual C++ Redistributable) and no separately installed installer or updater component. Everything the core app needs beyond the supported OS baseline MUST ship in the release package, and the installer and updater MUST satisfy the same rule for their own runtime needs. The core app is launch, the terminal with the user's shell, explorer, read-only preview, About and Diagnostics, and REL-06 to REL-11 updates. Optional external tools are not prerequisites: their absence MUST leave the core app usable and MUST produce only the unavailable or degraded states the realized specs already define. This rule covers the bundled plugin packages: `plugins/opencode-made` and `plugins/grok-made`, including the Grok hook, MUST NOT require anything beyond what the corresponding optional external tool itself provides. This depends on the active change `specs/changes/grok-native-hook/`, which replaces the Python Grok hook with a native per-target executable and so removes the host python3 requirement; a release MUST NOT be published while a bundled plugin still needs a host runtime.

The supported OS baseline is: Windows 10 version 1809 or later and Windows 11, x64; macOS 12 or later on arm64 and on x64; Linux x64 with glibc 2.35 or later and a graphical desktop session. On Linux, the baseline is assumed to provide glibc, `libgcc_s.so.1`, libxkbcommon, the Wayland and X11 client libraries, EGL/GL and a Vulkan loader; Made MUST NOT depend on any other host library, and the release binaries MUST NOT require a glibc symbol version newer than 2.35. Binary inspection covers the ELF `NEEDED` entries; libraries loaded at runtime by name (libxkbcommon, Wayland, X11, EGL/GL and Vulkan) are covered by the fresh-distribution acceptance below. macOS release binaries MUST declare a minimum OS version of 12.0 or lower. Linux acceptance distributions are Ubuntu 22.04, Ubuntu 24.04 and the current Fedora release at release time. The Linux AppImage MUST launch and update without host FUSE support installed, meaning no libfuse library and no `fusermount` helper. When the AppImage extracts itself to run, the extraction target MUST be private to the running user: Made MUST NOT run from a reused extraction directory that is a symlink, is not owned by that user, or has any group or other permission bit (mode `077`) set, and MUST NOT follow a symlink at that location. In that case Made MUST either extract to a fresh directory owned by the user with mode `077` clear, or exit with a clear message.

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
- Then nothing is executed from that entry and the symlink is not followed; Made either extracts to a fresh directory owned by the user with mode `077` clear and runs from it, or exits with a clear message

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

## MODIFIED Requirements

### Requirement: Minimal Workspace Layout

Made MUST present a terminal-first workspace in one native window: workspace tabs above the content area, a selected left surface that is either the terminal or the full-left read-only preview, and a full-height explorer in the right column. Persistent header icon actions MUST be icon-only buttons with tooltips and accessible labels: Open folder, `Servidores SSH`, Explorer toggle, and the optional Sessions action when shown. The Open folder action MUST live in the workspace tab strip immediately after the last visible tab and before the overflow trigger, use an app-drawn `+` glyph, reserve its width in the tab-fitting budget, and remain reachable with the overflow trigger when tabs overflow. The app-drawn `+` MUST retain its tab-strip placement and local folder/terminal/OpenCode choices while exposing the remote choices of SSH-01, including when no local compatible TUI is detected. Existing folder-picker confirmation semantics remain unchanged for local launches. The remaining right icon group MUST order Sessions (when shown), `Servidores SSH`, Explorer left-to-right, keeping Explorer rightmost. The top header MUST NOT show text labels for those icon actions; the Made identity and workspace-tab labels remain text, and workspace tabs MUST retain a close affordance. The header controls MUST float directly over the window clear/native backdrop and MUST share one vertical center and square hit targets, with vertically centered glyphs and the last icon’s glyph centered in its target. Workspace tabs MUST start at the window gutter/card left edge. The header control row MUST span the window gutter to `width - gutter` with no extra horizontal padding, so the last right-side icon’s 26/24 target, selected/hover fill, and focus ring end at the explorer/terminal card right edge with no gutter overhang. The 14px glyph MUST remain centered inside its target with normal optical padding on both sides, so its visible edge is inset from the card border. The wordmark MUST sit to the right of the tab strip immediately before the right icon group. The tab-fitting budget MUST reserve a 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark; with few tabs the visible flexible space between the last visible control and the wordmark is larger because the boundary is reserved, not measured from the last control. The wordmark-to-first-right-icon gap MUST be exactly 24px wide / 18px compact. The whole right cluster MUST be reserved so tabs, `+`, and overflow never overlap it, and the wordmark MUST NOT paint a tab fill, hover fill, or selection treatment. The MADE wordmark MUST be the persistent About trigger. While a workspace is active, activating the wordmark by pointer or keyboard MUST open and close the About card; when no workspace is active, activation MUST NOT open or toggle the About card. The wordmark MUST keep its two-tone MADE brand identity, MUST show a pointer cursor over its target, MUST expose the tooltip and accessibility name `About`, or `About, update ready` while the update indicator is shown, MUST provide a target of at least 24×24, MUST show a visible keyboard focus ring, and MUST activate on Enter or Space. While an installed build has a verified update ready to install (REL-07), the wordmark MUST show one discreet app-drawn accent dot as its update indicator; the indicator MUST NOT add a text label, banner, pill, tab fill, hover fill or selection treatment, MUST NOT change the wordmark target, the documented gaps or the reserved right-cluster width, and MUST be absent in every other state. The indicator appears only on the header wordmark while a workspace is active; the picker wordmark (the brand-only MADE wordmark on the folder picker shown when no workspace is active) never shows it, and a ready update is indicated again once a workspace becomes active. The top workspace strip and workspace tabs MUST both be 26/24 high while session-map metrics remain unchanged. Header icon actions, optional session tabs, explorer rows and picker rows share one interaction system: hover uses a `BG_HOVER` filled rounded rect with radius 6 and no border; selection uses a `BG_SELECTED` filled rounded rect with no border or accent bar; inactive icons use reduced opacity that returns to full on hover or selection. The wordmark trigger is the deliberate exception: it paints no hover or selection fill and conveys focus with its focus ring. Workspace tabs use the dedicated selected/hover/idle-fill treatment and the OpenCode icon and session status dots defined by the Brand and Workspace Tabs requirement. Expanded explorer folders MUST be marked by a small app-drawn disclosure triangle and a brighter label only, with no border, accent side bar or extra expansion fill. Explorer rows MUST use 24px height, a 4px horizontal inset, 6px radius, a 16px icon slot inset 4px within the painted row fill (slot starts 8px from the row's left edge) and text starting 28px from the row's left edge, with depth adding 14px; files keep a quiet app-drawn bullet. Text and disclosure/bullet markers MUST be vertically centered on one optical center in the row. Selection and focus in the tree MUST be conveyed by the inset `BG_SELECTED` fill and `TEXT_PRIMARY` label only, with no focus outline around the row or the tree container. When the tree container receives keyboard focus with no row selected, the first visible row MUST become selected with that same fill and primary text, without opening a preview or changing pointer row activation. Every scroll area (explorer tree, picker list, preview, About) and the terminal scrollback bar MUST use hover-only floating overlay bars: no track or thumb at rest, a thumb-only fade over about 120–160 ms when the area is hovered, expansion on bar hover/drag with a faint track only then, fade-out on leave, zero allocated width and no layout shift, and no always-on track. Routine integration, shell-running, watcher, version, branch, revision and performance statuses MUST NOT consume persistent banners, pills, path/status rows or empty session strips; technical details belong in the About card's on-demand diagnostics detail. Update checking, downloading and check failures are routine statuses under this rule; the only persistent update cue is the wordmark update indicator, and update detail belongs in the About Updates section. Real shell/input, file/root and explicitly requested action errors MUST remain contextual and non-blocking. The explorer MUST occupy the full right content column with no preview below it, MUST be hideable/restorable through the header Explorer toggle, and MUST have a resizable width constrained to leave a usable left pane. Explorer width MUST be adjusted only by pointer-dragging its divider; the explorer header MUST NOT contain Wider, Narrower, Reload or Hide text actions and MUST NOT offer a routine manual refresh control. At rest the divider MUST NOT paint a persistent line or grip; hover or drag MAY fade in a discreet thin line/grip using muted `BORDER_CONTROL` (and `ACCENT` at restrained opacity while dragging) while retaining a comfortable pointer hit area, with a fade-out on leave. Hiding it MUST give the left surface the reclaimed width without killing the shell, resetting the root or losing tree/preview state; restoring MUST reconcile changes that occurred while hidden. Width and visibility need only be retained for that live workspace, not across app restart. Keyboard reachability, accessible names, visible focus, target sizing and the pointer-only divider exception are owned by the Keyboard Focus and Accessibility requirement; the wordmark trigger behavior is owned here; essential controls MUST NOT clip or introduce noisy default technical chrome.

#### Scenario: Quiet shell workspace

- Given a newly opened folder and no running OpenCode
- When the default workspace is displayed
- Then a usable terminal and full-height right explorer appear without an empty preview, empty session strip, degraded/waiting banner, technical status bar or routine status pills; the header controls float over the clear/native backdrop without a persistent band and show only icon-only Open folder, `Servidores SSH` and Explorer actions with accessible labels, no Diagnostics icon, and the MADE wordmark as the About trigger; and the explorer header shows no text action buttons

#### Scenario: Icon-only header controls

- Given a live workspace at 1280×800 or 720×480
- When the user inspects or keyboard-navigates the top header
- Then Open folder, `Servidores SSH`, Explorer toggle and the visible optional Sessions action are icon-only with matching tooltips and accessible labels; the MADE brand text remains as the About trigger with the matching tooltip and accessibility name `About`; no persistent action text labels are rendered; workspace tabs retain their close affordance; and every one of these actions is reachable and visibly focused by keyboard

#### Scenario: Header placement and order

- Given a live workspace at 1280×800 or 720×480, with tabs that fit and a case that overflows
- When the header lays out
- Then workspace tabs start at the window gutter/card left edge, the last right-side icon’s 26/24 target, selected/hover fill, and focus ring end at the card right edge (`width - gutter`) with no gutter overhang and the 14px glyph centered inside the target with normal optical padding, the header row has no extra horizontal padding, the top strip and tabs are 26/24 high with the card top at 42/36 and one 8/6 pane gap above and below the strip, the tab-fitting budget reserves the documented 24px wide / 18px compact gap from the tab strip's right boundary to the wordmark (the visible space after the last visible control may be larger), the wordmark-to-first-icon gap is exactly 24px wide / 18px compact, the whole right cluster is reserved so tabs/plus/overflow never overlap it, the app-drawn `+` Open folder action follows the last visible tab and precedes the overflow trigger, the overflow trigger and `+` both remain reachable, the right icon group orders Sessions (when available), `Servidores SSH`, Explorer with Explorer rightmost and no Diagnostics icon while retaining tooltips, accessible names, minimum targets, and keyboard focus, and the MADE wordmark stays immediately before that group with a target of at least 24×24 and the `About` tooltip and accessibility name

#### Scenario: Wordmark About trigger

- Given a live workspace at 1280×800 or 720×480, then the shell with no active workspace
- When the user activates the MADE wordmark by pointer, then by keyboard focus plus Enter, then by keyboard focus plus Space, then without an active workspace
- Then each activation while a workspace is active opens and closes the About card between closed and open, and activation without an active workspace does not open or close the card

#### Scenario: Unified hover and selection states

- Given header icon actions, the MADE wordmark About trigger, optional session tabs, workspace tabs, explorer rows, and picker rows
- When each is hovered, selected, focused, or at rest
- Then header icons, session tabs, explorer rows, and picker rows paint their documented borderless radius-6 `BG_HOVER` hover and `BG_SELECTED` selection fills; the MADE wordmark paints no `BG_HOVER`, `BG_SELECTED`, `BG_TAB_HOVER`, or `BG_TAB_SELECTED` fill and conveys keyboard focus with its visible focus ring; workspace tabs use the translucent idle fill and opaque `BG_TAB_HOVER` hover fill with no stroke, and the opaque `BG_TAB_SELECTED` fill with the faint 1px inside stroke; selected icon toggles paint no border or accent bar; tree and picker selection paint no focus outline; inactive tabs/icons use the reduced-opacity treatment; and focus rings, tooltips, accessible names, and minimum targets remain intact on every control except the deliberate tree/picker row exception

#### Scenario: Explorer folder and row presentation

- Given expanded and collapsed folders, files, hover, selection, and keyboard focus in the explorer tree
- When the tree renders
- Then expanded folders use a brighter label, rows use 24px height, a 4px horizontal inset, 6px radius, a 16px icon slot inset 4px within the painted row fill (slot starts 8px from the row's left edge) and text 28px from the row's left edge with 14px per depth, explorer entry icons are owned by the realized `specs/explorer-file-icons.md` specification, which is authoritative for their resolution and rendering, and replace the former app-drawn file bullet and folder disclosure triangle in that 16px slot, entry icons and labels share one vertical optical center, selection/focus uses only the inset `BG_SELECTED` fill plus `TEXT_PRIMARY` text, keyboard focus with no selection selects the first visible row without opening a preview, and no border, focus outline, accent side bar or extra expansion fill is painted

#### Scenario: Hover-only fading scrollbars

- Given overflowing explorer, picker, preview, diagnostics and terminal scroll content
- When the area is at rest, hovered, or its bar is hovered or dragged
- Then no track or thumb is painted at rest, the floating overlay bar fades in over about 120–160 ms as a thumb when the area is hovered, expands and strengthens on bar hover/drag with a faint track only then, fades out on leave, allocates zero width, and never shifts layout

#### Scenario: Explorer hide and restore

- Given an expanded tree and a running shell, with or without an open preview
- When the user activates the header Explorer toggle to hide, files change on disk, and the user activates it again to restore
- Then the left surface reclaims/releases its width, shell and preview state survive, the restored tree reflects current files while preserving valid expansion/selection, and the toggle remains reachable and focused by keyboard

#### Scenario: Explorer resize and compact layout

- Given either a 1280×800 or 720×480 window
- When the user pointer-drags the explorer divider or navigates the rest of the layout by keyboard
- Then its full-height width changes within usable bounds, the divider is invisible at rest and fades in a thin line/grip on hover or drag, the active left surface reflows without terminal corruption, no Wider/Narrower/Reload/Hide text action is present, essential controls have visible focus and do not clip, and the pointer-only divider is not a keyboard stop

#### Scenario: Explorer header action removal

- Given a visible explorer header
- When the user inspects or navigates it by pointer or keyboard
- Then no Wider, Narrower, Reload or Hide text action is present, and no routine manual refresh control is offered for normal live updates

#### Scenario: Hover-only explorer divider

- Given a visible explorer at rest
- When the divider is not hovered or dragged
- Then no vertical divider line or grip is painted; when hovered or dragged, a thin line/grip fades in while the pointer hit area remains usable, and a mere hover does not change the stored per-workspace width

#### Scenario: Explorer splitter fade

- Given a visible explorer splitter with its 8px pointer hit area
- When the pointer hovers, starts dragging, releases, or leaves
- Then the splitter paints nothing at rest, fades in over about 140 ms to a discreet 1px line and 6×32 grip using muted `BORDER_CONTROL` at partial opacity on hover, uses the green `ACCENT` at a restrained opacity while dragging, fades out on leave, keeps the resize cursor and “Drag to resize explorer” tooltip, remains pointer-only (not a keyboard stop), and never paints a bright permanent line

#### Scenario: No manual refresh control

- Given a visible explorer while files change on disk
- When the user looks for refresh actions
- Then no routine manual Reload control exists and creates, deletes and renames still appear automatically through the watcher

#### Scenario: Quiet integration chrome

- Given OpenCode is absent, inactive, disconnected, degraded, or has exited normally
- When the workspace chrome is inspected
- Then no routine degraded/waiting/unsupported banner or status pill is shown, and integration detail remains available only through the About card's on-demand diagnostics detail

#### Scenario: New remote actions preserve existing shell chrome

- Given a shell-only workspace and then a workspace with Sessions available at both responsive sizes
- When the header and `+` are inspected with tab overflow and no local OpenCode installed
- Then SSH management and remote choices remain reachable as defined above, local creation still works, Explorer stays rightmost, the expanded cluster is reserved in the fitting budget, and no routine technical banner or empty session strip is added

#### Scenario: Update-ready indicator

- Given an installed build at 1280×800 and 720×480, first with no update ready, then while an update is checking or downloading, then with a verified update ready, with tabs that fit and a case that overflows
- When the header lays out and the user hovers and keyboard-focuses the MADE wordmark
- Then no indicator is shown until the update is ready; once ready, one discreet accent dot appears on the wordmark with no text label, banner, pill or fill, the wordmark target, gaps, reserved right-cluster width and tab fitting are unchanged, and the tooltip and accessibility name read `About, update ready`; with no active workspace the picker wordmark shows no indicator, and reopening a workspace shows it again; a non-installed build never shows the indicator

### Requirement: About Presentation

Made MUST present About as an opaque floating card opened by the header MADE wordmark. About MUST show the MADE wordmark, the Made application name and version, a short product description, the Updates section defined below, and a labeled `Show diagnostics` button. The status pill, reason, copy diagnostics actions, notices, sectioned connection/session/worktree/version/metrics/log detail, bounded log block, previous-log row, active-log path/copy row, and other technical detail MUST remain hidden until the user activates `Show diagnostics`; activating it MUST reveal the existing Diagnostics content in the same card and change the button label to `Hide diagnostics`, which hides that detail again. While diagnostics are revealed, the toggle MUST use the active accent treatment; while hidden, it MUST use the secondary treatment. The diagnostics detail MUST reveal with a smooth fade and height growth over about 220 ms and reverse when hidden. Closing and reopening About MUST return it to the default About-only state with diagnostics hidden. The About card MUST be a fixed, centered, non-resizable, non-movable card with no resize or move affordance. While diagnostics are hidden it MUST use a compact height; when diagnostics are revealed it MUST grow to use the available application height, with the diagnostics body taking the remaining space, and return to the compact height when hidden or reopened. The Updates section MUST state the update status in one short line: for a non-installed build, that updates are not available for this build, with no update controls; otherwise one of up to date with the time of the last successful check, checking, downloading the named version, the named version ready to install, that the last check could not reach the update service, that the download of the named version failed, or that the named version failed verification and was discarded. For an installed build it MUST offer a `Check for updates` button that starts one check and is unavailable while a check or download is in progress, a `Check for updates automatically` toggle that shows and changes the REL-10 choice, and, while a verified update is ready, a `Restart to update` button. Activating `Restart to update` MUST show, inside the About card, an inline confirmation that names the target version and states that every open workspace, terminal and running session, including OpenCode, Grok and SSH sessions, will close and will not be restored, with `Restart now` and `Not now` actions; `Not now` or closing About MUST dismiss the confirmation and leave the current version running with all workspaces untouched, and reopening About MUST NOT show it. The compact height MUST fit the Updates section and its inline confirmation. The About card MUST remain usable at the documented responsive sizes, and the diagnostics toggle and every Updates control MUST be keyboard reachable with a visible focus ring and matching accessible name.

#### Scenario: About default content

- Given a live workspace at 1280×800 or 720×480
- When the user opens About from the MADE wordmark
- Then the card shows the MADE wordmark, the Made application name and version, the short product description, the Updates section with its status line and the controls available for that build, and a `Show diagnostics` button in the secondary treatment, while the status pill, diagnostics sections, bounded log block, previous-log row, active-log row, and copy diagnostics actions remain hidden

#### Scenario: About diagnostics reveal and reset

- Given About is open with diagnostics hidden
- When the user activates `Show diagnostics`, then `Hide diagnostics`, then closes and reopens About
- Then the first activation reveals the existing Diagnostics detail in the same card, changes the button to `Hide diagnostics`, and switches that button to the active accent treatment; the second hides that detail, returns the button to the secondary treatment, and restores the compact card; and reopening About shows only the default About content with diagnostics hidden

#### Scenario: About diagnostics reveal animation

- Given About is open with diagnostics hidden at 1280×800 or 720×480
- When the user activates `Show diagnostics`, then `Hide diagnostics`
- Then the detail fades in while the card grows smoothly over about 220 ms, the reveal passes through intermediate heights rather than jumping, and hiding reverses the animation back to the compact About card

#### Scenario: About height and resize behavior

- Given About is open at 1280×800 or 720×480
- When the user inspects the card with diagnostics hidden, then reveals diagnostics, then hides them again
- Then the card has no resize or move affordance, stays centered, uses a compact height while diagnostics are hidden, grows to use the available application height when they are revealed, and returns to the compact height when they are hidden or About is reopened

#### Scenario: About Updates controls in an installed build

- Given an installed build at 1280×800 or 720×480 with no update ready
- When the user opens About and reaches the Updates controls by pointer and keyboard
- Then the status line shows up to date, checking, that the last check could not reach the update service, that a download failed, or that a version failed verification and was discarded, as applicable; `Check for updates` and `Check for updates automatically` are reachable with visible focus and matching accessible names; `Check for updates` is unavailable while a check or download runs; changing the toggle changes the REL-10 choice; and no `Restart to update` button is shown

#### Scenario: Restart confirmation inside About

- Given an installed build with verified update `0.3.0` ready and open workspaces
- When the user activates `Restart to update`, then `Not now`, then activates it again and closes About, then reopens About
- Then each activation shows an inline confirmation in the About card naming `0.3.0` and stating that every open workspace, terminal and running session, including OpenCode, Grok and SSH sessions, will close and not be restored, with `Restart now` and `Not now`; `Not now` and closing About dismiss it with all workspaces untouched; and reopening About shows the ready state without the confirmation

#### Scenario: About in a non-installed build

- Given a development or `cargo run` build
- When the user opens About
- Then the Updates section states that updates are not available for this build, shows no update controls and no error treatment

### Requirement: Keyboard Focus and Accessibility

Made MUST make every persistent interactive control keyboard reachable with a visible focus state, never relying on fill change alone. Icon-only actions MUST expose matching tooltip and accessibility-name metadata; the MADE wordmark About trigger MUST expose the matching tooltip and accessibility name `About`, or `About, update ready` while the update indicator is shown, show a visible focus ring, and open or close the About card on Enter or Space while a workspace is active. Workspace and optional session tabs MUST support keyboard entry, Left/Right movement, and Enter/Space activation; the terminal MUST remain one focus stop; the explorer tree, preview actions/return, picker, and the About card's diagnostics detail MUST remain keyboard operable. Focusing the tree before any row is selected MUST select the first visible row (fill plus primary text only, no preview). The explorer tree row/container and picker rows are a deliberate focus-outline exception: while focus moves there, it MUST be conveyed by the inset `BG_SELECTED` fill plus `TEXT_PRIMARY` text (luminance and text, not hue alone) with no outline; every other persistent control, including the wordmark trigger, the About diagnostics toggle, the picker card, and picker buttons, MUST keep its visible focus ring. The wordmark's focus cue MUST be its visible focus ring, not a fill change. The pointer-only explorer divider MUST NOT be a keyboard stop; hide/restore remains the keyboard path. Interactive targets MUST be at least 24×24, text MUST not fall below 10px, and status MUST use shape plus text plus color. Platform screen-reader exposure MUST NOT be claimed while the accesskit integration is absent.

#### Scenario: Visible focus on persistent controls

- Given a keyboard-only walkthrough at 1280×800 and 720×480
- When focus moves through the MADE wordmark About trigger, header actions, workspace/session tabs, tree, preview actions, picker, and the About card's diagnostics detail
- Then every persistent keyboard-reachable control shows a visible focus ring except tree rows/container and picker rows, which show only the inset `BG_SELECTED` fill and `TEXT_PRIMARY` text; the wordmark shows its focus ring while painting no fill and opens or closes About on Enter or Space while a workspace is active; a tree container focused before any selection selects its first visible row without opening a preview; the pointer-only divider is skipped, and focus never lands on a clipped or hidden control

#### Scenario: Icon actions expose accessible names

- Given the icon-only Open folder, Explorer, and optional Sessions actions plus the MADE wordmark About trigger and the About card's diagnostics toggle
- When each is inspected by keyboard or accessibility metadata
- Then each icon-only action has a matching tooltip, accessible name, minimum target, and visible focus; the wordmark trigger has the matching tooltip and accessibility name `About`, or `About, update ready` while the update indicator is shown, a target of at least 24×24, and a visible focus ring; the diagnostics toggle has a matching accessible name and visible focus; and no action relies on its icon or brand text alone as its label

#### Scenario: Keyboard workspace and session navigation

- Given multiple workspace tabs and an open optional session map
- When the user enters the tab strip, moves Left/Right, and presses Enter/Space
- Then the intended workspace or session activates without leaking focus or input to an inactive surface, and the terminal remains a single focus stop

#### Scenario: Status cues do not rely on color

- Given busy, attention, unread, renaming, connected, degraded, shell-exit, tree-error, and selected states
- When the UI is inspected without color or through a colorblind simulation
- Then each state retains a shape, text, or distinct non-color cue in addition to color, selected/active tabs use the 13.0px label cue versus 12.5px inactive labels, and active tabs do not rely on a dot/hollow marker
