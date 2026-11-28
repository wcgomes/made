# Security

Made is a local-first desktop application, not a multi-user service. Its main trust boundaries are the loopback OpenCode bridge, the user's read-only OpenCode config, the workspace filesystem, untrusted text and clipboard content arriving from OpenCode or the terminal, URL open requests from pane children, repository configuration read by git, and SSH servers browsed from the Open workspace screen. This page records the implemented model and accepted v1 residuals. Normative requirements are in [OpenCode Session Follow](../specs/opencode-session-follow.md).

## Local bridge

The bridge exists once per workspace tab. It binds `127.0.0.1:0` only and serves the exact path `/made`. There is no wildcard bind, IPv6 wildcard, external interface, or remote access path.

- **Authentication:** each workspace generates a 256-bit token from OS randomness. The plugin sends it in `hello`; comparison is constant time. A valid `hello` is required within one second of the accepted socket, including the WebSocket handshake.
- **No pre-auth state:** state is not sent before authentication. Missing/invalid credentials close with `4401`; protocol or OpenCode major mismatch with `4402`; handshake timeout with `4403`.
- **Pane binding:** the server expects the workspace's pane id and rejects cross-pane hello before replacement or disclosure.
- **One authenticated connection:** a newer valid connection for the same pane replaces the old one with `4409`. One pending state snapshot per epoch is kept and replaced in place.
- **Request hygiene:** path must be `/made`; `Host` must name the accepted loopback authority; requests carrying `Origin` are rejected.
- **Bounds:** messages cap at 256 KiB (`1009`); malformed JSON closes with a protocol error. Event queues are bounded with reserved lifecycle capacity; unauthenticated notices have a separate small budget.
- **Token handling:** `Token` never renders its secret through `Debug`/`Display`; bridge events and diagnostics carry at most a fingerprint. Logs and copied diagnostics redact token-shaped values.

The token is passed to the plugin through launch configuration/child environment. See [OpenCode bridge](integration/opencode.md) for composition details.

## OpenCode config immutability

- `made-bridge::config` reads the user's `cli.json` and any existing `OPENCODE_CLI_CONFIG_CONTENT` read-only, then composes a launch-only JSON string in memory.
- The app appends exactly one Made plugin entry and forces `tabs.mode: "on"`/`tabs.scope: "global"` for that launch. User plugin entries and order are preserved; duplicate Made entries are removed by identity.
- No user config file is written. The plugin path must canonicalize inside the app-owned root and contain the required package files.
- `ConfigGuard` hashes the user config before the session and verifies it after shutdown. The result is diagnostic evidence, not a rewrite path. A composition failure leaves the shell environment untouched and OpenCode runs without the plugin.

## Filesystem and path guard

[`made-explorer::paths`](../crates/made-explorer/src/paths.rs) canonicalizes the root and every requested target, resolves symlinks, and compares component-wise against allowed roots. Lexical `..` is rejected even if it would resolve back inside the root. Symlink escapes and traversal are rejected. A future explicit user grant can add another canonical root; v1 exposes no “open outside root” UI.

Only `made-explorer` reads workspace file contents for display. Read-only git metadata goes through a bounded allowlist in [`made-app::worktree`](../crates/made-app/src/worktree.rs), which wraps the shared runner in [`made-remote::git`](../crates/made-remote/src/git.rs) (also used by the server companion); see [Hardened git reads](#hardened-git-reads). The bridge config code is the only reader of the user's OpenCode config.

## Hardened git reads

Selecting a folder on the Open workspace screen, or opening a tab's git view, runs `git status` and metadata reads on a repository the user may not trust (a clone, a server checkout). Repository configuration can make those reads execute programs: a `core.fsmonitor` hook, a filter driver's `clean`/`process` command, a partial clone's lazy fetch that starts a transport through `core.sshCommand` or `remote.<name>.uploadpack`, and `submodule.*.ignore=none`, which walks into submodule worktrees. Every read therefore forces overrides on the command line (which beats every config file), scrubs inherited `GIT_*` redirection variables, and neutralizes repository-scoped filter drivers after a scope query. The flags, variables and fail-closed rules are the `Hardened read-only git reads` requirement in [`made-ui-ux.md`](../specs/made-ui-ux.md); the single implementation is `HARDENED_CONFIG`, `SCRUBBED_ENV` and the filter-driver query in [`made-remote::git`](../crates/made-remote/src/git.rs), so local and remote reads behave the same.

Accepted residuals:

- **Scope-read gap:** the driver-scope query and the status read are two git runs; a same-user process that rewrites the repository config between them is not caught.
- **Old git, lazy fetch:** git before 2.45 ignores `GIT_NO_LAZY_FETCH`; the `protocol.*.allow=never` overrides still stop every transport, so no fetch command runs.
- **Old git, drivers:** git before 2.26 has no `--show-scope`, so every configured driver is neutralized, including a trusted global git-lfs; LFS files can then show as modified.
- **Repo-local LFS:** `git lfs install --local` or any repository-scoped `filter.lfs.*` key is treated as untrusted, so LFS-tracked files with dirty stat info can show as spurious modifications.
- **Locale-dependent detection:** recognizing an old git relies on its English "unknown option" usage text; the runner sets `LC_ALL=C` for that reason, and any other failure fails closed.
- **Not enumerated:** Made overrides the known execution vectors instead of enumerating the whole repository config; a future git setting that runs a program during status is not covered until it is added.

## Remote browse grant

Selecting an SSH host on the Open workspace screen opens a browse grant through the server's companion (SSH-12 in [`ssh-remote-workspaces.md`](../specs/ssh-remote-workspaces.md)). It is scoped to one screen instance and one server user, separate from every tab's root grant, and ends on screen close, host change or record removal without touching tabs. It lists folders, checks a directory and runs the hardened git reads; it never reads file contents, writes, or starts a session. Paths are validated on the server and never resolved on the host (guarded by `tests/remote_workspace.rs`, see [Terminal-first runtime](runtime/terminal-first.md#ssh-remote-runtime)); symlinks are reported as entries but never followed. Replies from a previous host, epoch or target are discarded, and one git read per grant is in flight at a time.

## SSH password cache

SSH authentication stays with the host's OpenSSH and the `made-askpass` helper (SSH-01, SSH-08). To avoid re-prompting the same server, [`credential_cache.rs`](../crates/made-app/src/credential_cache.rs) keeps a typed server password in process memory only, never on disk, in logs, argv or environment:

- It is cached only after the attempt that used it authenticates (tab connect, browse grant open, staging, probe).
- The key is the saved record plus its exact `[user@]host[:port]`, so an edited record or another server never receives it.
- A cached value is sent at most once per attempt; a rejection forgets it and the user is prompted, and a failed or cancelled attempt forgets the value it sent. A stale rejection cannot evict another tab's fresh password.
- Prompt cards belong to the attempt that raised them, and an expired prompt is never answered.
- Only password prompts participate; key passphrases and host-key approvals keep their one-shot flows.

## Read-only preview

- Preview has no edit/save/write API, no dirty state, no LSP, and no diff path. Save shortcuts are inert while preview has focus; terminal Ctrl+S remains XOFF.
- Files over 2 MiB are rejected with a size message. Binary detection checks for a NUL byte in the first 8 KiB; invalid UTF-8 gets a safe message rather than replacement noise. A missing/unreadable/deleted file gets a contextual error.
- Preview text is rendered as plain text, never HTML or executable content.
- Reads happen off the UI thread and are generation-gated. A root change clears the preview and invalidates stale reads.
- A same-user process can replace a path between canonical check and open (TOCTOU), or supply a FIFO/device. An existing FIFO/device is rejected before open and the opened handle is rechecked, but the race is not atomic. V1 accepts this under the same-user exclusion.

## Untrusted text and input

- Session titles, paths, error strings, and log text are treated as untrusted. Central sanitization replaces controls/newlines with spaces and removes bidi/format controls before display/copy.
- Embedded legacy/C1 bracketed-paste markers are replaced with spaces to a fixed point, so overlapping input cannot reconstruct a marker. This is input hardening, not a general escape filter; pasted control characters still reach the child unless the child handles them.
- Terminal key semantics are preserved: Ctrl/Cmd+C copies only with a selection, otherwise Ctrl+C remains SIGINT on Linux/Windows; Ctrl+X remains CAN; Ctrl+S remains XOFF; macOS Cmd+C without selection is ignored. egui clipboard shortcuts stay disabled. Ctrl/Cmd+V forwards `0x16` when the clipboard holds an image so the child TUI reads it natively; Ctrl/Cmd+Shift+V saves the image as a private PNG and pastes its absolute path.
- Clipboard images use the private temp directory: `clipboard-images/` is `0700`, each PNG is created `0600` with `create_new`/`O_NOFOLLOW` on Unix, and raw RGBA input is bounded to 128 MiB and 16384 px per side. Image bytes and clipboard text are never logged; `Debug` reports dimensions and byte length only.
- App-generated clipboard PNGs are retained for at most 7 days and at most 64 files. Cleanup runs after a successful save and when a workbench starts, deletes oldest first, preserves the just-saved image, skips symlinks and unrelated files, and ignores deletion failures so the paste remains usable.
- OSC 52 from a pane child (local or SSH) may only write text to the OS clipboard, bounded to 1 MiB decoded; queries are refused so no program can read the clipboard, and the text is never logged. Residual: vte buffers the raw OSC unbounded before dispatch. Details: [Clipboard copy](runtime/terminal-first.md#clipboard-copy-osc-52).
- The image-path fallback is app-generated, absolute, and UTF-8; a non-UTF-8 temp path is refused instead of pasted lossily.
- The shell is never asked to evaluate cwd report contents, and Made does not scrape prompts or screen contents, or inject discovery commands into a running shell. Made consumes structured status-protocol metadata (OSC 7501) and reads the window title only as a sanitized tooltip, capped at 120 characters, while a `claude-code` OSC 7501 status is live in that tab. A title is never used for status or identity and never trusted as a path; its one exception is selecting, by exact normalized name, one of the user's own background sessions listed by `claude agents --json`, whose listed directory is then validated. Accepted residual: an unrelated title equal to exactly one background session's name attaches that session's directory. The OS window title is always `Made` and never derived from a tab's terminal title. Made runs no `claude` subcommand other than that listing, without a shell, and only in a short burst after a tab starts owning a live session, after the user submits input to such a tab (the input is only checked for Enter or a newline, never kept or logged) or after that tab's title, as seen while a `claude-code` status is live, changes (without a live status a title change triggers nothing and is not kept), with runs at least ~0.5 s apart; an idle session runs none. Each listing run is bounded: one app-wide worker, one run at a time and none while the previous run's output reader is still active, output capped at 1 MiB, led on Unix by its own process group, which is killed on the ~2 s timeout, at app shutdown (waiting at most ~1 s) and when output is still held open ~200 ms after the run exits, never Made's own group; a listing whose run started more than about 5 s ago is ignored, and a malformed entry is skipped on its own. Over SSH the listing runs on the server with one worker per server user, and Made never reads or writes the remote `~/.claude/**`; see [Claude Code SSH](integration/claude-code.md#ssh). See [ADR 0008](decisions/0008-claude-code-osc7501-session-plugin.md).

## URL open requests

A child's `$BROWSER <url>` is untrusted terminal content and never opens anything by itself; the user must choose Open on the card. Contract: "URL Open Confirmation Card" in [`made-ui-ux.md`](../specs/made-ui-ux.md) and SSH-08 in [`ssh-remote-workspaces.md`](../specs/ssh-remote-workspaces.md); mechanism in [URL open requests](runtime/terminal-first.md#url-open-requests).

- **Channel:** out of band, over a same-user Unix socket or named pipe per pane, not the PTY. Each launch gets a fresh `MADE_URL_TOKEN`; a stale or foreign token is refused.
- **Validation:** helper and app both accept only absolute `http`/`https` URLs within the spec's bounds; refused requests make the helper fail so the TUI prints the URL.
- **Remote:** requests reach only the attached client and are never replayed after reattach or resnapshot; the server cannot open a host browser without a host click.
- **Opener:** runs the platform opener with the URL as one argument, never a shell, with `MADE_URL_*` stripped and `MADE_ORIGINAL_BROWSER` restored so it cannot loop back into a card.
- **Anti-spoofing:** the host is always shown whole and the full URL is never truncated; Open stays disabled until the host fits, and Open/Copy arm only after a short delay so a click cannot land on a replaced URL. The dialog has no shortcuts and never takes keys while the terminal has them, so an Enter typed for the TUI cannot open a link.
- **Logging:** diagnostics record counts only, never a URL.
- **Limit:** a `BROWSER` assigned in the user's shell rc files bypasses the card; that is the user's own browser command, not a Made path.

## Private state and lifecycle

- App temp is per-user (Linux `$XDG_RUNTIME_DIR/made-<uid>` or data-dir fallback; macOS/Windows temp under `made-<user>`), created/validated `0700` on Unix, rejecting pre-existing symlinks/non-directories. Diagnostics logs use a `0600` active file (`integration.log`), one `0600` previous file (`integration.log.1`), and a persistent `0600` lock (`integration.log.lock`) in the `0700` log directory. A startup preflight tightens an existing regular `.1`, warns about an unsafe `.1` or lock path, and every successful append re-tightens a regular `.1` under the lock. An unsafe lock path refuses every append; an unsafe `.1` refuses only when rotation is required. Current-version cooperative appends keep the active log at most 1 MiB; a mixed-version or non-cooperating writer can exceed that bound. Rotation never truncates, rewrites, copies, or deletes existing log content; `.1` may exceed 1 MiB only when it preserves a legacy oversized active file. On POSIX the replacement rename is atomic; Windows replacement/lock behavior is cross-compiled but unverified. `write_all` is one call but not failure-atomic on partial I/O; the previous log is preserved after rotation and the active bound still holds for cooperative appends. A lock wait is bounded to one second per workspace; closing N tabs can wait up to roughly N seconds. The Diagnostics card does not stat `.1` per frame; it stats on cache rebuilds and in a one-second probe while open. Final-exit write/rotation failures are reported on stderr only; Diagnostics cannot show an issue raised after the last UI frame. Symlink and non-regular paths are rejected for the active, previous, and lock paths.
- Shutdown captures the selected workspace, cwd capability/reason, versions, and metrics before releasing the terminal, so persisted diagnostics retain context.
- Bridge events from a foreign workspace or connection are ignored and cannot release or reset a healthy authenticated workspace.
- There is no telemetry and Made transfers no workspace, session, or file state off the device. The only outbound request is the release update check; see [Update trust model](#update-trust-model).

## Update trust model

Installed builds (not `cargo run` or development artifacts) check GitHub Releases at launch and every 24 hours unless the user turns the check off in About. Decision: [ADR 0007](decisions/0007-github-releases-velopack-unsigned.md).

- **Request content:** `GET /wcgomes/made/releases/latest/download/releases.<channel>.json` with only `host`, `user-agent: Made/<version> (<os>; <arch>)`, `accept` and `accept-encoding`. No query string, cookie, or identifier. Velopack's own sources send an install identifier, so Made uses its own update source in `made-platform`.
- **Transport:** HTTPS only. An exact host allowlist is enforced on every redirect hop: `github.com` (pinned to the project repository path), `release-assets.githubusercontent.com`, and `objects.githubusercontent.com`. `api.github.com` is not used and not allowed. Velopack's built-in sources follow redirects to any host, which is why they are not used.
- **Verification before an update is ready:** SHA-256 of the package against the feed, size, and the version embedded in the package (Velopack never checks the embedded version). A failed newer download never deletes an older ready package. Apply and restart happen only after confirmation in About.
- **Unsigned builds:** no code signing or notarization on any OS (macOS bundles are ad-hoc signed only). Integrity rests on HTTPS to GitHub and the hash in the release's own feed, so anyone able to publish a release on the repository can ship an update. `SHA256SUMS` on each release detects corruption only.
- **Publish hardening:** protected `release` environment, `v*` tag ruleset, and PR-only `main`; see [Release and CI](operations/release-ci.md#repository-settings).
- **Linux AppImage extraction:** the runtime is replaced so it needs no FUSE. Its default extraction directory was predictable and ran a planted one, so the target is a private per-user `~/.cache/made/appimage-<version>-<hash>` (hash from the image SHA-256). Residual: if extraction is cut short inside `AppRun` itself, the truncated guard is reused and that build does not start until the directory is removed; untested because it needs a kill at that exact point.
- **Windows apply:** the swap is two renames and is not atomic; power loss during apply is unverified on Windows and on Linux layouts that cross filesystems.

## Accepted residuals

| Residual | Why accepted / handling |
|---|---|
| Same-user token inspection through environment/process memory | Loopback-only, per-launch rotating token, redacted logs, local threat model. |
| Cross-user bridge DoS | Eight pre-auth connection slots can delay a legitimate plugin attempt. No state disclosure and terminal stays usable; reserving a slot or OS-authenticated transport would close it. |
| Preview TOCTOU/FIFO replacement | Same-user exclusion; display-only preview; existing FIFO/device rejected. |
| Windows private-log reparse-point TOCTOU | Owner-only app directory; check/open is non-atomic; packaging-time hardening. This applies to the active log, `.1`, and `.lock` paths. Lock, replacement-rename, and mode behavior are cross-compiled but not runtime-verified on Windows. |
| Old-version instance ignores the log lock | Current builds append one record under a bounded persistent lock; an older build without the lock can still interleave and can push the active log above 1 MiB, so mixed-version shutdowns are not protected. |
| Final-exit log failure visibility | The shutdown write has no UI frame left; refusals/I/O failures go to stderr and cannot be shown in Diagnostics. Automated refusal tests cover the contract, but no in-app final-exit notice is claimed. |
| Lock timeout during shutdown | A stuck current-version peer causes the append to be refused after at most one second instead of hanging shutdown; the record is not written. |
| Windows Job Object cleanup | Implemented/cross-compiled, native runtime evidence pending; immediate-child-only is not claimed as full descendant cleanup. |
| Native macOS/Windows backdrop | Compile-checked only; no runtime effect/contrast claim. |
| Hardened git read gaps | Scope-read timing, git before 2.45/2.26, repo-local LFS, C-locale detection, non-enumerated config; see [Hardened git reads](#hardened-git-reads). |
| Same-user hostile session title | Sanitized when it reaches the plugin; a real control+newline title blocked before hello in one probe, exact blocker not isolated. |

The bridge is not defended against a same-user attacker who can inspect process memory or ptrace the process. It does defend against accidental connections, other local users on a multi-user machine, and non-loopback access.

## Evidence

Targeted tests cover token redaction, auth rejection, cross-workspace/pane rejection, config immutability, path guard, preview no-write/binary/size guards, bracketed-paste fixed point, private permissions, and foreign events. See the [v1 evidence record](records/20261026-v1-evidence.md) and the bridge tests under [`crates/made-bridge/tests`](../crates/made-bridge/tests).

Related: [OpenCode bridge](integration/opencode.md), [Architecture](architecture.md), [v1 evidence](records/20261026-v1-evidence.md).
