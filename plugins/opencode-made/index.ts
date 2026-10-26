/**
 * Server-side package entrypoint.
 *
 * Made only implements the CLI/TUI side, but OpenCode's directory package
 * layout keeps a server entry beside `./tui`. This no-op preserves that shape
 * without adding server behavior or filesystem access.
 */

import { Plugin } from "@opencode/plugin";

export default Plugin.define({
  id: "made.opencode.server",
  setup() {},
});
