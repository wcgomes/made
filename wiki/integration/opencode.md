# OpenCode Bridge

Made integrates OpenCode v2 through its official CLI/TUI plugin API and a per-workspace loopback WebSocket bridge. Made never launches or manages OpenCode: the user runs `opencode` in a Made terminal, and the plugin loads from configuration injected into that shell. The bridge carries normalized state and focus commands; it is not a remote API. Normative integration requirements live in [`specs/changes/opencode-session-follow-v1/spec.md`](../../specs/changes/opencode-session-follow-v1/spec.md).

## Topology

```text
Made workspace tab
  terminal PTY
    user runs `opencode`
      OpenCode v2 TUI + app-owned plugin
        ws://127.0.0.1:<ephemeral>/made
          made-bridge (one listener per tab)
            normalized SessionSnapshot -> made-app root arbitration
```

Each workspace tab owns its listener, pane id, and 256-bit token. There is no shared multiplexed bridge. A second authenticated connection for the same pane replaces the first with close code `4409`; a cross-pane hello is rejected before replacement or state disclosure.

OpenCode v2 is not a single process. Default `opencode` uses a managed background service, while `opencode --standalone` still starts a private `serve --stdio --port 0`. Made does not own either service lifecycle and does not attach to the user's service. The Phase 0 full session-follow harness used `--standalone`; managed mode was load-tested only.

## Plugin package

The plugin lives at [`plugins/opencode-made`](../../plugins/opencode-made/README.md) and must remain a directory package:

```text
package.json      exports "." (server no-op) and "./tui" (TUI entry)
index.ts          no-op server entry
tui.ts            TUI plugin entry; imports @opencode/plugin/tui and solid-js
src/integration.ts testable core; no OpenCode or Solid imports
src/protocol.ts   wire types, option validation, loopback URL check
```

OpenCode v2.1.4 did not load a bare `.ts` path in the Phase 0 probe; it loaded the directory package. The app-owned plugin path is resolved through [`made-platform::AppPaths`](../../crates/made-platform/src/lib.rs), checked to be inside the app-owned root, and injected as a package entry. Release builds require the package under `<install_root>/plugins/opencode-made`; development builds prefer a complete staged package, otherwise the repository package or `MADE_PLUGIN_DIR`.

## Configuration injection

Made composes `OPENCODE_CLI_CONFIG_CONTENT` in memory for the launch; it never edits the user's `cli.json`:

1. Read the user's `cli.json` as a read-only base, if present.
2. Merge an existing `OPENCODE_CLI_CONFIG_CONTENT` overlay, if present. At the top level, plugin arrays are combined; nested objects merge and other values replace.
3. Force `tabs.mode: "on"` and `tabs.scope: "global"` for this launch so the plugin sees all open root sessions and worktrees.
4. Remove any pre-existing Made plugin entry, then append one package entry with namespaced options:

```json
{
  "plugins": [{
    "package": "/absolute/path/to/plugins/opencode-made",
    "options": {
      "made": {
        "bridgeUrl": "ws://127.0.0.1:54321/made",
        "token": "<per-launch secret>",
        "paneId": "pane-1",
        "protocolVersion": 1,
        "appVersion": "0.1.0"
      }
    }
  }]
}
```

Composition runs off the UI thread and fails open: if the base or overlay is invalid, the shell keeps its original environment, OpenCode runs without Made's plugin, and Diagnostics reports configuration failure rather than blocking the terminal. [`ConfigGuard`](../../crates/made-bridge/src/config_guard.rs) hashes the user config before the session and verifies it afterward; the hash is evidence, not a repair mechanism.

## Protocol

Transport is WebSocket at `ws://127.0.0.1:<ephemeral>/made`. The listener is always `127.0.0.1:0`; the path must be `/made`; the `Host` header must match the loopback authority and an `Origin` header is rejected. No state is emitted before authentication.

| Direction | Messages |
|---|---|
| plugin → app | `hello`, `state`, `focus_result`, `pong`, `error`, `goodbye` |
| app → plugin | `welcome`, `focus`, `request_state`, `ping` |

Handshake and limits implemented in [`server.rs`](../../crates/made-bridge/src/server.rs) and [`security.rs`](../../crates/made-bridge/src/security.rs):

- `hello` must arrive within one second of the accepted socket, covering the WebSocket handshake. Validation checks token, pane id, protocol major, and OpenCode major.
- Close codes: `4401` invalid token/pane/malformed hello, `4402` protocol or OpenCode version mismatch, `4403` handshake deadline, `4409` replaced by a newer authenticated connection, `1009` oversized message.
- Messages are capped at 256 KiB; snapshots at 64 sessions, 512-character titles, 4096-character paths, and 128-character IDs. A malformed frame closes with a protocol error; unknown fields are ignored.
- `revision` increases per sent snapshot and resets to `1` on a new authenticated epoch. The app discards stale revisions and coalesces one pending snapshot per epoch.
- Heartbeat is every 5 seconds with a 3-missed-pong limit. Focus requests time out after 5 seconds and fail non-blockingly.
- The bridge never sends `goodbye`; the plugin tolerates one for forward compatibility.

## Plugin behavior

The React/Solid integration observes `context.ui.tabs.enabled()` and `context.ui.tabs.list()` inside an injected reactive effect. Each tab's directory comes synchronously from `context.data.session.get(sessionID)`. Rapid changes are coalesced with a 60 ms debounce; full snapshots are sent on accepted change, `request_state`, and reconnect.

- `unread` is derived as `promptPulse > 0` because OpenCode v2.1.4 exposes no unread field; `attention` is the real attention signal.
- Before focusing, the plugin checks the current tab list. A closed or missing session returns `focus_result {ok:false, error:"session not open"}` and is not reopened.
- Unexpected closes reconnect with bounded exponential backoff (100 ms base, 5 s cap, jitter). Auth/protocol rejection, replacement, and fatal plugin-error closes are terminal for that instance; `1008`/`1009` retries are limited to three consecutive attempts.
- Backoff and fatal-rejection budgets reset only after a connection has held both `welcome` and a sent state for five seconds.
- The plugin never logs the token; error text is truncated and token-redacted. It does not run shell commands or read arbitrary files.

## Phase 0 observed API divergences

The Phase 0 gate ran against OpenCode v2.1.4. These are the durable differences from the original design:

| Assumption | Observed |
|---|---|
| Import `@opencode/plugin` | TUI plugins import `@opencode/plugin/tui`; `@opencode/plugin` is the server plugin API. |
| `tabs.list()` includes `unread` | No `unread` field; returns `sessionID`, `title`, `active`, `promptPulse`, `attention`, `busy`, `renaming`. |
| `session.get()` is async | Synchronous and reactive in 2.1.4. |
| `tabs.focus()` focuses only open tabs | It can open a closed tab; the plugin now checks the list and refuses closed sessions. |
| `context.location` is the launch cwd | Reactive to the active session location. Use `process.cwd()` for `hello.cwd` and `session.get().location.directory` per tab. |
| A `.ts` file can be injected | A directory package with `tui.ts` loads; a bare `.ts` entry did not. |
| `tabs.enabled` is enough | Current settings are `tabs.mode` (`on`/`auto`/`off`) and `tabs.scope` (`cwd`/`global`); legacy `tabs.enabled` is still accepted. |
| Focus changes state atomically | A transient `activeSessionID: null` (~20 ms observed) can occur during transitions; the app must tolerate it. |

The plugin is pinned to this observed surface. Bridge protocol version gates major drift, but minor OpenCode API changes remain a risk; failures should surface as `unsupported` or `degraded` rather than crashing the TUI.

## Managed versus standalone

- **Standalone (`opencode --standalone`):** the TUI starts a private `serve --stdio --port 0`. The Phase 0 gate used this to avoid the user's service on port 49374.
- **Default managed mode:** OpenCode uses a managed background service. A separate isolated load probe showed the plugin loads, but full session follow was not repeated in managed mode.
- **Made's position:** the terminal runs the user's command; Made does not choose the topology, launch a service, or attach to one. A managed service may outlive Made and may conflict with another user service; that is OpenCode-owned behavior.

## Evidence and limits

- Plugin tests: 29/29 plus typecheck (`plugins/opencode-made`).
- Phase 0 gate: 19/19 gate plus 5/5 supporting checks against OpenCode v2.1.4; see the [Phase 0 gate record](../records/20261025-phase-0-gate.md). The harness was removed after the gate.
- App-level live session switching on the revised terminal-first build is still pending; the current evidence for bridge/config/root precedence is test-level plus historical real-OpenCode runs.
- WSL-hosted OpenCode, remote/SSH OpenCode, ACP, and other agents are out of scope.

Related: [Security](../security.md), [Runtime terminal-first](../runtime/terminal-first.md), [Phase 0 record](../records/20261025-phase-0-gate.md).
