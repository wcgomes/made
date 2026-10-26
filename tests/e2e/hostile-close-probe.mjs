// Test-only unauthenticated bridge client used to feed a hostile WebSocket
// close reason into Made. It never sends session state and never receives a
// token; the app must close it before authentication and sanitize any reason
// it logs.
//
// Usage: node hostile-close-probe.mjs <port>
// Exit 0 after the socket closes.

const port = process.argv[2];
if (!port) {
  console.error("usage: hostile-close-probe.mjs <port>");
  process.exit(2);
}

const hostile =
  "unauth-\u001b]0;owned\u0007\nline\u202ereversed\u200bzero";
const url = `ws://127.0.0.1:${port}/made`;
const ws = new WebSocket(url);
const timeout = setTimeout(() => {
  console.error("probe: timeout waiting for close");
  process.exit(3);
}, 5000);

ws.addEventListener("open", () => {
  console.log("probe: connected");
  try {
    ws.close(1000, hostile);
    console.log("probe: sent hostile close reason");
  } catch (error) {
    console.error(`probe: close rejected: ${error.message}`);
    process.exit(4);
  }
});
ws.addEventListener("close", (event) => {
  clearTimeout(timeout);
  console.log(`probe: closed code=${event.code} reason=${JSON.stringify(event.reason)}`);
  process.exit(0);
});
ws.addEventListener("error", () => {
  // The expected path is open then close; an error may race the close.
  console.error("probe: socket error");
});
