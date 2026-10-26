/**
 * OpenCode v2 TUI entrypoint for Made.
 *
 * OpenCode supplies `@opencode/plugin/tui` and `solid-js` at runtime and loads
 * this file from the app-owned plugin package directory. The testable bridge
 * implementation lives in src/integration.ts and has no OpenCode imports.
 */

import { Plugin } from "@opencode/plugin/tui";
import { createEffect } from "solid-js";
import { createIntegration, type TuiPluginContext } from "./src/integration.ts";

export default Plugin.define({
  id: "made.opencode.bridge",
  setup(context) {
    return createIntegration(context as TuiPluginContext, createEffect);
  },
});
