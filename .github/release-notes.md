Made {{VERSION}}

## Download

| System | Download |
|---|---|
| Windows 10 (1809 or later) and Windows 11, x64 | `made-{{VERSION}}-windows-x64-setup.exe` |
| macOS 12 or later, Apple silicon | `made-{{VERSION}}-macos-arm64.pkg` (installer) or `made-{{VERSION}}-macos-arm64.zip` (portable app) |
| macOS 12 or later, Intel | `made-{{VERSION}}-macos-x64.pkg` (installer) or `made-{{VERSION}}-macos-x64.zip` (portable app) |
| Linux x64 (glibc 2.35 or later: Ubuntu 22.04, Ubuntu 24.04, current Fedora) | `made-{{VERSION}}-linux-x64.AppImage` |

Nothing else needs to be installed first. OpenCode, Grok Build, OpenSSH and git are optional. Made installs for your user account only and never asks for administrator rights. The other files in this release (`releases.*.json`, `*.nupkg`) are update data used by Made itself.

## Install

These builds are not signed, so Windows and macOS warn the first time you open Made. The steps below open Made anyway for this app only; never turn off SmartScreen, Gatekeeper or any other protection for your whole computer. The same warning can appear again after Made updates itself; the same steps apply.

### Windows

1. Run `made-{{VERSION}}-windows-x64-setup.exe`.
2. If Windows shows "Windows protected your PC", choose **More info**, then **Run anyway**.
3. Made installs for your user and starts. Open it later from the Start menu; remove it from **Settings › Apps › Installed apps**.

### macOS

Installer package:

1. Open `made-{{VERSION}}-macos-<arch>.pkg`. If macOS says it cannot verify the package, follow the first-run steps below for the package, then open it again.
2. Choose **Install for me only** when asked where to install. Made is installed in the Applications folder of your home folder.

Portable app:

1. Open `made-{{VERSION}}-macos-<arch>.zip` and move **Made** to the Applications folder of your home folder.

First run (macOS 15 and later): open Made; when macOS says it cannot verify Made, choose **Done**. Open **System Settings › Privacy & Security**, scroll to **Security**, choose **Open Anyway** next to the message about Made, then confirm **Open Anyway** and enter your login password. Later launches open normally.

First run (macOS 12 to 14): Control-click Made in Finder, choose **Open**, then choose **Open** in the dialog. Later launches open normally.

### Linux

1. Download `made-{{VERSION}}-linux-x64.AppImage`.
2. Mark it executable: right-click it, choose **Properties › Permissions › Allow executing file as program**, or run `chmod +x made-{{VERSION}}-linux-x64.AppImage`.
3. Double-click the file to start Made. No FUSE package is needed.

## Updates

Made checks this page for a newer release when it starts and once a day, downloads it in the background, and shows a small dot on the MADE wordmark when it is ready. Open **About** (click the wordmark) and choose **Restart to update** when convenient; restarting closes every open workspace. To stop automatic checks, turn off **Check for updates automatically** in About; **Check for updates** still checks once when you ask.

## Checksums

`SHA256SUMS` lists the SHA-256 digest of every other file in this release. Check a download with `sha256sum -c SHA256SUMS --ignore-missing` (Linux), `shasum -a 256 -c SHA256SUMS --ignore-missing` (macOS) or `Get-FileHash <file>` (Windows). A matching digest shows the download is not corrupted; it does not prove who published the release.
