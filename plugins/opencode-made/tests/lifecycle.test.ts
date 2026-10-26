import test from "node:test";
import assert from "node:assert/strict";
import type { AddressInfo } from "node:net";
import { WebSocket, WebSocketServer } from "ws";
import {
  createIntegration,
  type IntegrationTuning,
  type TuiPluginContext,
  type TuiSessionInfo,
  type TuiTab,
} from "../src/integration.ts";
import {
  MAX_MESSAGE_BYTES,
  MAX_PROJECT_ID_LENGTH,
  MAX_PATH_LENGTH,
  MAX_TITLE_LENGTH,
  PROTOCOL_VERSION,
  isLoopbackWebSocketURL,
  readMadeOptions,
} from "../src/protocol.ts";

const TOKEN = "test-token-0123456789abcdef0123456789abcdef";
const WAIT_MS = 3000;

interface WireMessage {
  [key: string]: unknown;
  type?: string;
}

interface HarnessState {
  tabs: TuiTab[];
  sessions: Map<string, unknown>;
  enabled: boolean;
  focusCalls: string[];
  focusResult: (sessionID: string) => boolean;
  toasts: string[];
  sessionError: Error | undefined;
  listError: Error | undefined;
}

interface Harness {
  state: HarnessState;
  trigger(): void;
  dispose(): void;
  effectCalls(): number;
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitFor<T>(
  probe: () => T | undefined | null | false,
  label: string,
  timeoutMs = WAIT_MS,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = probe();
    if (value) return value;
    if (Date.now() >= deadline) throw new Error(`timed out waiting for ${label}`);
    await delay(10);
  }
}

class TestConnection {
  readonly messages: WireMessage[] = [];
  readonly rawMessages: string[] = [];
  readonly socket: WebSocket;
  closed = false;

  constructor(socket: WebSocket) {
    this.socket = socket;
    socket.on("message", (data) => {
      const text = data.toString();
      this.rawMessages.push(text);
      try {
        this.messages.push(JSON.parse(text) as WireMessage);
      } catch {
        // Only the plugin's own protocol messages are expected.
      }
    });
    socket.on("close", () => {
      this.closed = true;
    });
  }

  send(message: Record<string, unknown>): void {
    if (this.socket.readyState === WebSocket.OPEN) {
      this.socket.send(JSON.stringify(message));
    }
  }

  terminate(): void {
    this.socket.terminate();
  }

  waitForMessage(
    type: string,
    predicate: (message: WireMessage) => boolean = () => true,
    timeoutMs = WAIT_MS,
  ): Promise<WireMessage> {
    return waitFor(
      () => this.messages.find((message) => message.type === type && predicate(message)),
      `message ${type}`,
      timeoutMs,
    );
  }

  stateMessages(): WireMessage[] {
    return this.messages.filter((message) => message.type === "state");
  }
}

class TestBridge {
  private wss: WebSocketServer | undefined;
  private port = 0;
  readonly connections: TestConnection[] = [];
  welcomeEnabled = true;
  /** Close with this code as soon as hello arrives (auth/protocol rejection). */
  helloCloseCode: number | undefined;
  /** Close with this code shortly after sending welcome (replacement). */
  welcomeCloseCode: number | undefined;

  async start(): Promise<string> {
    const wss = new WebSocketServer({ host: "127.0.0.1", port: 0 });
    this.wss = wss;
    wss.on("connection", (socket) => {
      const connection = new TestConnection(socket);
      this.connections.push(connection);

      socket.on("message", (data) => {
        let message: WireMessage;
        try {
          message = JSON.parse(data.toString()) as WireMessage;
        } catch {
          return;
        }
        if (message.type !== "hello") return;
        if (this.helloCloseCode !== undefined) {
          socket.close(this.helloCloseCode, "test close");
          return;
        }
        if (!this.welcomeEnabled) return;
        connection.send({
          type: "welcome",
          protocolVersion: PROTOCOL_VERSION,
          appVersion: "0.1.0-test",
          heartbeatIntervalMs: 1000,
          // Unknown fields must be ignored by the plugin.
          ignoredField: true,
        });
        if (this.welcomeCloseCode !== undefined) {
          const code = this.welcomeCloseCode;
          setTimeout(() => {
            try {
              socket.close(code, "test close");
            } catch {
              // Already closed.
            }
          }, 10);
        }
      });
    });

    await new Promise<void>((resolve, reject) => {
      wss.once("listening", () => resolve());
      wss.once("error", reject);
    });
    this.port = (wss.address() as AddressInfo).port;
    return this.url;
  }

  get url(): string {
    return `ws://127.0.0.1:${this.port}/made`;
  }

  connection(index = 0): Promise<TestConnection> {
    return waitFor(
      () => this.connections[index],
      `connection ${index + 1}`,
    );
  }

  async close(): Promise<void> {
    const wss = this.wss;
    this.wss = undefined;
    for (const connection of this.connections) {
      try {
        connection.terminate();
      } catch {
        // Already closed.
      }
    }
    if (!wss) return;
    await new Promise<void>((resolve) => {
      wss.close(() => resolve());
    });
  }
}

function createState(): HarnessState {
  return {
    tabs: [],
    sessions: new Map<string, unknown>(),
    enabled: true,
    focusCalls: [],
    focusResult: () => true,
    toasts: [],
    sessionError: undefined,
    listError: undefined,
  };
}

function createContext(made: Record<string, unknown>, state: HarnessState): TuiPluginContext {
  return {
    options: { made },
    app: { version: "2.1.4" },
    location: { directory: "/launch/workspace" },
    ui: {
      tabs: {
        enabled: () => state.enabled,
        list: () => {
          if (state.listError) throw state.listError;
          return state.tabs.map((tab) => ({ ...tab }));
        },
        focus: (sessionID: string) => {
          state.focusCalls.push(sessionID);
          return state.focusResult(sessionID);
        },
      },
      toast: {
        show: (input: { message: string }) => {
          state.toasts.push(String(input.message));
        },
      },
    },
    data: {
      session: {
        get: (sessionID: string) => {
          if (state.sessionError) throw state.sessionError;
          return state.sessions.get(sessionID) as
            | TuiSessionInfo
            | undefined
            | PromiseLike<TuiSessionInfo | undefined>;
        },
      },
    },
  };
}

interface HarnessOptions {
  token?: string;
  protocolVersion?: number;
  tuning?: IntegrationTuning;
}

function createHarness(bridgeUrl: string, overrides: HarnessOptions = {}): Harness {
  const state = createState();
  const context = createContext(
    {
      bridgeUrl,
      token: overrides.token ?? TOKEN,
      paneId: "pane-test",
      protocolVersion: overrides.protocolVersion ?? PROTOCOL_VERSION,
      appVersion: "0.1.0-test",
    },
    state,
  );

  let effectFn: (() => void) | undefined;
  let calls = 0;
  const dispose = createIntegration(
    context,
    (fn) => {
      calls += 1;
      effectFn = fn;
      fn();
    },
    overrides.tuning,
  );

  return {
    state,
    trigger: () => {
      if (!effectFn) throw new Error("the reactive effect was not registered");
      effectFn();
    },
    dispose,
    effectCalls: () => calls,
  };
}

function byteLength(value: string): number {
  return Buffer.byteLength(value, "utf8");
}

function hasLoneSurrogate(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (next < 0xdc00 || next > 0xdfff) return true;
      index += 1;
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      return true;
    }
  }
  return false;
}

test("connects, sends hello, waits for welcome, and sends initial state", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    const connection = await bridge.connection();
    const hello = await connection.waitForMessage("hello");

    assert.equal(hello.type, "hello");
    assert.equal(hello.protocolVersion, PROTOCOL_VERSION);
    assert.equal(hello.token, TOKEN);
    assert.equal(hello.paneId, "pane-test");
    assert.equal(hello.pluginVersion, "0.1.0");
    assert.equal(hello.opencodeVersion, "2.1.4");
    assert.equal(hello.pid, process.pid);
    assert.equal(hello.cwd, process.cwd());

    const state = await connection.waitForMessage("state");
    assert.equal(state.type, "state");
    assert.equal(state.revision, 1);
    assert.equal(state.activeSessionID, null);
    assert.equal(state.tabsEnabled, true);
    assert.deepEqual(state.sessions, []);

    const types = connection.messages.map((message) => message.type);
    assert.deepEqual(types.slice(0, 2), ["hello", "state"]);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("sends a full snapshot after tabs change and derives unread from promptPulse", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    const connection = await bridge.connection();
    await connection.waitForMessage("state");

    harness.state.tabs = [
      {
        sessionID: "ses-a",
        title: "Alpha",
        active: true,
        busy: true,
        attention: true,
        promptPulse: 2,
        renaming: true,
      },
    ];
    harness.state.sessions.set("ses-a", {
      title: "Alpha session",
      projectID: "prj-1",
      subpath: "src",
      location: { directory: "/work/a" },
    });
    harness.trigger();

    const state = await connection.waitForMessage(
      "state",
      (message) => Array.isArray(message.sessions) && message.sessions.length === 1,
    );

    assert.equal(state.revision, 2);
    assert.equal(state.activeSessionID, "ses-a");
    assert.equal(state.tabsEnabled, true);
    assert.deepEqual(state.sessions, [
      {
        sessionID: "ses-a",
        title: "Alpha",
        active: true,
        busy: true,
        attention: true,
        unread: true,
        promptPulse: 2,
        renaming: true,
        directory: "/work/a",
        subpath: "src",
        projectID: "prj-1",
      },
    ]);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("coalesces rapid tab changes and increments revision monotonically", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    const connection = await bridge.connection();
    await connection.waitForMessage("state");

    for (let index = 0; index < 5; index += 1) {
      harness.state.tabs = [
        { sessionID: "ses-a", title: `Title ${index}`, active: true },
      ];
      harness.trigger();
      await delay(5);
    }

    const state = await connection.waitForMessage(
      "state",
      (message) =>
        Array.isArray(message.sessions) &&
        (message.sessions[0] as { title?: string } | undefined)?.title === "Title 4",
    );
    assert.equal(state.revision, 2);

    await delay(150);
    const states = connection.stateMessages();
    assert.equal(states.length, 2);
    assert.deepEqual(
      states.map((message) => message.revision),
      [1, 2],
    );
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("follows an active-session directory change", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    harness.state.tabs = [
      { sessionID: "ses-a", title: "Alpha", active: true },
      { sessionID: "ses-b", title: "Beta", active: false },
    ];
    harness.state.sessions.set("ses-a", {
      location: { directory: "/work/a" },
    });
    harness.state.sessions.set("ses-b", {
      location: { directory: "/work/b" },
    });

    // The initial effect ran before these fixtures were set, so drive one more
    // reactive pass as OpenCode would after its tab data settles.
    harness.trigger();

    const connection = await bridge.connection();
    const first = await connection.waitForMessage(
      "state",
      (message) => message.activeSessionID === "ses-a",
    );
    assert.equal(
      ((first.sessions as Array<{ directory?: string }>)[0] ?? {}).directory,
      "/work/a",
    );

    harness.state.tabs = [
      { sessionID: "ses-a", title: "Alpha", active: false },
      { sessionID: "ses-b", title: "Beta", active: true },
    ];
    harness.trigger();

    const second = await connection.waitForMessage(
      "state",
      (message) => message.activeSessionID === "ses-b",
    );
    const sessions = second.sessions as Array<{ sessionID: string; directory: string | null }>;
    assert.equal(sessions.find((session) => session.sessionID === "ses-b")?.directory, "/work/b");
    assert.ok((second.revision as number) > (first.revision as number));
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("a slow old active-tab lookup cannot overwrite a newer selection", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();

  let resolveOld: ((value: TuiSessionInfo) => void) | undefined;
  const oldLookup = new Promise<TuiSessionInfo>((resolve) => {
    resolveOld = resolve;
  });

  const harness = createHarness(url);
  try {
    harness.state.tabs = [{ sessionID: "ses-a", title: "Old", active: true }];
    harness.state.sessions.set("ses-a", oldLookup);
    harness.trigger();

    const connection = await bridge.connection();
    await connection.waitForMessage(
      "state",
      (message) => Array.isArray(message.sessions) && message.sessions.length === 1,
    );

    harness.state.tabs = [
      { sessionID: "ses-b", title: "New", active: true },
    ];
    harness.state.sessions.set("ses-b", {
      location: { directory: "/work/new" },
    });
    harness.trigger();

    const newer = await connection.waitForMessage(
      "state",
      (message) => message.activeSessionID === "ses-b",
    );
    const newerSessions = newer.sessions as Array<{ sessionID: string; directory?: string }>;
    assert.equal(
      newerSessions.find((session) => session.sessionID === "ses-b")?.directory,
      "/work/new",
    );

    const statesBeforeStaleResolution = connection.stateMessages().length;
    resolveOld?.({ location: { directory: "/work/old" } });
    await delay(200);

    const states = connection.stateMessages();
    const last = states[states.length - 1] as WireMessage;
    assert.equal(last.activeSessionID, "ses-b");
    const sessions = last.sessions as Array<{ sessionID: string; directory: string | null }>;
    assert.equal(sessions.find((session) => session.sessionID === "ses-b")?.directory, "/work/new");
    assert.equal(states.length, statesBeforeStaleResolution);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("reports focus success and failure without throwing", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    harness.state.tabs = [
      { sessionID: "ses-a", title: "Alpha", active: true },
      { sessionID: "ses-b", title: "Beta", active: false },
    ];
    harness.state.focusResult = (sessionID) => {
      harness.state.tabs = harness.state.tabs.map((tab) => ({
        ...tab,
        active: tab.sessionID === sessionID,
      }));
      return true;
    };

    const connection = await bridge.connection();
    await connection.waitForMessage("state");

    connection.send({ type: "focus", requestID: "req-1", sessionID: "ses-b" });
    const success = await connection.waitForMessage(
      "focus_result",
      (message) => message.requestID === "req-1",
    );
    assert.equal(success.ok, true);
    assert.equal(success.error, undefined);
    assert.deepEqual(harness.state.focusCalls, ["ses-b"]);

    // OpenCode would rerun the reactive effect after tabs.focus changes tabs.
    harness.trigger();
    const focused = await connection.waitForMessage(
      "state",
      (message) => message.activeSessionID === "ses-b",
    );
    assert.equal(focused.activeSessionID, "ses-b");

    harness.state.focusResult = () => false;
    connection.send({ type: "focus", requestID: "req-2", sessionID: "ses-missing" });
    const failure = await connection.waitForMessage(
      "focus_result",
      (message) => message.requestID === "req-2",
    );
    assert.equal(failure.ok, false);
    assert.equal(typeof failure.error, "string");
    assert.ok(String(failure.error).length > 0);

    harness.state.focusResult = () => {
      throw new Error(`focus failed for ${TOKEN}`);
    };
    connection.send({ type: "focus", requestID: "req-3", sessionID: "ses-a" });
    const thrown = await connection.waitForMessage(
      "focus_result",
      (message) => message.requestID === "req-3",
    );
    assert.equal(thrown.ok, false);
    assert.ok(!String(thrown.error).includes(TOKEN));
    assert.ok(String(thrown.error).includes("[redacted]"));
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("answers ping with pong and request_state with a fresh full snapshot", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    const connection = await bridge.connection();
    await connection.waitForMessage("state");

    connection.send({ type: "ping", nonce: "nonce-1", unknown: "ignored" });
    const pong = await connection.waitForMessage("pong");
    assert.deepEqual(pong, { type: "pong", nonce: "nonce-1" });

    connection.send({ type: "request_state", requestID: "req-state-1" });
    const state = await connection.waitForMessage(
      "state",
      (message) => message.revision === 2,
    );
    assert.equal(state.revision, 2);
    assert.equal(state.activeSessionID, null);
    assert.deepEqual(state.sessions, []);

    // Unknown message types are ignored without a response or a crash.
    connection.send({ type: "mystery", payload: true });
    await delay(100);
    assert.equal(connection.messages.filter((message) => message.type === "mystery").length, 0);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("reconnects after a drop with a new revision epoch", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    const first = await bridge.connection();
    await first.waitForMessage("state");

    harness.state.tabs = [{ sessionID: "ses-a", title: "Reconnect me", active: true }];
    harness.trigger();
    await first.waitForMessage(
      "state",
      (message) => message.revision === 2,
    );

    first.terminate();
    const second = await waitFor(
      () => bridge.connections[1],
      "reconnect connection",
    );
    const hello = await second.waitForMessage("hello");
    assert.equal(hello.paneId, "pane-test");

    const state = await second.waitForMessage("state");
    assert.equal(state.revision, 1);
    assert.equal(
      (state.sessions as Array<{ title?: string }>)[0]?.title,
      "Reconnect me",
    );
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("sends goodbye on unload and does not reconnect", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    const connection = await bridge.connection();
    await connection.waitForMessage("state");

    harness.dispose();
    const goodbye = await connection.waitForMessage("goodbye");
    assert.equal(goodbye.type, "goodbye");
    assert.equal(goodbye.reason, "plugin unloaded");

    const closed = await waitFor(
      () => (connection.closed ? true : undefined),
      "socket close",
    );
    assert.equal(closed, true);
    await delay(200);
    assert.equal(bridge.connections.length, 1);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("stays inert and keeps the effect alive when the bridge is unreachable", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  await bridge.close();

  const harness = createHarness(url);
  try {
    await delay(250);
    assert.equal(harness.effectCalls(), 1);

    harness.state.tabs = [{ sessionID: "ses-a", title: "Still usable", active: true }];
    harness.state.sessions.set("ses-a", { location: { directory: "/work/a" } });
    assert.doesNotThrow(() => harness.trigger());
    await delay(100);
  } finally {
    harness.dispose();
  }
});

test("rejects invalid options without connecting or throwing", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const disposers: Array<() => void> = [];
  const state = createState();
  let effectCalls = 0;

  try {
    const invalidOptions: Array<Record<string, unknown>> = [
      {
        bridgeUrl: "ws://192.168.1.20:4444/made",
        token: TOKEN,
        paneId: "pane-test",
        protocolVersion: 1,
        appVersion: "0.1.0-test",
      },
      {
        bridgeUrl: "ws://127.0.0.1/made",
        token: TOKEN,
        paneId: "pane-test",
        protocolVersion: 1,
        appVersion: "0.1.0-test",
      },
      { bridgeUrl: url, token: "short", paneId: "pane-test", protocolVersion: 1, appVersion: "x" },
    ];

    for (const made of invalidOptions) {
      assert.doesNotThrow(() => {
        disposers.push(
          createIntegration(createContext(made, state), (fn) => {
            effectCalls += 1;
            fn();
          }),
        );
      });
    }

    await delay(300);
    assert.equal(effectCalls, 0);
    assert.equal(bridge.connections.length, 0);
    assert.equal(state.toasts.length, invalidOptions.length);
    for (const message of state.toasts) {
      assert.ok(!message.includes(TOKEN));
    }
  } finally {
    for (const dispose of disposers) dispose();
    await bridge.close();
  }
});

test("redacts the token from session lookup errors", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    const connection = await bridge.connection();
    await connection.waitForMessage("state");

    harness.state.tabs = [{ sessionID: "ses-a", title: "Broken", active: true }];
    harness.state.sessionError = new Error(`session lookup failed with ${TOKEN}`);
    harness.trigger();

    const error = await connection.waitForMessage("error");
    assert.equal(error.type, "error");
    assert.equal(error.code, "session_get_failed");
    assert.equal(error.fatal, false);
    assert.ok(!String(error.message).includes(TOKEN));
    assert.ok(String(error.message).includes("[redacted]"));

    const serialized = JSON.stringify({
      errors: connection.messages.filter((message) => message.type === "error"),
      toasts: harness.state.toasts,
    });
    assert.ok(!serialized.includes(TOKEN));
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("accepts only explicit loopback websocket URLs and namespaced options", () => {
  assert.equal(isLoopbackWebSocketURL("ws://127.0.0.1:45037/made"), true);
  assert.equal(isLoopbackWebSocketURL("ws://127.0.0.2:1/made"), true);
  assert.equal(isLoopbackWebSocketURL("ws://[::1]:45037/made"), true);
  assert.equal(isLoopbackWebSocketURL("ws://localhost:45037/made"), false);
  assert.equal(isLoopbackWebSocketURL("wss://127.0.0.1:45037/made"), false);
  assert.equal(isLoopbackWebSocketURL("ws://0.0.0.0:45037/made"), false);
  assert.equal(isLoopbackWebSocketURL("ws://192.168.1.20:45037/made"), false);
  assert.equal(isLoopbackWebSocketURL("ws://127.0.0.1/made"), false);
  assert.equal(isLoopbackWebSocketURL("http://127.0.0.1:45037/made"), false);

  const valid = readMadeOptions({
    made: {
      bridgeUrl: "ws://127.0.0.1:45037/made",
      token: TOKEN,
      paneId: "pane-1",
      protocolVersion: 1,
      appVersion: "0.1.0",
    },
  });
  assert.equal(valid.ok, true);

  assert.equal(readMadeOptions({}).ok, false);
  const mismatched = readMadeOptions({
    made: {
      bridgeUrl: "ws://127.0.0.1:45037/made",
      token: TOKEN,
      paneId: "pane-1",
      protocolVersion: 2,
      appVersion: "0.1.0",
    },
  });
  // A version mismatch is the bridge's decision (4402), not a local refusal.
  assert.equal(mismatched.ok, true);
  if (mismatched.ok) assert.equal(mismatched.options.protocolVersion, 2);

  assert.equal(
    readMadeOptions({
      made: {
        bridgeUrl: "ws://127.0.0.1:45037/made",
        token: TOKEN,
        paneId: "pane-1",
        protocolVersion: "1",
        appVersion: "0.1.0",
      },
    }).ok,
    false,
  );
});

test("caps snapshots at 64 sessions and keeps a single active tab", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    const connection = await bridge.connection();
    await connection.waitForMessage("state");

    harness.state.tabs = Array.from({ length: 65 }, (_, index) => ({
      sessionID: `ses-${index}`,
      title: `Tab ${index}`,
      active: index === 0,
    }));
    harness.trigger();

    const state = await connection.waitForMessage(
      "state",
      (message) => Array.isArray(message.sessions) && message.sessions.length === 64,
    );
    const sessions = state.sessions as Array<{ sessionID: string; active: boolean }>;
    assert.equal(sessions.length, 64);
    assert.equal(sessions[63]?.sessionID, "ses-63");
    assert.equal(state.activeSessionID, "ses-0");
    assert.equal(sessions.filter((session) => session.active).length, 1);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("deduplicates session ids and keeps the first active selection", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    const connection = await bridge.connection();
    await connection.waitForMessage("state");

    harness.state.tabs = [
      { sessionID: "ses-a", title: "First", active: false },
      { sessionID: "ses-a", title: "Second", active: true },
      { sessionID: "ses-b", title: "Other", active: false },
    ];
    harness.trigger();

    const state = await connection.waitForMessage(
      "state",
      (message) => Array.isArray(message.sessions) && message.sessions.length === 2,
    );
    const sessions = state.sessions as Array<{
      sessionID: string;
      title: string;
      active: boolean;
    }>;
    assert.deepEqual(
      sessions.map((session) => session.sessionID),
      ["ses-a", "ses-b"],
    );
    assert.equal(sessions[0]?.title, "First");
    assert.equal(state.activeSessionID, "ses-a");
    assert.equal(sessions.filter((session) => session.active).length, 1);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("marks only the first of two active tabs active", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    const connection = await bridge.connection();
    await connection.waitForMessage("state");

    harness.state.tabs = [
      { sessionID: "ses-a", title: "Alpha", active: true },
      { sessionID: "ses-b", title: "Beta", active: true },
    ];
    harness.trigger();

    const state = await connection.waitForMessage(
      "state",
      (message) => Array.isArray(message.sessions) && message.sessions.length === 2,
    );
    const sessions = state.sessions as Array<{ sessionID: string; active: boolean }>;
    assert.equal(state.activeSessionID, "ses-a");
    assert.equal(sessions.filter((session) => session.active).length, 1);
    assert.equal(
      sessions.find((session) => session.sessionID === "ses-a")?.active,
      true,
    );
    assert.equal(
      sessions.find((session) => session.sessionID === "ses-b")?.active,
      false,
    );
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("normalizes fractional, non-finite, negative, and oversized promptPulse values", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    const connection = await bridge.connection();
    await connection.waitForMessage("state");

    harness.state.tabs = [
      { sessionID: "ses-fraction", promptPulse: 1.5 },
      { sessionID: "ses-nan", promptPulse: Number.NaN },
      { sessionID: "ses-infinity", promptPulse: Number.POSITIVE_INFINITY },
      { sessionID: "ses-huge", promptPulse: Number.MAX_SAFE_INTEGER },
      { sessionID: "ses-negative", promptPulse: -3 },
      { sessionID: "ses-normal", promptPulse: 7 },
    ];
    harness.trigger();

    const state = await connection.waitForMessage(
      "state",
      (message) => Array.isArray(message.sessions) && message.sessions.length === 6,
    );
    const sessions = state.sessions as Array<{
      sessionID: string;
      promptPulse: number;
      unread: boolean;
    }>;
    const byID = new Map(sessions.map((session) => [session.sessionID, session]));

    for (const id of ["ses-fraction", "ses-nan", "ses-infinity", "ses-negative"]) {
      assert.equal(byID.get(id)?.promptPulse, 0, id);
      assert.equal(byID.get(id)?.unread, false, id);
    }
    assert.equal(byID.get("ses-huge")?.promptPulse, 0xffff_ffff);
    assert.equal(byID.get("ses-huge")?.unread, true);
    assert.equal(byID.get("ses-normal")?.promptPulse, 7);
    for (const session of sessions) {
      assert.equal(Number.isInteger(session.promptPulse), true);
      assert.ok(session.promptPulse >= 0 && session.promptPulse <= 0xffff_ffff);
    }
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("truncates emoji titles and paths on code point boundaries", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    const connection = await bridge.connection();
    await connection.waitForMessage("state");

    const emoji = "\u{1f600}";
    harness.state.tabs = [
      {
        sessionID: "ses-emoji",
        title: emoji.repeat(600),
        active: true,
      },
    ];
    harness.state.sessions.set("ses-emoji", {
      title: emoji.repeat(600),
      projectID: emoji.repeat(600),
      subpath: emoji.repeat(6000),
      location: { directory: `/${emoji.repeat(5000)}` },
    });
    harness.trigger();

    const state = await connection.waitForMessage(
      "state",
      (message) => Array.isArray(message.sessions) && message.sessions.length === 1,
    );
    const session = (state.sessions as Array<{
      title: string;
      directory: string;
      subpath: string;
      projectID: string;
    }>)[0];
    assert.ok(session);

    assert.equal(session.title, emoji.repeat(MAX_TITLE_LENGTH));
    assert.equal(session.directory, `/${emoji.repeat(MAX_PATH_LENGTH - 1)}`);
    assert.equal(session.subpath, emoji.repeat(MAX_PATH_LENGTH));
    assert.equal(session.projectID, emoji.repeat(MAX_PROJECT_ID_LENGTH));
    assert.equal(session.title.length, MAX_TITLE_LENGTH * 2);
    assert.equal(hasLoneSurrogate(session.title), false);
    assert.equal(hasLoneSurrogate(session.directory), false);
    assert.equal(hasLoneSurrogate(session.subpath), false);
    assert.equal(hasLoneSurrogate(session.projectID), false);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("fits an oversized snapshot under the bridge message limit", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    const connection = await bridge.connection();
    await connection.waitForMessage("state");

    const emoji = "\u{1f600}";
    const count = 64;
    harness.state.tabs = Array.from({ length: count }, (_, index) => ({
      sessionID: `ses-${index}`,
      title: emoji.repeat(MAX_TITLE_LENGTH),
      active: index === 0,
    }));
    for (let index = 0; index < count; index += 1) {
      harness.state.sessions.set(`ses-${index}`, {
        projectID: emoji.repeat(MAX_PROJECT_ID_LENGTH),
        subpath: emoji.repeat(MAX_PATH_LENGTH),
        location: { directory: `/${emoji.repeat(MAX_PATH_LENGTH - 1)}` },
      });
    }
    harness.trigger();

    const state = await connection.waitForMessage(
      "state",
      (message) => Array.isArray(message.sessions) && message.sessions.length === count,
    );
    const sessions = state.sessions as Array<{
      sessionID: string;
      title: string;
      directory: string;
      subpath: string;
      projectID: string;
      active: boolean;
    }>;
    assert.equal(sessions.length, count);
    assert.equal(new Set(sessions.map((session) => session.sessionID)).size, count);
    assert.equal(state.activeSessionID, "ses-0");
    assert.equal(sessions.filter((session) => session.active).length, 1);

    // The input is several megabytes before fitting, so at least one long
    // field must have been shortened to make the snapshot fit.
    assert.ok(Array.from(sessions[0]?.title ?? "").length < MAX_TITLE_LENGTH);

    for (const session of sessions) {
      assert.ok(Array.from(session.title).length <= MAX_TITLE_LENGTH);
      assert.ok(Array.from(session.directory).length <= MAX_PATH_LENGTH);
      assert.ok(Array.from(session.subpath).length <= MAX_PATH_LENGTH);
      assert.ok(Array.from(session.projectID).length <= MAX_PROJECT_ID_LENGTH);
    }

    const largest = Math.max(...connection.rawMessages.map(byteLength));
    assert.ok(
      largest <= MAX_MESSAGE_BYTES,
      `outbound JSON was ${largest} bytes; limit is ${MAX_MESSAGE_BYTES}`,
    );
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("sets tabsEnabled false when tabs.list throws even if enabled was true", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    const connection = await bridge.connection();
    const initial = await connection.waitForMessage("state");
    assert.equal(initial.tabsEnabled, true);

    harness.state.listError = new Error("tabs list exploded");
    harness.trigger();

    const state = await connection.waitForMessage(
      "state",
      (message) => message.tabsEnabled === false,
    );
    assert.equal(state.tabsEnabled, false);
    assert.deepEqual(state.sessions, []);

    const error = await connection.waitForMessage("error");
    assert.equal(error.code, "tabs_unavailable");
    assert.ok(!String(error.message).includes(TOKEN));

    harness.state.listError = undefined;
    harness.trigger();
    const recovered = await connection.waitForMessage(
      "state",
      (message) => message.tabsEnabled === true,
    );
    assert.equal(recovered.tabsEnabled, true);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("does not focus a session that is no longer open", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url);

  try {
    const connection = await bridge.connection();
    await connection.waitForMessage("state");

    harness.state.tabs = [{ sessionID: "ses-a", title: "Alpha", active: true }];
    harness.trigger();
    await connection.waitForMessage(
      "state",
      (message) => Array.isArray(message.sessions) && message.sessions.length === 1,
    );

    harness.state.tabs = [];
    harness.trigger();
    await connection.waitForMessage(
      "state",
      (message) => Array.isArray(message.sessions) && message.sessions.length === 0,
    );

    connection.send({ type: "focus", requestID: "req-closed", sessionID: "ses-a" });
    const result = await connection.waitForMessage(
      "focus_result",
      (message) => message.requestID === "req-closed",
    );
    assert.equal(result.ok, false);
    assert.equal(result.error, "session not open");
    assert.deepEqual(harness.state.focusCalls, []);
    assert.ok(!String(result.error).includes(TOKEN));
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("sends hello with the plugin protocol version when the option differs", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url, { protocolVersion: 2 });

  try {
    const connection = await bridge.connection();
    const hello = await connection.waitForMessage("hello");
    assert.equal(hello.protocolVersion, PROTOCOL_VERSION);
    assert.equal(hello.protocolVersion, 1);
    // The connection still becomes usable when the bridge accepts v1.
    const state = await connection.waitForMessage("state");
    assert.equal(state.revision, 1);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

for (const closeCode of [4401, 4402, 4403, 4409]) {
  test(`stops reconnecting after terminal close ${closeCode}`, async () => {
    const bridge = new TestBridge();
    if (closeCode === 4409) {
      bridge.welcomeCloseCode = closeCode;
    } else {
      bridge.helloCloseCode = closeCode;
    }
    const url = await bridge.start();
    const harness = createHarness(url);

    try {
      const connection = await bridge.connection();
      await waitFor(() => (connection.closed ? true : undefined), "connection close");
      await delay(500);
      assert.equal(bridge.connections.length, 1);
    } finally {
      harness.dispose();
      await bridge.close();
    }
  });
}

for (const closeCode of [1008, 1009]) {
  test(`stops after the fatal-rejection budget for close ${closeCode}`, async () => {
    const bridge = new TestBridge();
    const url = await bridge.start();
    const harness = createHarness(url, {
      tuning: { maxConsecutiveFatalClosures: 2 },
    });

    try {
      for (let attempt = 0; attempt < 2; attempt += 1) {
        const connection = await bridge.connection(attempt);
        await connection.waitForMessage("state");
        connection.socket.close(closeCode, "test fatal rejection");
      }

      await delay(900);
      assert.equal(bridge.connections.length, 2);
    } finally {
      harness.dispose();
      await bridge.close();
    }
  });
}

test("resets the fatal-rejection budget only after a stable welcome+state period", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url, {
    tuning: { maxConsecutiveFatalClosures: 2, stableConnectionMs: 50 },
  });

  try {
    const first = await bridge.connection(0);
    await first.waitForMessage("state");
    first.socket.close(1008, "test fatal rejection");

    const second = await bridge.connection(1);
    await second.waitForMessage("state");
    // Hold welcome+state beyond the stable period; this must clear the budget.
    await delay(250);
    second.socket.close(1008, "test fatal rejection");

    const third = await bridge.connection(2);
    const state = await third.waitForMessage("state");
    assert.equal(state.revision, 1);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});
