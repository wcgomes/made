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
  MAX_PATH_LENGTH,
  MAX_PROJECT_ID_LENGTH,
  MAX_TITLE_LENGTH,
  PROTOCOL_VERSION,
} from "../src/protocol.ts";

const TOKEN = "test-token-0123456789abcdef0123456789abcdef";
const WAIT_MS = 3000;

interface WireMessage {
  [key: string]: unknown;
  type?: string;
}

interface ViewState {
  tabs: TuiTab[];
  sessions: Map<string, unknown>;
  enabled: boolean;
  location: { directory?: string } | undefined;
  toasts: string[];
  listError: Error | undefined;
}

interface ViewHarness {
  state: ViewState;
  trigger(): void;
  dispose(): void;
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
        connection.send({
          type: "welcome",
          protocolVersion: PROTOCOL_VERSION,
          appVersion: "0.1.0-test",
          heartbeatIntervalMs: 1000,
        });
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
    return waitFor(() => this.connections[index], `connection ${index + 1}`);
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

function createState(location: { directory?: string } | undefined): ViewState {
  return {
    tabs: [],
    sessions: new Map<string, unknown>(),
    enabled: true,
    location,
    toasts: [],
    listError: undefined,
  };
}

function createContext(made: Record<string, unknown>, state: ViewState): TuiPluginContext {
  return {
    options: { made },
    app: { version: "2.2.0" },
    get location() {
      return state.location;
    },
    ui: {
      tabs: {
        enabled: () => state.enabled,
        list: () => {
          if (state.listError) throw state.listError;
          return state.tabs.map((tab) => ({ ...tab }));
        },
        focus: () => true,
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
  tuning?: IntegrationTuning;
}

function createHarness(
  bridgeUrl: string,
  location: { directory?: string } | undefined,
  overrides: HarnessOptions = {},
): ViewHarness {
  const state = createState(location);
  const context = createContext(
    {
      bridgeUrl,
      token: TOKEN,
      paneId: "pane-test",
      protocolVersion: PROTOCOL_VERSION,
      appVersion: "0.1.0-test",
    },
    state,
  );

  let effectFn: (() => void) | undefined;
  const dispose = createIntegration(
    context,
    (fn) => {
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
  };
}

function byteLength(value: string): number {
  return Buffer.byteLength(value, "utf8");
}

test("reports a usable view directory before any session exists", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url, { directory: "/work/view-a" });

  try {
    const connection = await bridge.connection();
    const state = await connection.waitForMessage("state");

    assert.equal(state.activeSessionID, null);
    assert.equal(state.tabsEnabled, true);
    assert.equal(state.viewDirectory, "/work/view-a");
    assert.deepEqual(state.sessions, []);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("reports null when no usable view directory is available", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url, undefined);

  try {
    const connection = await bridge.connection();
    const state = await connection.waitForMessage("state");
    assert.equal(state.viewDirectory, null);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("follows a view directory change with no active session", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url, { directory: "/work/view-a" });

  try {
    const connection = await bridge.connection();
    const first = await connection.waitForMessage("state");
    assert.equal(first.viewDirectory, "/work/view-a");

    harness.state.location = { directory: "/work/view-b" };
    harness.trigger();

    const second = await connection.waitForMessage(
      "state",
      (message) => message.viewDirectory === "/work/view-b",
    );
    assert.ok((second.revision as number) > (first.revision as number));
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("emits null while an active session exists and resumes the view when it closes", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url, { directory: "/work/view-a" });

  try {
    const connection = await bridge.connection();
    await connection.waitForMessage("state");

    harness.state.tabs = [{ sessionID: "ses-a", title: "Alpha", active: true }];
    harness.state.sessions.set("ses-a", { location: { directory: "/work/session-a" } });
    harness.trigger();

    const active = await connection.waitForMessage(
      "state",
      (message) => message.activeSessionID === "ses-a",
    );
    assert.equal(active.viewDirectory, null);

    // A view change while a session is active must not replace the null.
    harness.state.location = { directory: "/work/view-b" };
    harness.state.tabs = [{ sessionID: "ses-a", title: "Alpha 2", active: true }];
    harness.trigger();
    const activeChanged = await connection.waitForMessage(
      "state",
      (message) => (message.sessions as Array<{ title?: string }>)[0]?.title === "Alpha 2",
    );
    assert.equal(activeChanged.viewDirectory, null);

    // Closing the session makes the current view usable again.
    harness.state.tabs = [];
    harness.trigger();
    const resumed = await connection.waitForMessage(
      "state",
      (message) =>
        Array.isArray(message.sessions) &&
        message.sessions.length === 0 &&
        message.viewDirectory === "/work/view-b",
    );
    assert.equal(resumed.activeSessionID, null);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("clears the cached view when an active session closes into a transient undefined", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url, { directory: "/work/view-a" });

  try {
    const connection = await bridge.connection();
    const initial = await connection.waitForMessage("state");
    assert.equal(initial.viewDirectory, "/work/view-a");

    harness.state.tabs = [{ sessionID: "ses-a", title: "Alpha", active: true }];
    harness.state.sessions.set("ses-a", { location: { directory: "/work/session-a" } });
    harness.trigger();
    const active = await connection.waitForMessage(
      "state",
      (message) => message.activeSessionID === "ses-a",
    );
    assert.equal(active.viewDirectory, null);

    // The active state must clear the cache, not merely suppress it. If it did
    // not, the pre-session value would be retained across this undefined.
    harness.state.tabs = [];
    harness.state.location = undefined;
    harness.trigger();
    const closed = await connection.waitForMessage(
      "state",
      (message) =>
        Array.isArray(message.sessions) &&
        message.sessions.length === 0 &&
        message.activeSessionID === null &&
        (message.revision as number) > (active.revision as number),
    );
    assert.equal(closed.viewDirectory, null);
    assert.ok((closed.revision as number) > (active.revision as number));

    // A later usable value is observed again.
    harness.state.location = { directory: "/work/view-b" };
    harness.trigger();
    const recovered = await connection.waitForMessage(
      "state",
      (message) => message.viewDirectory === "/work/view-b",
    );
    assert.equal(recovered.activeSessionID, null);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("emits null while session tabs are unavailable", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url, { directory: "/work/view-a" });

  try {
    const connection = await bridge.connection();
    const first = await connection.waitForMessage("state");
    assert.equal(first.viewDirectory, "/work/view-a");

    harness.state.enabled = false;
    harness.state.location = { directory: "/work/view-b" };
    harness.trigger();

    const disabled = await connection.waitForMessage(
      "state",
      (message) => message.tabsEnabled === false,
    );
    assert.equal(disabled.viewDirectory, null);

    harness.state.location = { directory: "/work/view-c" };
    harness.state.tabs = [{ sessionID: "ses-a", title: "hidden" }];
    harness.trigger();
    const disabledAgain = await connection.waitForMessage(
      "state",
      (message) =>
        message.tabsEnabled === false &&
        Array.isArray(message.sessions) &&
        message.sessions.length === 1,
    );
    assert.equal(disabledAgain.viewDirectory, null);

    // Disabling tabs clears the cache. Re-enabling while the location is
    // transiently undefined must not resurrect the pre-disable value.
    harness.state.location = undefined;
    harness.state.enabled = true;
    harness.trigger();
    const cleared = await connection.waitForMessage(
      "state",
      (message) =>
        message.tabsEnabled === true &&
        Array.isArray(message.sessions) &&
        message.sessions.length === 1 &&
        message.viewDirectory === null,
    );
    assert.equal(cleared.activeSessionID, null);

    // A later usable value is observed again.
    harness.state.location = { directory: "/work/view-c" };
    harness.trigger();
    const recovered = await connection.waitForMessage(
      "state",
      (message) => message.tabsEnabled === true && message.viewDirectory === "/work/view-c",
    );
    assert.equal(recovered.activeSessionID, null);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("retains the last known view directory across a transient undefined", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url, { directory: "/work/view-a" });

  try {
    const connection = await bridge.connection();
    await connection.waitForMessage("state");

    harness.state.location = undefined;
    harness.state.tabs = [{ sessionID: "ses-a", title: "Inactive" }];
    harness.trigger();
    const retained = await connection.waitForMessage(
      "state",
      (message) => Array.isArray(message.sessions) && message.sessions.length === 1,
    );
    assert.equal(retained.viewDirectory, "/work/view-a");

    // An object with an undefined directory is transient too.
    harness.state.location = { directory: undefined };
    harness.state.tabs = [{ sessionID: "ses-a", title: "Inactive 2" }];
    harness.trigger();
    const retainedAgain = await connection.waitForMessage(
      "state",
      (message) => (message.sessions as Array<{ title?: string }>)[0]?.title === "Inactive 2",
    );
    assert.equal(retainedAgain.viewDirectory, "/work/view-a");

    harness.state.location = { directory: "/work/view-b" };
    harness.trigger();
    const changed = await connection.waitForMessage(
      "state",
      (message) => message.viewDirectory === "/work/view-b",
    );
    assert.ok((changed.revision as number) > (retainedAgain.revision as number));
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("omits invalid view directories and reports a non-fatal redacted error", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url, { directory: "/work/view-a" });

  try {
    const connection = await bridge.connection();
    await connection.waitForMessage("state");

    const cases: Array<{ name: string; path: string; marker: string; reason: string }> = [
      { name: "empty", path: "", marker: "is empty", reason: "empty" },
      {
        name: "relative",
        path: `${TOKEN}/relative-view`,
        marker: "relative-view",
        reason: "absolute",
      },
      { name: "nul", path: "/work/nul-\0-view", marker: "nul-\0-view", reason: "NUL" },
      { name: "dot", path: "/work/./view", marker: "/work/./view", reason: "dot" },
      { name: "parent", path: "/work/../view", marker: "/work/../view", reason: "dot" },
      {
        name: "oversized",
        path: `/case-oversized/${"a".repeat(MAX_PATH_LENGTH)}`,
        marker: "case-oversized",
        reason: "code points",
      },
    ];

    for (const [index, entry] of cases.entries()) {
      harness.state.location = { directory: entry.path };
      harness.state.tabs = [{ sessionID: "ses-view", title: `case-${index}` }];
      harness.trigger();

      const state = await connection.waitForMessage(
        "state",
        (message) => (message.sessions as Array<{ title?: string }>)[0]?.title === `case-${index}`,
      );
      assert.equal(state.viewDirectory, null, entry.name);
      if (entry.name === "oversized") {
        assert.notEqual(state.viewDirectory, entry.path.slice(0, MAX_PATH_LENGTH));
      }

      const error = await connection.waitForMessage(
        "error",
        (message) =>
          message.code === "view_directory_invalid" &&
          String(message.message).includes(entry.marker),
      );
      assert.equal(error.fatal, false, entry.name);
      assert.ok(String(error.message).includes(entry.reason), entry.name);
      if (entry.name === "relative") {
        assert.ok(!String(error.message).includes(TOKEN), entry.name);
        assert.ok(String(error.message).includes("[redacted]"), entry.name);
      }
    }

    // A valid value after invalid ones is accepted and keeps the connection usable.
    harness.state.location = { directory: "/work/view-recovered" };
    harness.state.tabs = [{ sessionID: "ses-view", title: "recovered" }];
    harness.trigger();
    const recovered = await connection.waitForMessage(
      "state",
      (message) => (message.sessions as Array<{ title?: string }>)[0]?.title === "recovered",
    );
    assert.equal(recovered.viewDirectory, "/work/view-recovered");
    assert.equal(connection.closed, false);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("clears the view directory on a new connection epoch and on unload", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url, { directory: "/work/view-a" });

  try {
    const first = await bridge.connection();
    const initial = await first.waitForMessage("state");
    assert.equal(initial.viewDirectory, "/work/view-a");

    first.terminate();
    harness.state.location = undefined;

    const second = await waitFor(() => bridge.connections[1], "reconnect connection");
    const epochState = await second.waitForMessage("state");
    assert.equal(epochState.viewDirectory, null);
    assert.equal(epochState.revision, 1);

    // The reactive effect still follows the current view after the epoch reset.
    harness.state.location = { directory: "/work/view-b" };
    harness.trigger();
    const followed = await second.waitForMessage(
      "state",
      (message) => message.viewDirectory === "/work/view-b",
    );
    assert.ok((followed.revision as number) > 1);

    const statesBeforeUnload = second.stateMessages().length;
    harness.dispose();
    const goodbye = await second.waitForMessage("goodbye");
    assert.equal(goodbye.reason, "plugin unloaded");
    const closed = await waitFor(() => (second.closed ? true : undefined), "socket close");
    assert.equal(closed, true);

    assert.doesNotThrow(() => harness.trigger());
    await delay(150);
    assert.equal(second.stateMessages().length, statesBeforeUnload);
  } finally {
    harness.dispose();
    await bridge.close();
  }
});

test("includes the view directory in change detection and snapshot sizing", async () => {
  const bridge = new TestBridge();
  const url = await bridge.start();
  const harness = createHarness(url, { directory: "/work/view-a" });

  try {
    const connection = await bridge.connection();
    const first = await connection.waitForMessage("state");
    assert.equal(first.viewDirectory, "/work/view-a");

    // Changing only the view directory must produce a new snapshot.
    harness.state.location = { directory: "/work/view-b" };
    harness.trigger();
    const changed = await connection.waitForMessage(
      "state",
      (message) => message.viewDirectory === "/work/view-b",
    );
    assert.ok((changed.revision as number) > (first.revision as number));

    // A maximum-length view path participates in sizing and is never truncated.
    const emoji = "\u{1f600}";
    const longView = `/${emoji.repeat(MAX_PATH_LENGTH - 1)}`;
    assert.equal(Array.from(longView).length, MAX_PATH_LENGTH);

    harness.state.location = { directory: longView };
    harness.state.tabs = Array.from({ length: 64 }, (_, index) => ({
      sessionID: `ses-${index}`,
      title: emoji.repeat(MAX_TITLE_LENGTH),
      active: false,
    }));
    for (let index = 0; index < 64; index += 1) {
      harness.state.sessions.set(`ses-${index}`, {
        projectID: emoji.repeat(MAX_PROJECT_ID_LENGTH),
        subpath: emoji.repeat(MAX_PATH_LENGTH),
        location: { directory: `/${emoji.repeat(MAX_PATH_LENGTH - 1)}` },
      });
    }
    harness.trigger();

    const sized = await connection.waitForMessage(
      "state",
      (message) =>
        Array.isArray(message.sessions) &&
        message.sessions.length > 0 &&
        message.viewDirectory !== undefined,
    );
    const sizedView = sized.viewDirectory;
    assert.ok(sizedView === null || sizedView === longView);
    if (typeof sizedView === "string") {
      assert.equal(sizedView, longView);
      assert.equal(Array.from(sizedView).length, MAX_PATH_LENGTH);
    }

    const sessions = sized.sessions as Array<{
      title: string;
      directory: string;
      subpath: string;
    }>;
    assert.ok(sessions.length <= 64);
    for (const session of sessions) {
      assert.ok(Array.from(session.title).length <= MAX_TITLE_LENGTH);
      assert.ok(Array.from(session.directory).length <= MAX_PATH_LENGTH);
      assert.ok(Array.from(session.subpath).length <= MAX_PATH_LENGTH);
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
