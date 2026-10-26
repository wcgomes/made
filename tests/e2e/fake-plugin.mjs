// Test-only fake OpenCode TUI plugin for the Made app session-follow E2E.
//
// It is launched as the terminal shell through tests/e2e/fake-plugin-wrapper.sh.
// It reads the per-launch OPENCODE_CLI_CONFIG_CONTENT that Made composes,
// extracts the `made` plugin options, connects to the loopback bridge with
// Node 24's global WebSocket, authenticates with `hello`, and then sends two
// valid `state` revisions:
//
//   1. active session in worktree A
//   2. active session in worktree B
//
// This validates Made's app-level bridge -> follow -> explorer wiring without
// claiming real OpenCode tab evidence. It never logs the bridge token.

import fs from "node:fs";
import process from "node:process";

const logPath = process.env.MADE_E2E_FAKE_LOG || "";
const worktreeA = process.env.MADE_E2E_WORKTREE_A || "";
const worktreeB = process.env.MADE_E2E_WORKTREE_B || "";
const sessionA = process.env.MADE_E2E_SESSION_A || "ses_e2e_worktree_a";
const sessionB = process.env.MADE_E2E_SESSION_B || "ses_e2e_worktree_b";
const switchDelayMs = Number(process.env.MADE_E2E_SWITCH_DELAY_MS || "1200");
const titleA = process.env.MADE_E2E_TITLE_A || "E2E session in worktree A";
const titleB = process.env.MADE_E2E_TITLE_B || "E2E session in worktree B";
const nodePath = process.env.MADE_E2E_NODE || process.execPath;

function log(event, fields = {}) {
  const entry = { ts: new Date().toISOString(), event, ...fields };
  const line = JSON.stringify(entry);
  if (logPath) {
    try {
      fs.appendFileSync(logPath, line + "\n");
    } catch (error) {
      process.stderr.write(`fake-plugin: log write failed: ${error.message}\n`);
    }
  }
  process.stderr.write(`fake-plugin: ${line}\n`);
}

function fail(message, code = 2) {
  log("fatal", { message });
  process.stderr.write(`fake-plugin: ${message}\n`);
  process.exit(code);
}

if (!worktreeA || !worktreeB) {
  fail("MADE_E2E_WORKTREE_A and MADE_E2E_WORKTREE_B are required");
}
for (const [label, directory] of [
  ["A", worktreeA],
  ["B", worktreeB],
]) {
  let stat;
  try {
    stat = fs.statSync(directory);
  } catch (error) {
    fail(`worktree ${label} is unavailable: ${error.message}`);
  }
  if (!stat.isDirectory()) {
    fail(`worktree ${label} is not a directory: ${directory}`);
  }
}

let config;
try {
  const content = process.env.OPENCODE_CLI_CONFIG_CONTENT;
  if (!content) {
    fail("OPENCODE_CLI_CONFIG_CONTENT is absent");
  }
  config = JSON.parse(content);
} catch (error) {
  fail(`could not parse OPENCODE_CLI_CONFIG_CONTENT: ${error.message}`);
}

const plugins = Array.isArray(config.plugins) ? config.plugins : [];
const madeEntry = plugins.find(
  (entry) => entry && typeof entry === "object" && entry.options && entry.options.made,
);
if (!madeEntry) {
  fail("composed config has no plugins[].options.made entry");
}
const made = madeEntry.options.made;
for (const key of ["bridgeUrl", "token", "paneId", "protocolVersion", "appVersion"]) {
  if (made[key] === undefined || made[key] === null || made[key] === "") {
    fail(`composed made options are missing ${key}`);
  }
}
if (!/^ws:\/\/127\.0\.0\.1:\d+\/made$/.test(String(made.bridgeUrl))) {
  fail(`unexpected bridge URL shape: ${String(made.bridgeUrl)}`);
}
if (Number(made.protocolVersion) !== 1) {
  fail(`unexpected protocol version: ${String(made.protocolVersion)}`);
}

log("config_read", {
  bridgeUrl: made.bridgeUrl,
  paneId: made.paneId,
  protocolVersion: made.protocolVersion,
  appVersion: made.appVersion,
  pluginPackage: String(madeEntry.package || ""),
  worktreeA,
  worktreeB,
});

function session(id, title, active, directory) {
  return {
    sessionID: id,
    title,
    active,
    busy: false,
    attention: false,
    unread: false,
    promptPulse: 0,
    renaming: false,
    directory,
    subpath: null,
    projectID: "e2e-project",
  };
}

function snapshot(revision, active) {
  return {
    type: "state",
    revision,
    activeSessionID: active === "A" ? sessionA : sessionB,
    tabsEnabled: true,
    sessions: [
      session(sessionA, titleA, active === "A", worktreeA),
      session(sessionB, titleB, active === "B", worktreeB),
    ],
  };
}

let revision = 0;
let active = "A";
let socket = null;
let welcomeSeen = false;
let finished = false;

function send(payload, options = {}) {
  if (!socket || socket.readyState !== WebSocket.OPEN) {
    log("send_skipped", { type: payload && payload.type, readyState: socket && socket.readyState });
    return false;
  }
  const text = JSON.stringify(payload);
  if (!options.allowToken && text.includes(String(made.token))) {
    fail("refusing to send a payload containing the bridge token");
  }
  socket.send(text);
  return true;
}

function sendState() {
  revision += 1;
  const payload = snapshot(revision, active);
  send(payload);
  log("state_sent", {
    revision,
    activeSessionID: payload.activeSessionID,
    tabsEnabled: payload.tabsEnabled,
    sessions: payload.sessions.length,
    activeDirectory: active === "A" ? worktreeA : worktreeB,
  });
}

function connect(attempt = 1) {
  const ws = new WebSocket(String(made.bridgeUrl));
  socket = ws;

  ws.addEventListener("open", () => {
    log("socket_open", { attempt });
    send({
      type: "hello",
      protocolVersion: 1,
      token: String(made.token),
      paneId: String(made.paneId),
      pluginVersion: "e2e-fake-0.1.0",
      opencodeVersion: "2.1.4",
      pid: process.pid,
      cwd: process.cwd(),
    }, { allowToken: true });
    log("hello_sent", {
      pluginVersion: "e2e-fake-0.1.0",
      opencodeVersion: "2.1.4",
      paneId: made.paneId,
      pid: process.pid,
      node: nodePath,
    });
  });

  ws.addEventListener("message", (event) => {
    let message;
    try {
      message = JSON.parse(String(event.data));
    } catch (error) {
      log("message_parse_error", { message: error.message });
      return;
    }
    log("bridge_message", { type: message.type });
    if (message.type === "welcome") {
      welcomeSeen = true;
      log("welcome_received", {
        protocolVersion: message.protocolVersion,
        appVersion: message.appVersion,
        heartbeatIntervalMs: message.heartbeatIntervalMs,
      });
      sendState();
      setTimeout(() => {
        active = "B";
        sendState();
      }, switchDelayMs);
    } else if (message.type === "ping") {
      send({ type: "pong", nonce: String(message.nonce) });
    } else if (message.type === "request_state") {
      sendState();
    } else if (message.type === "focus") {
      const requested = String(message.sessionID);
      if (requested === sessionA) active = "A";
      if (requested === sessionB) active = "B";
      send({ type: "focus_result", requestID: String(message.requestID), ok: true });
      sendState();
    }
  });

  ws.addEventListener("close", (event) => {
    log("socket_close", { code: event.code, reason: event.reason, finished });
    if (!finished && attempt < 20) {
      setTimeout(() => connect(attempt + 1), 100);
    }
  });

  ws.addEventListener("error", () => {
    log("socket_error", { attempt });
  });
}

process.on("SIGTERM", () => {
  finished = true;
  log("sigterm");
  try {
    socket && socket.close(1000, "test harness done");
  } catch {}
  process.exit(0);
});
process.on("SIGHUP", () => {
  finished = true;
  log("sighup");
  process.exit(0);
});
process.on("SIGINT", () => {
  finished = true;
  log("sigint");
  process.exit(0);
});

setTimeout(() => {
  if (!welcomeSeen) {
    fail("bridge did not send welcome within 10s");
  }
}, 10000);

log("start", { node: nodePath, cwd: process.cwd() });
connect();
