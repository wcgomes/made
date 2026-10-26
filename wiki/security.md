# Security

Made is a local-first desktop application, not a multi-user service. Its main trust boundaries are the loopback OpenCode bridge, the user's read-only OpenCode config, the workspace filesystem, and untrusted text arriving from OpenCode or the terminal. This page records the implemented model and accepted v1 residuals. Normative requirements are in [`specs/changes/opencode-session-follow-v1/spec.md`](../specs/changes/opencode-session-follow-v1/spec.md).

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

Only `made-explorer` reads workspace file contents for display. Read-only git metadata goes through a bounded allowlist in [`made-app::worktree`](../crates/made-app/src/worktree.rs). The bridge config code is the only reader of the user's OpenCode config.

## Read-only preview

- Preview has no edit/save/write API, no dirty state, no LSP, and no diff path. Save shortcuts are inert while preview has focus; terminal Ctrl+S remains XOFF.
- Files over 2 MiB are rejected with a size message. Binary detection checks for a NUL byte in the first 8 KiB; invalid UTF-8 gets a safe message rather than replacement noise. A missing/unreadable/deleted file gets a contextual error.
- Preview text is rendered as plain text, never HTML or executable content.
- Reads happen off the UI thread and are generation-gated. A root change clears the preview and invalidates stale reads.
- A same-user process can replace a path between canonical check and open (TOCTOU), or supply a FIFO/device. An existing FIFO/device is rejected before open and the opened handle is rechecked, but the race is not atomic. V1 accepts this under the same-user exclusion.

## Untrusted text and input

- Session titles, paths, error strings, and log text are treated as untrusted. Central sanitization replaces controls/newlines with spaces and removes bidi/format controls before display/copy.
- Embedded legacy/C1 bracketed-paste markers are replaced with spaces to a fixed point, so overlapping input cannot reconstruct a marker. This is input hardening, not a general escape filter; pasted control characters still reach the child unless the child handles them.
- Terminal key semantics are preserved: Ctrl/Cmd+C copies only with a selection, otherwise Ctrl+C remains SIGINT on Linux/Windows; Ctrl+X remains CAN; Ctrl+S remains XOFF; macOS Cmd+C without selection is ignored. egui clipboard shortcuts stay disabled.
- The shell is never asked to evaluate cwd report contents, and Made does not scrape prompts/titles or inject discovery commands into a running shell.

## Private state and lifecycle

- App temp is per-user (Linux `$XDG_RUNTIME_DIR/made-<uid>` or data-dir fallback; macOS/Windows temp under `made-<user>`), created/validated `0700` on Unix, rejecting pre-existing symlinks/non-directories. Logs are append-only `0600` and reject symlink/non-regular files.
- Shutdown captures the selected workspace, cwd capability/reason, versions, and metrics before releasing the terminal, so persisted diagnostics retain context.
- Bridge events from a foreign workspace or connection are ignored and cannot release or reset a healthy authenticated workspace.
- There is no telemetry or off-device state transfer in v1.

## Accepted residuals

| Residual | Why accepted / handling |
|---|---|
| Same-user token inspection through environment/process memory | Loopback-only, per-launch rotating token, redacted logs, local threat model. |
| Cross-user bridge DoS | Eight pre-auth connection slots can delay a legitimate plugin attempt. No state disclosure and terminal stays usable; reserving a slot or OS-authenticated transport would close it. |
| Preview TOCTOU/FIFO replacement | Same-user exclusion; display-only preview; existing FIFO/device rejected. |
| Windows private-log reparse-point TOCTOU | Owner-only app directory; check/open is non-atomic; packaging-time hardening. |
| Windows Job Object cleanup | Implemented/cross-compiled, native runtime evidence pending; immediate-child-only is not claimed as full descendant cleanup. |
| Native macOS/Windows backdrop | Compile-checked only; no runtime effect/contrast claim. |
| Same-user hostile session title | Sanitized when it reaches the plugin; a real control+newline title blocked before hello in one probe, exact blocker not isolated. |

The bridge is not defended against a same-user attacker who can inspect process memory or ptrace the process. It does defend against accidental connections, other local users on a multi-user machine, and non-loopback access.

## Evidence

Targeted tests cover token redaction, auth rejection, cross-workspace/pane rejection, config immutability, path guard, preview no-write/binary/size guards, bracketed-paste fixed point, private permissions, and foreign events. See the [v1 evidence record](records/20261026-v1-evidence.md) and the bridge tests under [`crates/made-bridge/tests`](../crates/made-bridge/tests).

Related: [OpenCode bridge](integration/opencode.md), [Architecture](architecture.md), [v1 evidence](records/20261026-v1-evidence.md).
