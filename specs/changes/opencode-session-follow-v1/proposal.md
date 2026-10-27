# Proposal: OpenCode Session Follow v1

## Intent

Made is a native, terminal-first workspace: open a folder and immediately use a shell and following explorer, without needing OpenCode. Independent workspace tabs retain their live terminals; opening a file selects a read-only preview state, never stopping its process. OpenCode v2 is an optional enhancement that follows the active session and worktree when connected. V1 is still an undelivered UX validation slice, not a production release. The UI/UX/visual contract has been split into the active `made-ui-ux-v1` change so this change can stay focused on product, runtime, integration, security, and performance behavior.

## Scope

In scope:

- A native cross-platform desktop app for macOS, Linux, and Windows from one source tree.
- A workspace launch flow that opens an explicit folder, opens the current user's home directory by default, and presents the in-app folder picker only when explicitly requested by UI action or `--picker`; picker presentation is owned by `made-ui-ux-v1`.
- Independent workspace tabs, each owning a real shell, explorer, directory-follow state, optional integration, preview state, and focus state; switching retains live processes.
- A terminal that works independently and also runs the real OpenCode v2 TUI with its required terminal features.
- OpenCode v2 integration through an officially supported extension mechanism and a local-only bridge.
- Shell-directory follow through safe per-launch integration; a connected active OpenCode session directory takes precedence, including OpenCode-managed worktrees, and disconnect resumes the latest shell cwd.
- Session-map data for open root-session tabs, with focus-from-Made as a SHOULD.
- A per-workspace explorer and read-only preview state, with layout/presentation delegated to `made-ui-ux-v1`.
- Read-only preview guards, diagnostics data and bounded active-plus-previous log retention, security boundaries, and measured performance/fluidity targets for the complete workflow.
- CI builds and development artifacts for every supported target.

## Non-goals

- File editing, saving, dirty state, LSP, or a diff editor.
- Claude Code, Codex, Grok, other agents, ACP, or any integration other than OpenCode v2.
- Creating, removing, merging, or orchestrating worktrees; parallel agent management.
- Remote, SSH, or WSL-hosted OpenCode.
- Multi-window, terminal splits, plugin marketplace, auto-update, production installers, signing, or notarization. Workspace tabs are in scope; a second terminal-tab hierarchy is not.
- Workspace/tab restoration across app restart, automatic OpenCode launch, persistent shell or OpenCode configuration writes, prompt/title scraping, or injecting commands into a running shell to discover cwd.
- Any AI or chat UI that replaces the OpenCode TUI.
- Production release readiness.
- UI/UX/visual ownership: Calm Editor layout, chrome, wordmark, glass/native backdrop, tabs/session-map presentation, picker UX, preview/diagnostics presentation, motion, responsive layout, keyboard focus, and accessibility presentation are in `made-ui-ux-v1`.

## Approach

Made opens a shell and explorer per workspace tab in one native window. With no launch path it opens the current user's home directory; the picker is an explicit action or `--picker`, and an explicit path takes precedence. Safe launch-local shell hooks report cwd without rewriting user configuration; shell capability and fallback are explicit. The user may run OpenCode normally; the app-owned plugin and authenticated tab-local loopback bridge then supply active-session state. Directory arbitration, tab isolation, root generations, read-only preview safety, integration state, diagnostics data with bounded active-plus-previous log retention, and bounded hidden-workspace processing preserve the terminal-first runtime. Normal shell use is not a degraded state; technical integration detail lives in Diagnostics. UI presentation is owned by `made-ui-ux-v1` and its design. Expanded core decisions are in `design.md`.

## Decision index

- **OpenCode v2 only in v1:** limits the integration surface and keeps UX validation focused; no ACP or other agents (design: [D1](design.md#d1-scope-and-integration-boundary)).
- **Official OpenCode extension plus loopback bridge:** observes tabs and session directories reliably, supports focus commands, and avoids screen scraping (design: [D2](design.md#d2-plugin-and-loopback-bridge)).
- **No persistent user-config changes:** integration configuration is composed per launch and injected into the child process; user files stay untouched (design: [D3](design.md#d3-plugin-loading-and-config-merge)).
- **Home-default startup with explicit override and picker:** no-arg startup opens the current user's home directory instead of blocking on a picker; `--workspace`/positional wins, while `--picker` and the Open folder action make the picker explicit (design: [D5](design.md#d5-session-and-workspace-model), [D19](design.md#d19-home-default-launch-and-picker-boundary)).
- **Directory precedence:** connected integration with an active session directory wins; otherwise latest known shell cwd, then launch folder. Disconnect never pins the old agent root (design: [D5](design.md#d5-session-and-workspace-model)).
- **Safe shell capabilities:** provide launch-local cwd reporting for bash, zsh, fish, and PowerShell; cmd and unsafe/unsupported launches explicitly fall back without claiming automatic follow (design: [D17](design.md#d17-shell-directory-reporting)).
- **Independent workspace tabs:** reuse one workbench per tab with isolated credentials, process, root generation, preview state, and focus; hidden tabs keep processing bounded output/state without painting grids (design: [D18](design.md#d18-workspace-ownership-and-visibility)).
- **Read-only preview safety:** preview state is read-only, keeps the terminal process alive, isolates input, and rejects stale roots; presentation is owned by `made-ui-ux-v1` (design: [D8](design.md#d8-explorer-and-preview)).
- **Explorer reflects live filesystem changes:** creates, deletes, and renames appear without manual refresh; stale root events cannot populate the new tree (design: [D8](design.md#d8-explorer-and-preview)).
- **Worktree lifecycle stays in OpenCode:** Made observes and displays worktrees but never creates, removes, or merges them (design: [D5](design.md#d5-session-and-workspace-model)).
- **Force OpenCode tabs mode and global scope for v1:** required so the plugin can see all open sessions and worktrees; documented as a v1 constraint (design: [D3](design.md#d3-plugin-loading-and-config-merge)).
- **One source tree with a thin platform layer:** macOS arm64/x64, Linux x64 on X11 and Wayland, and Windows x64; different artifacts, not per-OS rewrites (design: [D7](design.md#d7-platform-layer)).
- **Native stack, no webview:** terminal performance and lightness are product requirements; stack details are in design (design: [D6](design.md#d6-terminal-stack)).
- **Optional integration over hard failure:** missing, disabled, incompatible, or disconnected integration never blocks or labels ordinary shell use degraded; reasons remain inspectable in Diagnostics (design: [D9](design.md#d9-degraded-state-machine)).
- **Failures are first-class:** integration status, active session and root, integration version, and logs are inspectable without a debugger (design: [D13](design.md#d13-diagnostics-and-observability)).
- **UI contract has a separate active owner:** layout/chrome/presentation/motion/accessibility are specified in `made-ui-ux-v1`, not duplicated here (design: [Boundary](#architectural-boundaries)).
- **UX validation, not production:** no signing, notarization, auto-update, or installers in v1 (design: [D1](design.md#d1-scope-and-integration-boundary)).
- **No delivery calendar:** tasks are dependency-ordered and test-first; no dates or durations are included.

## Clarifications

- **Q:** Which language should the spec bundle use?
  **A:** English for all artifact bundles.
- **Q:** Which agent integrations are in scope for v1?
  **A:** OpenCode v2 only. Claude Code, Codex, Grok, ACP, remote agents, and other TUIs are excluded.
- **Q:** Is file editing in v1?
  **A:** No. A file opens read-only; editing, saving, dirty state, LSP, and diff views are excluded.
- **Q:** Which platforms are required?
  **A:** macOS arm64/x64, Linux x64 on X11 and Wayland, and Windows x64 using the native OpenCode v2 distribution. WSL is excluded.
- **Q:** Does Made manage worktrees?
  **A:** No. OpenCode owns worktree creation, removal, and lifecycle; Made observes and follows them.
- **Q:** How is OpenCode integrated?
  **A:** Through its official CLI/TUI plugin mechanism and a Made-owned loopback bridge, not ACP, not screen scraping, and not terminal-title scraping.
- **Q:** Is a separate OpenCode server required?
  **A:** No. Launching `opencode` directly in the Made terminal is sufficient for v1.
- **Q:** What happens when integration is unavailable?
  **A:** Made keeps the shell usable and follows the latest known shell cwd (launch folder only until cwd is known); integration failure reasons are available in Diagnostics without treating shell use as degraded.
- **Q:** Who implements v1?
  **A:** AI agents execute the dependency-ordered tasks; no staffing assumptions are made.
- **Q:** Are delivery dates or effort forecasts part of this bundle?
  **A:** No. Tasks are dependency-ordered and test-first; no dates or durations are included.
- **Q:** What is the primary success criterion?
  **A:** Validate an extremely responsive, beautiful and fluid terminal-first workspace, including real shell cd follow, independent tabs and preview roundtrips, with optional real OpenCode v2 session follow; production readiness is not required.
- **Q:** Is this a separate version or change?
  **A:** No. The UI contract was split into the active `made-ui-ux-v1` change; this remains the active, undelivered v1 core. Nothing is archived and no version is created.
- **Q:** Which directory wins while OpenCode is active, and after disconnect?
  **A:** The live active OpenCode session directory wins while integration is connected; otherwise use the latest known shell cwd, then the launch folder. Continue recording shell cwd while OpenCode wins.
- **Q:** What happens when Made starts with no workspace folder?
  **A:** Made opens the current user's home directory directly with a terminal and explorer. The picker is not a blocking startup step; it remains available from the Open folder action and `--picker`. An explicit `--workspace`/positional path takes precedence over `--picker`.
- **Q:** What happens when the default home directory is missing or unreadable?
  **A:** Made shows a visible workspace error and keeps a usable terminal in a fallback directory; it does not block on the picker or crash.
