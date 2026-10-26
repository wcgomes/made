/**
 * Ambient declarations for modules that OpenCode supplies at runtime.
 *
 * These are intentionally minimal: the integration core accepts a structural
 * context, and tests use their own fakes. The declarations exist so the
 * package can be typechecked without installing OpenCode's runtime packages.
 */

declare module "@opencode/plugin/tui" {
  export interface PluginDefinition {
    id: string;
    setup(context: unknown): void | (() => void);
  }

  export const Plugin: {
    define<T extends PluginDefinition>(definition: T): T;
  };
}

declare module "@opencode/plugin" {
  export interface PluginDefinition {
    id: string;
    setup(context?: unknown): void | Promise<void>;
  }

  export const Plugin: {
    define<T extends PluginDefinition>(definition: T): T;
  };
}

declare module "solid-js" {
  export function createEffect(fn: () => void): void;
}
