/**
 * Production Made bridge integration.
 *
 * This module deliberately has no OpenCode or Solid imports. `tui.ts` injects
 * the real context and `createEffect`; tests inject fakes. Every bridge and
 * OpenCode interaction is guarded so a missing bridge or a rejected API call
 * cannot throw into the TUI.
 *
 * State is bounded to the same limits the Rust bridge enforces in
 * `crates/made-bridge/src/security.rs` and `state.rs`. If the plugin sent an
 * out-of-bounds snapshot the bridge would close with `1008`/`1009`; mirroring
 * the limits keeps that from becoming a reconnect loop.
 */

import {
  MAX_CONSECUTIVE_FATAL_CLOSURES,
  MAX_ERROR_CODE_LENGTH,
  MAX_ERROR_LENGTH,
  MAX_GOODBYE_LENGTH,
  MAX_MESSAGE_BYTES,
  MAX_OPENCODE_VERSION_LENGTH,
  MAX_PATH_LENGTH,
  MAX_PROJECT_ID_LENGTH,
  MAX_SESSIONS,
  MAX_SESSION_ID_LENGTH,
  MAX_TITLE_LENGTH,
  PLUGIN_VERSION,
  PROTOCOL_VERSION,
  RECONNECT_BASE_MS,
  RECONNECT_MAX_MS,
  STABLE_CONNECTION_MS,
  STATE_DEBOUNCE_MS,
  UINT32_MAX,
  clamp,
  exceedsCodePoints,
  isRecord,
  readMadeOptions,
  utf8ByteLength,
  type ErrorMessage,
  type FocusResultMessage,
  type GoodbyeMessage,
  type HelloMessage,
  type MadeOptions,
  type PluginMessage,
  type SessionSnapshot,
  type StateMessage,
} from "./protocol.ts";

export interface TuiTab {
  sessionID: string;
  title?: string;
  active?: boolean;
  busy?: boolean;
  attention?: boolean;
  promptPulse?: number;
  renaming?: boolean;
}

export interface TuiSessionInfo {
  title?: string;
  projectID?: string;
  subpath?: string;
  location?: { directory?: string };
}

export interface TuiPluginContext {
  options?: Record<string, unknown>;
  app?: { version?: string };
  location?: { directory?: string };
  ui: {
    tabs: {
      enabled(): boolean;
      list(): TuiTab[];
      focus(sessionID: string): boolean;
    };
    toast?: {
      show(input: { message: string; variant?: string }): void;
    };
  };
  data: {
    session: {
      get(sessionID: string): TuiSessionInfo | undefined | PromiseLike<TuiSessionInfo | undefined>;
    };
  };
}

export type EffectRunner = (fn: () => void) => void;

/**
 * Timing knobs for tests. Production uses the protocol defaults; the app does
 * not inject these through plugin options.
 */
export interface IntegrationTuning {
  /** Time a welcome+state connection must stay up before counters reset. */
  stableConnectionMs?: number;
  /** Consecutive 1008/1009 closes tolerated before reconnecting stops. */
  maxConsecutiveFatalClosures?: number;
}

interface WebSocketLike {
  readyState: number;
  send(data: string): void;
  close(code?: number, reason?: string): void;
  onopen: (() => void) | null;
  onmessage: ((event: { data: unknown }) => void) | null;
  onerror: (() => void) | null;
  onclose: ((event: { code?: number; reason?: string }) => void) | null;
}

type WebSocketFactory = new (url: string) => WebSocketLike;

interface SessionInfoView {
  title: string | null;
  directory: string | null;
  subpath: string | null;
  projectID: string | null;
}

const WEBSOCKET_OPEN = 1;

/** Close codes that end reconnection for this plugin instance. */
const TERMINAL_CLOSE_CODES: ReadonlySet<number> = new Set([1011, 4401, 4402, 4403, 4409]);

/** Close codes that indicate a fatal state/message rejection. */
const FATAL_REJECTION_CLOSE_CODES: ReadonlySet<number> = new Set([1008, 1009]);

const MIN_FIT_TITLE_LENGTH = 32;
const MIN_FIT_PATH_LENGTH = 128;
const MIN_FINAL_TITLE_LENGTH = 8;
const MIN_FINAL_PATH_LENGTH = 32;
const SNAPSHOT_SIZE_HEADROOM = 128;

const EMPTY_INFO: SessionInfoView = {
  title: null,
  directory: null,
  subpath: null,
  projectID: null,
};

function noop(): void {
  // Used when the plugin is disabled; the TUI keeps running.
}

function getWebSocketFactory(): WebSocketFactory | undefined {
  const candidate = (globalThis as { WebSocket?: unknown }).WebSocket;
  return typeof candidate === "function" ? (candidate as WebSocketFactory) : undefined;
}

function readOpenCodeVersion(context: TuiPluginContext): string {
  try {
    const version = context.app?.version;
    return typeof version === "string" && version.length > 0
      ? clamp(version, MAX_OPENCODE_VERSION_LENGTH)
      : "unknown";
  } catch {
    return "unknown";
  }
}

function readProcessInfo(): { pid: number; cwd: string } {
  try {
    const processRef = (globalThis as {
      process?: { pid?: unknown; cwd?: () => unknown };
    }).process;
    const rawPid = processRef?.pid;
    const pid =
      typeof rawPid === "number" && Number.isInteger(rawPid) && rawPid >= 0 ? rawPid : 0;
    const rawCwd = typeof processRef?.cwd === "function" ? processRef.cwd() : "";
    return {
      pid,
      cwd: typeof rawCwd === "string" ? clamp(rawCwd, MAX_PATH_LENGTH) : "",
    };
  } catch {
    return { pid: 0, cwd: "" };
  }
}

function errorText(error: unknown): string {
  try {
    if (error instanceof Error) return error.message;
    if (typeof error === "string") return error;
    return JSON.stringify(error) ?? String(error);
  } catch {
    return "unknown error";
  }
}

function isThenable(value: unknown): value is PromiseLike<unknown> {
  if ((typeof value !== "object" || value === null) && typeof value !== "function") return false;
  try {
    return typeof (value as { then?: unknown }).then === "function";
  } catch {
    return false;
  }
}

/**
 * The bridge field is `u32`. Fractional and non-finite values are rejected
 * (normalized to zero); positive integers above the u32 range are clamped.
 */
function normalizePromptPulse(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value) || !Number.isInteger(value)) {
    return 0;
  }
  if (value <= 0) return 0;
  return value > UINT32_MAX ? UINT32_MAX : value;
}

function hasActiveTab(tabs: TuiTab[]): boolean {
  for (const tab of tabs) {
    if (tab.active === true) return true;
  }
  return false;
}

function normalizeTab(value: unknown): TuiTab | undefined {
  if (!isRecord(value)) return undefined;
  try {
    if (typeof value.sessionID !== "string" || value.sessionID.length === 0) return undefined;
    if (exceedsCodePoints(value.sessionID, MAX_SESSION_ID_LENGTH)) return undefined;
    const tab: TuiTab = { sessionID: value.sessionID };
    if (typeof value.title === "string") tab.title = clamp(value.title, MAX_TITLE_LENGTH);
    if (typeof value.active === "boolean") tab.active = value.active;
    if (typeof value.busy === "boolean") tab.busy = value.busy;
    if (typeof value.attention === "boolean") tab.attention = value.attention;
    if (typeof value.promptPulse === "number") {
      tab.promptPulse = normalizePromptPulse(value.promptPulse);
    }
    if (typeof value.renaming === "boolean") tab.renaming = value.renaming;
    return tab;
  } catch {
    return undefined;
  }
}

function normalizeSessionInfo(value: unknown): SessionInfoView {
  if (!isRecord(value)) return EMPTY_INFO;
  try {
    const location = isRecord(value.location) ? value.location : undefined;
    return {
      title: typeof value.title === "string" ? clamp(value.title, MAX_TITLE_LENGTH) : null,
      directory:
        location && typeof location.directory === "string"
          ? clamp(location.directory, MAX_PATH_LENGTH)
          : null,
      subpath: typeof value.subpath === "string" ? clamp(value.subpath, MAX_PATH_LENGTH) : null,
      projectID:
        typeof value.projectID === "string"
          ? clamp(value.projectID, MAX_PROJECT_ID_LENGTH)
          : null,
    };
  } catch {
    return EMPTY_INFO;
  }
}

type ViewDirectoryObservation =
  | { kind: "usable"; directory: string }
  | { kind: "absent"; retain: boolean }
  | { kind: "invalid"; reason: string; value: string };

function isAbsoluteLocalPath(value: string): boolean {
  if (value.startsWith("/")) return true;
  // Windows drive and UNC paths, for OpenCode running on Windows.
  if (/^[A-Za-z]:[\\/]/.test(value)) return true;
  return value.startsWith("\\\\");
}

/**
 * Validate the Home/new-session view directory before it reaches the wire.
 *
 * `undefined` is transient during OpenCode synchronization and retains the
 * last known value. Every other unusable value is absent: an invalid value
 * clears the retained directory and is reported non-fatally. Oversized values
 * are omitted rather than truncated so a path prefix is never presented as the
 * selected folder.
 */
function validateViewDirectory(value: unknown): ViewDirectoryObservation {
  if (value === undefined) return { kind: "absent", retain: true };
  if (value === null) return { kind: "absent", retain: false };
  if (typeof value !== "string") {
    return { kind: "invalid", reason: "is not a string", value: "" };
  }
  if (value.length === 0) return { kind: "invalid", reason: "is empty", value };
  if (value.includes("\0")) {
    return { kind: "invalid", reason: "contains a NUL character", value };
  }
  if (exceedsCodePoints(value, MAX_PATH_LENGTH)) {
    return {
      kind: "invalid",
      reason: `exceeds ${MAX_PATH_LENGTH} code points`,
      value,
    };
  }
  if (!isAbsoluteLocalPath(value)) {
    return { kind: "invalid", reason: "is not absolute", value };
  }
  const segments = value.split(/[\\/]+/);
  if (segments.some((segment) => segment === "." || segment === "..")) {
    return { kind: "invalid", reason: "contains a dot segment", value };
  }
  return { kind: "usable", directory: value };
}

function trimSession(
  session: SessionSnapshot,
  titleLength: number,
  pathLength: number,
): SessionSnapshot {
  return {
    ...session,
    title: clamp(session.title, titleLength),
    directory: session.directory === null ? null : clamp(session.directory, pathLength),
    subpath: session.subpath === null ? null : clamp(session.subpath, pathLength),
  };
}

function stateForSize(
  sessions: SessionSnapshot[],
  activeSessionID: string | null,
  tabsEnabled: boolean,
  viewDirectory: string | null,
): StateMessage {
  return {
    type: "state",
    // Use the largest exactly representable revision for sizing so the real
    // revision can never push a fitted snapshot over the bridge limit.
    revision: Number.MAX_SAFE_INTEGER,
    activeSessionID,
    tabsEnabled,
    viewDirectory,
    sessions,
  };
}

function snapshotByteLength(message: StateMessage): number {
  try {
    return utf8ByteLength(JSON.stringify(message));
  } catch {
    return MAX_MESSAGE_BYTES + 1;
  }
}

function positiveInteger(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) && value >= 1
    ? Math.floor(value)
    : fallback;
}

export function createIntegration(
  context: TuiPluginContext,
  effect: EffectRunner,
  tuning: IntegrationTuning = {},
): () => void {
  let parsed;
  try {
    parsed = readMadeOptions(context.options);
  } catch {
    parsed = { ok: false as const, error: "made options could not be read" };
  }

  const toast = (message: string, variant = "error"): void => {
    try {
      context.ui?.toast?.show({ message, variant });
    } catch {
      // A toast is best-effort and must never block the plugin.
    }
  };

  if (!parsed.ok) {
    toast(`Made bridge disabled: ${parsed.error}`);
    return noop;
  }

  const options: MadeOptions = parsed.options;
  const WebSocketCtor = getWebSocketFactory();
  if (!WebSocketCtor) {
    toast("Made bridge disabled: WebSocket unavailable");
    return noop;
  }

  const stableConnectionMs = positiveInteger(tuning.stableConnectionMs, STABLE_CONNECTION_MS);
  const maxFatalClosures = positiveInteger(
    tuning.maxConsecutiveFatalClosures,
    MAX_CONSECUTIVE_FATAL_CLOSURES,
  );

  const opencodeVersion = readOpenCodeVersion(context);
  const redact = (message: string): string =>
    clamp(message.split(options.token).join("[redacted]"), MAX_ERROR_LENGTH);

  let socket: WebSocketLike | undefined;
  let welcomed = false;
  let unloaded = false;
  /** Set by a terminal close or an unrecoverable local integration failure. */
  let stopped = false;
  let revision = 0;
  let lastSnapshotKey = "";
  let reconnectAttempt = 0;
  let consecutiveFatalClosures = 0;
  let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  let stableTimer: ReturnType<typeof setTimeout> | undefined;
  let debounceTimer: ReturnType<typeof setTimeout> | undefined;
  let pendingForce = false;
  let generation = 0;
  let tabsEnabled = false;
  let tabList: TuiTab[] = [];
  let viewDirectory: string | null = null;
  let lastViewErrorKey = "";
  let stateSentThisEpoch = false;
  let deferredError: { code: string; message: string } | undefined;

  const infoBySession = new Map<string, SessionInfoView>();

  const send = (message: PluginMessage): boolean => {
    if (!socket || socket.readyState !== WEBSOCKET_OPEN) return false;
    let text: string;
    try {
      text = JSON.stringify(message);
    } catch {
      return false;
    }
    if (utf8ByteLength(text) > MAX_MESSAGE_BYTES) return false;
    try {
      socket.send(text);
      return true;
    } catch {
      return false;
    }
  };

  const sendError = (code: string, message: string, fatal = false): void => {
    const payload: ErrorMessage = {
      type: "error",
      code: clamp(code, MAX_ERROR_CODE_LENGTH),
      message: redact(message),
      fatal,
    };
    send(payload);
  };

  const observeViewDirectory = (): void => {
    let raw: unknown;
    try {
      raw = context.location?.directory;
    } catch (error) {
      viewDirectory = null;
      lastViewErrorKey = "";
      sendError("view_directory_unavailable", errorText(error));
      return;
    }

    const observation = validateViewDirectory(raw);
    if (observation.kind === "usable") {
      viewDirectory = observation.directory;
      lastViewErrorKey = "";
      return;
    }
    if (observation.kind === "absent") {
      if (!observation.retain) viewDirectory = null;
      lastViewErrorKey = "";
      return;
    }

    // An invalid value is absent and clears the retained directory. Report it
    // once per distinct bad value so a reactive effect cannot spam the bridge.
    viewDirectory = null;
    const key = `${observation.reason}\u0000${observation.value}`;
    if (key === lastViewErrorKey) return;
    lastViewErrorKey = key;
    const detail = observation.value.length > 0 ? `: ${observation.value}` : "";
    sendError("view_directory_invalid", `view directory ${observation.reason}${detail}`);
  };

  const clearStableTimer = (): void => {
    if (stableTimer !== undefined) {
      clearTimeout(stableTimer);
      stableTimer = undefined;
    }
  };

  const armStableTimer = (): void => {
    if (
      unloaded ||
      stopped ||
      stableTimer !== undefined ||
      !welcomed ||
      !stateSentThisEpoch
    ) {
      return;
    }
    stableTimer = setTimeout(() => {
      stableTimer = undefined;
      if (unloaded || stopped || !welcomed || !stateSentThisEpoch) return;
      // Only a connection that held welcome plus state for the stable period
      // proves the bridge path healthy. A welcome alone must not reset backoff.
      reconnectAttempt = 0;
      consecutiveFatalClosures = 0;
    }, stableConnectionMs);
  };

  const buildSnapshot = (): {
    sessions: SessionSnapshot[];
    activeSessionID: string | null;
  } => {
    const sessions: SessionSnapshot[] = [];
    const seen = new Set<string>();

    for (const tab of tabList) {
      if (seen.has(tab.sessionID)) continue;
      if (sessions.length >= MAX_SESSIONS) break;
      seen.add(tab.sessionID);

      const info = infoBySession.get(tab.sessionID);
      const promptPulse = normalizePromptPulse(tab.promptPulse);
      const tabTitle = typeof tab.title === "string" && tab.title.length > 0 ? tab.title : null;
      sessions.push({
        sessionID: tab.sessionID,
        title: clamp(tabTitle ?? info?.title ?? "(untitled)", MAX_TITLE_LENGTH),
        active: false,
        busy: tab.busy === true,
        attention: tab.attention === true,
        // OpenCode v2.1.4 has no unread field; promptPulse > 0 is the documented approximation.
        unread: promptPulse > 0,
        promptPulse,
        renaming: tab.renaming === true,
        directory: info?.directory ?? null,
        subpath: info?.subpath ?? null,
        projectID: info?.projectID ?? null,
      });
    }

    // At most one active session. Choose the first active tab whose session is
    // actually present in the capped, de-duplicated snapshot; otherwise null.
    let activeSessionID: string | null = null;
    for (const tab of tabList) {
      if (tab.active === true && seen.has(tab.sessionID)) {
        activeSessionID = tab.sessionID;
        break;
      }
    }
    for (const session of sessions) {
      session.active = session.sessionID === activeSessionID;
    }

    return { sessions, activeSessionID };
  };

  /**
   * Fit a snapshot inside the bridge's message limit. Long titles and paths
   * are shortened first; if that is not enough, trailing non-active sessions
   * are dropped. The active session is preserved whenever one exists.
   */
  const fitSnapshot = (
    sessions: SessionSnapshot[],
    activeSessionID: string | null,
    viewDirectory: string | null,
  ): StateMessage => {
    let titleLength = MAX_TITLE_LENGTH;
    let pathLength = MAX_PATH_LENGTH;
    let currentView = viewDirectory;
    let current = sessions.map((session) => trimSession(session, titleLength, pathLength));
    let message = stateForSize(current, activeSessionID, tabsEnabled, currentView);

    while (
      snapshotByteLength(message) + SNAPSHOT_SIZE_HEADROOM >
      MAX_MESSAGE_BYTES
    ) {
      if (titleLength > MIN_FIT_TITLE_LENGTH || pathLength > MIN_FIT_PATH_LENGTH) {
        const nextTitle = Math.max(MIN_FIT_TITLE_LENGTH, Math.floor(titleLength / 2));
        const nextPath = Math.max(MIN_FIT_PATH_LENGTH, Math.floor(pathLength / 2));
        titleLength = nextTitle < titleLength ? nextTitle : titleLength;
        pathLength = nextPath < pathLength ? nextPath : pathLength;
        current = sessions.map((session) =>
          trimSession(session, titleLength, pathLength),
        );
        message = stateForSize(current, activeSessionID, tabsEnabled, currentView);
        continue;
      }

      // Drop the view directory before any session. It is optional, while the
      // session map is the primary payload.
      if (currentView !== null) {
        currentView = null;
        message = stateForSize(current, activeSessionID, tabsEnabled, currentView);
        continue;
      }

      if (current.length > 1) {
        let dropIndex = -1;
        for (let index = current.length - 1; index >= 0; index -= 1) {
          if (!current[index].active) {
            dropIndex = index;
            break;
          }
        }
        if (dropIndex >= 0) {
          current.splice(dropIndex, 1);
          message = stateForSize(current, activeSessionID, tabsEnabled, currentView);
          continue;
        }
      }

      // A single session at the minimum field lengths is far below the limit;
      // this final trim is defensive if a future field grows.
      current = current.map((session) =>
        trimSession(session, MIN_FINAL_TITLE_LENGTH, MIN_FINAL_PATH_LENGTH),
      );
      message = stateForSize(current, activeSessionID, tabsEnabled, currentView);
      break;
    }

    return message;
  };

  const sendState = (force: boolean): void => {
    if (!welcomed || unloaded || stopped) return;
    const built = buildSnapshot();
    const view = tabsEnabled && !hasActiveTab(tabList) ? viewDirectory : null;
    const snapshot = fitSnapshot(built.sessions, built.activeSessionID, view);
    let key: string;
    try {
      key = JSON.stringify({ ...snapshot, revision: 0 });
    } catch {
      return;
    }
    if (!force && key === lastSnapshotKey) return;

    const nextRevision = revision + 1;
    if (send({ ...snapshot, revision: nextRevision })) {
      revision = nextRevision;
      lastSnapshotKey = key;
      stateSentThisEpoch = true;
      armStableTimer();
    }
  };

  const scheduleState = (force: boolean): void => {
    if (unloaded || stopped) return;
    pendingForce = pendingForce || force;
    if (debounceTimer !== undefined) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      debounceTimer = undefined;
      const shouldForce = pendingForce;
      pendingForce = false;
      try {
        sendState(shouldForce);
      } catch (error) {
        sendError("state_failed", errorText(error));
      }
    }, STATE_DEBOUNCE_MS);
  };

  const scheduleReconnect = (): void => {
    if (unloaded || stopped || reconnectTimer !== undefined) return;
    const exponent = Math.min(reconnectAttempt, 6);
    const backoff = Math.min(RECONNECT_BASE_MS * 2 ** exponent, RECONNECT_MAX_MS);
    const jitter = Math.floor(Math.random() * 100);
    reconnectAttempt += 1;
    reconnectTimer = setTimeout(() => {
      reconnectTimer = undefined;
      connect();
    }, backoff + jitter);
  };

  const readTabIDs = (): Set<string> | undefined => {
    let rawTabs: unknown;
    try {
      rawTabs = context.ui.tabs.list();
    } catch (error) {
      sendError("tabs_unavailable", errorText(error));
      return undefined;
    }
    if (!Array.isArray(rawTabs)) {
      sendError("tabs_unavailable", "tabs.list() did not return an array");
      return undefined;
    }
    const ids = new Set<string>();
    for (const entry of rawTabs) {
      const tab = normalizeTab(entry);
      if (tab) ids.add(tab.sessionID);
    }
    return ids;
  };

  const handleFocus = (message: Record<string, unknown>): void => {
    const requestID = message.requestID;
    const sessionID = message.sessionID;
    if (typeof requestID !== "string" || typeof sessionID !== "string") return;

    // OpenCode v2 `tabs.focus` opens a closed/missing tab. Never call it for a
    // session that is not in the current tab list; report and resync instead.
    const openSessionIDs = readTabIDs();
    if (!openSessionIDs || !openSessionIDs.has(sessionID)) {
      const missing: FocusResultMessage = {
        type: "focus_result",
        requestID,
        ok: false,
        error: "session not open",
      };
      send(missing);
      refresh(true);
      return;
    }

    let ok = false;
    let failure: string | undefined;
    try {
      ok = context.ui.tabs.focus(sessionID) === true;
      if (!ok) failure = "tabs.focus returned false";
    } catch (error) {
      failure = redact(errorText(error));
    }

    const result: FocusResultMessage = { type: "focus_result", requestID, ok };
    if (!ok) result.error = failure ?? "focus failed";
    send(result);
    if (ok) {
      scheduleState(true);
    } else {
      refresh(true);
    }
  };

  const handleMessage = (event: { data: unknown }): void => {
    if (unloaded || stopped) return;

    let parsedMessage: unknown;
    try {
      const text = typeof event.data === "string" ? event.data : String(event.data);
      parsedMessage = JSON.parse(text);
    } catch {
      sendError("malformed_bridge_message", "bridge message was not JSON");
      return;
    }
    if (!isRecord(parsedMessage)) return;

    switch (parsedMessage.type) {
      case "welcome": {
        if (parsedMessage.protocolVersion !== PROTOCOL_VERSION) {
          // The bridge has rejected or is about to reject this connection.
          // Reconnecting cannot change a protocol major mismatch.
          stopped = true;
          clearStableTimer();
          sendError("protocol_mismatch", "bridge protocol version is unsupported", true);
          try {
            socket?.close(4402, "protocol mismatch");
          } catch {
            // Closing is best-effort; the socket handler owns reconnection.
          }
          return;
        }
        // A duplicate welcome on an established connection must not reset the
        // revision epoch or the reconnect backoff.
        if (welcomed) return;
        welcomed = true;
        revision = 0;
        lastSnapshotKey = "";
        stateSentThisEpoch = false;
        // A new authenticated epoch never reuses the previous epoch's view
        // directory. Re-read it so the first snapshot reflects the current
        // Home/new-session selection without waiting for another effect run.
        viewDirectory = null;
        lastViewErrorKey = "";
        clearStableTimer();
        if (deferredError) {
          sendError(deferredError.code, deferredError.message, true);
          deferredError = undefined;
          // A broken reactive effect cannot be fixed by reconnecting.
          stopped = true;
          clearStableTimer();
          return;
        }
        if (tabsEnabled && !hasActiveTab(tabList)) {
          observeViewDirectory();
        }
        sendState(true);
        return;
      }
      case "focus":
        handleFocus(parsedMessage);
        return;
      case "request_state": {
        if (typeof parsedMessage.requestID !== "string") return;
        sendState(true);
        return;
      }
      case "ping": {
        if (typeof parsedMessage.nonce !== "string") return;
        send({ type: "pong", nonce: parsedMessage.nonce });
        return;
      }
      case "goodbye":
        return;
      default:
        // Unknown message types and unknown fields are ignored for forward compatibility.
        return;
    }
  };

  const connect = (): void => {
    if (unloaded || stopped) return;

    let next: WebSocketLike;
    try {
      next = new WebSocketCtor(options.bridgeUrl);
    } catch {
      scheduleReconnect();
      return;
    }
    socket = next;
    welcomed = false;
    stateSentThisEpoch = false;
    clearStableTimer();

    next.onopen = () => {
      if (unloaded || stopped || socket !== next) {
        try {
          next.close();
        } catch {
          // Ignore a stale connection close failure.
        }
        return;
      }
      const processInfo = readProcessInfo();
      const hello: HelloMessage = {
        type: "hello",
        protocolVersion: PROTOCOL_VERSION,
        token: options.token,
        paneId: options.paneId,
        pluginVersion: PLUGIN_VERSION,
        opencodeVersion,
        pid: processInfo.pid,
        cwd: processInfo.cwd,
      };
      let text: string;
      try {
        text = JSON.stringify(hello);
      } catch {
        try {
          next.close(4000, "hello encoding failed");
        } catch {
          // The close handler owns reconnection.
        }
        return;
      }
      try {
        if (utf8ByteLength(text) > MAX_MESSAGE_BYTES) {
          throw new Error("hello exceeds the bridge message limit");
        }
        next.send(text);
      } catch (error) {
        sendError("hello_failed", errorText(error));
        // A transient send failure should retry with backoff instead of
        // waiting for the bridge's auth timeout and a terminal 4401.
        try {
          next.close(4000, "hello send failed");
        } catch {
          // The close handler owns reconnection.
        }
      }
    };
    next.onmessage = (event) => {
      if (socket !== next) return;
      try {
        handleMessage(event);
      } catch (error) {
        sendError("bridge_message_failed", errorText(error));
      }
    };
    next.onerror = () => {
      // The close handler owns reconnection. Do not surface raw errors: they
      // can contain the authenticated URL or token.
    };
    next.onclose = (event) => {
      const wasCurrent = socket === next;
      if (wasCurrent) {
        socket = undefined;
        welcomed = false;
        stateSentThisEpoch = false;
        clearStableTimer();
      }
      if (unloaded || !wasCurrent) return;
      const code = typeof event?.code === "number" ? event.code : 0;

      // Authentication/protocol rejection, a replaced connection, or a clean
      // bridge close after a fatal plugin error cannot be fixed by reconnecting.
      if (TERMINAL_CLOSE_CODES.has(code)) {
        stopped = true;
        return;
      }

      // A fatal state/message rejection is bounded: the plugin gets a small
      // number of corrected attempts, then stops instead of storming the bridge.
      if (FATAL_REJECTION_CLOSE_CODES.has(code)) {
        consecutiveFatalClosures += 1;
        if (consecutiveFatalClosures >= maxFatalClosures) {
          stopped = true;
          return;
        }
      }

      scheduleReconnect();
    };
  };

  const readTabs = (): { enabled: boolean; tabs: TuiTab[] } => {
    let enabled = false;
    try {
      enabled = context.ui.tabs.enabled() === true;
    } catch (error) {
      sendError("tabs_enabled_failed", errorText(error));
    }

    let rawTabs: unknown;
    let listFailed = false;
    try {
      rawTabs = context.ui.tabs.list();
    } catch (error) {
      sendError("tabs_unavailable", errorText(error));
      rawTabs = [];
      listFailed = true;
    }

    if (listFailed || !Array.isArray(rawTabs)) {
      if (!listFailed) {
        sendError("tabs_unavailable", "tabs.list() did not return an array");
      }
      // A snapshot built from a failed list is incomplete. Report the
      // integration as tabs-disabled so the app shows the v1 tabs constraint.
      enabled = false;
    }

    const tabs: TuiTab[] = [];
    if (Array.isArray(rawTabs)) {
      for (const entry of rawTabs) {
        const tab = normalizeTab(entry);
        if (tab) tabs.push(tab);
      }
    }
    return { enabled, tabs };
  };

  const resolveSessionInfo = (sessionID: string, runGeneration: number): void => {
    let result: unknown;
    try {
      // Synchronous in OpenCode v2.1.4; PromiseLike is supported defensively.
      result = context.data.session.get(sessionID);
    } catch (error) {
      sendError("session_get_failed", errorText(error));
      return;
    }

    if (isThenable(result)) {
      Promise.resolve(result).then(
        (info) => {
          // Generation guard: a slow lookup for an old active tab must not
          // overwrite the newer selection.
          if (unloaded || runGeneration !== generation) return;
          infoBySession.set(sessionID, normalizeSessionInfo(info));
          scheduleState(false);
        },
        (error) => {
          if (unloaded || runGeneration !== generation) return;
          sendError("session_get_failed", errorText(error));
          scheduleState(false);
        },
      );
      return;
    }

    infoBySession.set(sessionID, normalizeSessionInfo(result));
  };

  const refresh = (force: boolean): void => {
    if (unloaded) return;
    const runGeneration = ++generation;
    const { enabled, tabs } = readTabs();

    // Read the selected Home/new-session folder in the same reactive pass as
    // the tab list. Only the sessionless tab-enabled state may retain a value
    // across a transient undefined; an active session or unavailable tabs
    // clear the cache so a later undefined cannot resurrect it.
    if (enabled && !hasActiveTab(tabs)) {
      observeViewDirectory();
    } else {
      viewDirectory = null;
      lastViewErrorKey = "";
    }

    const seen = new Set<string>();
    for (const tab of tabs) {
      seen.add(tab.sessionID);
      resolveSessionInfo(tab.sessionID, runGeneration);
    }
    for (const sessionID of infoBySession.keys()) {
      if (!seen.has(sessionID)) infoBySession.delete(sessionID);
    }

    tabList = tabs;
    tabsEnabled = enabled;
    scheduleState(force);
  };

  try {
    effect(() => {
      try {
        refresh(false);
      } catch (error) {
        sendError("refresh_failed", errorText(error));
      }
    });
  } catch (error) {
    deferredError = { code: "effect_failed", message: redact(errorText(error)) };
    toast("Made bridge: tab observation unavailable");
  }

  connect();

  return () => {
    if (unloaded) return;
    unloaded = true;
    generation += 1;
    viewDirectory = null;
    lastViewErrorKey = "";
    if (debounceTimer !== undefined) {
      clearTimeout(debounceTimer);
      debounceTimer = undefined;
    }
    if (reconnectTimer !== undefined) {
      clearTimeout(reconnectTimer);
      reconnectTimer = undefined;
    }
    clearStableTimer();

    const active = socket;
    socket = undefined;
    welcomed = false;
    stateSentThisEpoch = false;
    if (!active) return;

    if (active.readyState === WEBSOCKET_OPEN) {
      const goodbye: GoodbyeMessage = {
        type: "goodbye",
        reason: clamp("plugin unloaded", MAX_GOODBYE_LENGTH),
      };
      try {
        const text = JSON.stringify(goodbye);
        if (utf8ByteLength(text) <= MAX_MESSAGE_BYTES) active.send(text);
      } catch {
        // The bridge may already be gone.
      }
    }
    try {
      active.close(1000, "plugin unloaded");
    } catch {
      // Best-effort close.
    }
  };
}
