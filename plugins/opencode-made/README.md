# Made OpenCode plugin

App-owned OpenCode v2 CLI/TUI plugin. It observes the open session tabs, resolves each tab's
working directory, and sends normalized snapshots to Made's loopback WebSocket bridge. It also
handles focus requests from Made.

This package is not installed into the user's OpenCode configuration. Made composes a per-launch
`OPENCODE_CLI_CONFIG_CONTENT` value and points OpenCode at this directory for that launch only.

## Package shape

OpenCode v2.1.4 did **not** load a bare `.ts` path in the Phase 0 probe; it loaded a directory
package. Keep this directory intact.

```text
plugins/opencode-made/
  package.json          exports "." (server no-op) and "./tui" (TUI entry)
  index.ts              no-op server entry, kept for OpenCode's package layout
  tui.ts                TUI plugin entry; imports @opencode/plugin/tui and solid-js
  src/integration.ts    testable core; no OpenCode or Solid imports
  src/protocol.ts       wire types, option validation, loopback URL check
  tests/lifecycle.test.ts
  types/runtime.d.ts    ambient declarations for runtime-supplied modules
```

`src/integration.ts` exports `createIntegration(context, effect)`. The real entrypoint injects
OpenCode's `createEffect`; tests inject a fake effect and a fake context.

## Injected options

The app appends one plugin entry to the launch config. Options are namespaced under `made`:

```json
{
  "tabs": { "mode": "on", "scope": "global" },
  "plugins": [
    {
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
    }
  ]
}
```

| Option | Required | Validation |
|---|---|---|
| `bridgeUrl` | yes | `ws://` with an explicit port and a loopback IP literal (`127.x.x.x` or `::1`); `localhost`, `wss://`, wildcard, and LAN addresses are rejected |
| `token` | yes | string of at least 32 characters; never logged |
| `paneId` | yes | non-empty string, at most 128 characters |
| `protocolVersion` | yes | integer; a version that differs from the plugin's `PROTOCOL_VERSION` is accepted so the bridge can reject it with `4402` and the app can show `unsupported` |
| `appVersion` | yes | non-empty string, at most 128 characters |

`tabs.mode: "on"` and `tabs.scope: "global"` are required so the plugin can see all root-session
tabs. Made forces those settings for the launch; this package does not write any OpenCode file.

## Behavior

- Connects to `bridgeUrl`, sends `hello`, and waits for `welcome` before sending state.
- `hello` always carries this plugin's `PROTOCOL_VERSION` (`1`). The plugin does not refuse a
  differing `options.made.protocolVersion` locally; the bridge owns that decision and closes with
  `4402`, which lets the app show an unsupported-integration status.
- Observes `context.ui.tabs.enabled()` and `context.ui.tabs.list()` inside the injected reactive
  effect. Each tab is resolved synchronously with `context.data.session.get(sessionID)` for
  `location.directory`, `subpath`, and `projectID`.
- Coalesces rapid changes with a 60 ms debounce and sends full state snapshots. `revision`
  increases for each sent snapshot and resets to `1` on a new authenticated connection epoch.
- Resolves asynchronous session lookups defensively with a generation guard, so a slow lookup
  for an old active tab cannot overwrite a newer selection.
- Bounds every snapshot to the same limits the Rust bridge enforces: at most 64 sessions, unique
  non-empty session IDs at 128 code points, at most one active session (the first active tab
  wins; otherwise `activeSessionID: null`), `promptPulse` as an integer `u32`, titles at 512 code
  points, paths at 4096 code points, project IDs at 128 code points, and a total outbound JSON
  frame of at most 256 KiB (UTF-8). If a snapshot is still too large, long titles/paths are
  shortened and then trailing non-active sessions are dropped; the active session is preserved.
- If `tabs.list()` throws while `tabs.enabled()` was true, the snapshot reports
  `tabsEnabled: false` instead of claiming a complete tab list.
- Handles `request_state`, `ping`/`pong`, and `focus`. Before focusing, the plugin verifies that
  the session is present in the current `tabs.list()`. A closed or missing session returns
  `focus_result {ok: false, error: "session not open"}` and is **not** reopened; exceptions and
  `false` also become `ok: false`.
- On unload sends `goodbye { reason: "plugin unloaded" }` and closes.
- Reconnects after unexpected closes with bounded exponential backoff (100 ms base, 5 s cap) plus
  jitter. Authentication/protocol rejection (`4401`, `4402`, `4403`), replacement (`4409`), and a
  fatal plugin-error close (`1011`) are terminal for that plugin instance. Fatal state/message
  rejections (`1008`, `1009`) are retried at most three consecutive times before stopping.
- Backoff and the fatal-rejection budget reset only after a connection has held both `welcome` and
  a sent state for five seconds. A `welcome` alone does not reset them, so a bridge that accepts
  the handshake and then rejects state cannot be tight-looped.
- Reports non-fatal integration failures as `error` messages. Error text is truncated and any
  occurrence of the token is replaced with `[redacted]`.
- Never throws into the TUI, never logs the token, and never reads files or runs commands.

### Protocol messages

Plugin to Made: `hello`, `state`, `focus_result`, `pong`, `error`, `goodbye`.
Made to plugin: `welcome`, `focus`, `request_state`, `ping`. The bridge does not send a
`goodbye`; the plugin tolerates an incoming `goodbye` for forward compatibility.

`hello` contains `type`, `protocolVersion`, `token`, `paneId`, `pluginVersion`,
`opencodeVersion`, `pid`, and `cwd`. `cwd` is the OpenCode process launch directory
(`process.cwd()`), not `context.location`: in v2.1.4 `context.location` follows the active
session's directory and is not a stable launch cwd.

Each state session contains exactly:

```text
sessionID, title, active, busy, attention, unread, promptPulse, renaming,
directory, subpath, projectID
```

Unknown incoming message types and unknown fields are ignored for forward compatibility.

## `unread` approximation

OpenCode v2.1.4's tab object exposes `sessionID`, `title`, `active`, `promptPulse`, `attention`,
`busy`, and `renaming`. It has **no `unread` field**. This plugin derives:

```text
unread = promptPulse > 0
```

`attention` is passed through separately as the real OpenCode attention signal. This is an
approximation for the session-map unread indicator, not a new OpenCode capability.

## Security

- The plugin connects only to an explicit loopback IP literal with an explicit port. It rejects
  non-loopback hosts, `localhost`, `wss://`, wildcard addresses, and URLs without a port.
- The bridge token is only sent in the `hello` frame. It is not included in error text, toasts,
  logs, state, or goodbye messages; errors are redacted before sending.
- The plugin uses only the OpenCode plugin context, WebSocket, timers, and `process.pid` /
  `process.cwd()`. It does not execute shell commands, read arbitrary files, or modify user
  configuration.
- Same-user processes can still inspect the token through the child environment/plugin options.
  This is the documented v1 residual risk; the bridge is loopback-only and the token rotates per
  launch.

## Test and typecheck

Requires Node 24.

```sh
cd plugins/opencode-made
npm install
npm test          # node --test tests/lifecycle.test.ts
npm run typecheck # tsc --noEmit
```

The lifecycle tests use an in-process `ws` server and fake OpenCode context. They cover
handshake, state snapshots, coalescing and revision ordering, active-directory changes, stale
lookup rejection, focus success/failure and closed-tab refusal, ping/pong, `request_state`,
reconnect epoch reset, unload goodbye, invalid options, token redaction, the bridge snapshot bounds
(64 sessions, duplicate IDs, multiple active tabs, `u32` prompt pulses, emoji-safe truncation, and
the 256 KiB frame limit), `tabsEnabled: false` on a failing `tabs.list()`, terminal close codes,
the bounded `1008`/`1009` budget, and the stable-period backoff reset. They do not read the user's
OpenCode configuration or data.

## OpenCode v2.1.4 assumptions and risks

- The plugin imports `@opencode/plugin/tui` and `solid-js`; OpenCode resolves both at runtime.
  No build step is used.
- `context.ui.tabs.list()` and `context.data.session.get()` are synchronous in v2.1.4. The
  integration also tolerates a promise-like `get()` result for tests and future drift.
- `context.ui.tabs.focus()` returns a boolean and may open a closed tab. The integration
  therefore checks the current `tabs.list()` first and refuses to focus a session that is not
  open; the API's open-on-focus behavior is no longer relied on. A boolean `false` or a thrown
  error is reported as `focus_result.ok: false`.
- `context.location` is reactive and follows the active session. All per-session directories come
  from `context.data.session.get().location.directory`.
- A transient "no active tab" state can occur during focus transitions; the 60 ms debounce
  coalesces it. The app must still tolerate a snapshot with `activeSessionID: null`.
- OpenCode's plugin API can drift across minor releases. The bridge validates the protocol major
  version; an incompatible runtime fails closed instead of throwing into the TUI. A differing
  `options.made.protocolVersion` is sent through as a normal `hello` so the bridge can report
  `4402`; the plugin does not pre-empt that decision.
