/**
 * Wire protocol for the Made loopback bridge (design D4, protocol major v1).
 *
 * Message field names and types are part of the bridge contract. Additive
 * fields are allowed within a major version; consumers ignore unknown fields.
 *
 * The size limits below mirror `crates/made-bridge/src/security.rs` and the
 * snapshot validation in `crates/made-bridge/src/state.rs`. Bounding state in
 * the plugin as well as in the bridge keeps an oversized or malformed snapshot
 * from closing the connection with `1008`/`1009` and feeding a reconnect storm.
 */

export const PROTOCOL_VERSION = 1;

/** Kept in sync with package.json. The plugin has no build step and does not read files. */
export const PLUGIN_VERSION = "0.1.0";

export const STATE_DEBOUNCE_MS = 60;
export const RECONNECT_BASE_MS = 100;
export const RECONNECT_MAX_MS = 5000;

/**
 * A connection must hold `welcome` plus at least one sent state for this long
 * before reconnect backoff and fatal-rejection counters are reset.
 */
export const STABLE_CONNECTION_MS = 5000;

/** Consecutive `1008`/`1009` close codes tolerated before giving up. */
export const MAX_CONSECUTIVE_FATAL_CLOSURES = 3;

/** Maximum size of a single outbound WebSocket message, in UTF-8 bytes. */
export const MAX_MESSAGE_BYTES = 256 * 1024;

/** Maximum number of sessions in one state snapshot. */
export const MAX_SESSIONS = 64;

/** Maximum number of Unicode code points in a session title. */
export const MAX_TITLE_LENGTH = 512;

/** Maximum number of Unicode code points in a directory, subpath, or hello cwd. */
export const MAX_PATH_LENGTH = 4096;

/** Maximum number of Unicode code points in a session identifier. */
export const MAX_SESSION_ID_LENGTH = 128;

/** Maximum number of Unicode code points in a project identifier. */
export const MAX_PROJECT_ID_LENGTH = 128;

/** Maximum number of Unicode code points in a pane identifier. */
export const MAX_PANE_ID_LENGTH = 128;

/** Maximum number of Unicode code points in an app version. */
export const MAX_APP_VERSION_LENGTH = 128;

/** Maximum number of Unicode code points in an OpenCode version string. */
export const MAX_OPENCODE_VERSION_LENGTH = 64;

/** Maximum number of Unicode code points in a plugin error message. */
export const MAX_ERROR_LENGTH = 512;

/** Maximum number of Unicode code points in a plugin error code. */
export const MAX_ERROR_CODE_LENGTH = 64;

/** Maximum number of Unicode code points in a goodbye reason. */
export const MAX_GOODBYE_LENGTH = 256;

/** Largest value carried by the bridge's `promptPulse: u32` field. */
export const UINT32_MAX = 0xffff_ffff;

export interface HelloMessage {
  type: "hello";
  protocolVersion: number;
  token: string;
  paneId: string;
  pluginVersion: string;
  opencodeVersion: string;
  pid: number;
  cwd: string;
}

export interface SessionSnapshot {
  sessionID: string;
  title: string;
  active: boolean;
  busy: boolean;
  attention: boolean;
  /**
   * OpenCode v2.1.4 does not expose an `unread` tab field. Made derives it as
   * `promptPulse > 0`; `attention` remains a separate, real OpenCode signal.
   */
  unread: boolean;
  promptPulse: number;
  renaming: boolean;
  directory: string | null;
  subpath: string | null;
  projectID: string | null;
}

export interface StateMessage {
  type: "state";
  revision: number;
  activeSessionID: string | null;
  tabsEnabled: boolean;
  /**
   * Absolute directory selected in OpenCode's Home/new-session view.
   *
   * Additive within protocol major 1: absent or `null` means no connected view
   * directory. The plugin omits unusable values rather than truncating them.
   */
  viewDirectory?: string | null;
  sessions: SessionSnapshot[];
}

export interface FocusResultMessage {
  type: "focus_result";
  requestID: string;
  ok: boolean;
  error?: string;
}

export interface PongMessage {
  type: "pong";
  nonce: string;
}

export interface ErrorMessage {
  type: "error";
  code: string;
  message: string;
  fatal: boolean;
}

export interface GoodbyeMessage {
  type: "goodbye";
  reason: string;
}

export interface WelcomeMessage {
  type: "welcome";
  protocolVersion: number;
  appVersion: string;
  heartbeatIntervalMs: number;
}

export interface FocusMessage {
  type: "focus";
  requestID: string;
  sessionID: string;
}

export interface RequestStateMessage {
  type: "request_state";
  requestID: string;
}

export interface PingMessage {
  type: "ping";
  nonce: string;
}

export type PluginMessage =
  | HelloMessage
  | StateMessage
  | FocusResultMessage
  | PongMessage
  | ErrorMessage
  | GoodbyeMessage;

export type BridgeMessage = WelcomeMessage | FocusMessage | RequestStateMessage | PingMessage;

export type MadeOptions = {
  bridgeUrl: string;
  token: string;
  paneId: string;
  /**
   * The major version supplied by Made. It is deliberately not compared here:
   * `hello` always carries this plugin's `PROTOCOL_VERSION`, so the bridge is
   * the single authority that rejects a mismatch with `4402` and lets the app
   * show an unsupported-integration status.
   */
  protocolVersion: number;
  appVersion: string;
};

export type ParsedOptions =
  | { ok: true; options: MadeOptions }
  | { ok: false; error: string };

/**
 * Truncate to at most `max` Unicode code points.
 *
 * Iterating a string yields code points, so a supplementary character (emoji,
 * CJK extension, etc.) is never split into a lone surrogate pair.
 */
export function clamp(value: string, max: number): string {
  if (value.length <= max) return value;
  let result = "";
  let count = 0;
  for (const character of value) {
    if (count >= max) break;
    result += character;
    count += 1;
  }
  return result;
}

/** Return true when `value` contains more than `max` Unicode code points. */
export function exceedsCodePoints(value: string, max: number): boolean {
  if (value.length <= max) return false;
  let count = 0;
  for (const _character of value) {
    count += 1;
    if (count > max) return true;
  }
  return false;
}

/**
 * Exact UTF-8 byte length without depending on a runtime-specific Buffer.
 * Lone surrogates are counted as the three-byte replacement character, which
 * matches how `ws` encodes an outgoing string.
 */
export function utf8ByteLength(value: string): number {
  let bytes = 0;
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code < 0x80) {
      bytes += 1;
    } else if (code < 0x800) {
      bytes += 2;
    } else if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        bytes += 4;
        index += 1;
      } else {
        bytes += 3;
      }
    } else {
      bytes += 3;
    }
  }
  return bytes;
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Only an explicit loopback IP literal with an explicit port is accepted.
 * `localhost` is deliberately rejected: it can resolve to a non-loopback
 * address depending on the host's resolver and hosts file.
 */
export function isLoopbackWebSocketURL(raw: string): boolean {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    return false;
  }
  if (url.protocol !== "ws:") return false;
  if (!url.port) return false;
  if (url.username.length > 0 || url.password.length > 0) return false;

  const host = url.hostname.replace(/^\[/, "").replace(/\]$/, "");
  if (host === "::1") return true;

  const match = /^127\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(host);
  if (!match) return false;
  return match.slice(1).every((part) => Number(part) <= 255);
}

/**
 * Options are namespaced by the app as `{ made: { ... } }` so user plugin
 * options cannot collide with the integration.
 *
 * A protocol version that differs from this plugin's version is accepted: the
 * hello still carries `PROTOCOL_VERSION`, and the bridge owns the `4402`
 * unsupported-version decision. Missing credentials and invalid bridge URLs
 * still disable the integration locally.
 */
export function readMadeOptions(raw: unknown): ParsedOptions {
  if (!isRecord(raw) || !isRecord(raw.made)) {
    return { ok: false, error: "made options are missing" };
  }
  const made = raw.made;

  const bridgeUrl = made.bridgeUrl;
  if (typeof bridgeUrl !== "string" || !isLoopbackWebSocketURL(bridgeUrl)) {
    return { ok: false, error: "bridgeUrl must be a loopback ws:// URL with an explicit port" };
  }

  const token = made.token;
  if (typeof token !== "string" || token.length < 32) {
    return { ok: false, error: "token is missing or too short" };
  }

  const paneId = made.paneId;
  if (
    typeof paneId !== "string" ||
    paneId.length === 0 ||
    exceedsCodePoints(paneId, MAX_PANE_ID_LENGTH)
  ) {
    return { ok: false, error: "paneId is missing or invalid" };
  }

  const protocolVersion = made.protocolVersion;
  if (
    typeof protocolVersion !== "number" ||
    !Number.isInteger(protocolVersion) ||
    protocolVersion < 0
  ) {
    return { ok: false, error: "protocolVersion is missing or invalid" };
  }

  const appVersion = made.appVersion;
  if (
    typeof appVersion !== "string" ||
    appVersion.length === 0 ||
    exceedsCodePoints(appVersion, MAX_APP_VERSION_LENGTH)
  ) {
    return { ok: false, error: "appVersion is missing or invalid" };
  }

  return {
    ok: true,
    options: { bridgeUrl, token, paneId, protocolVersion, appVersion },
  };
}
