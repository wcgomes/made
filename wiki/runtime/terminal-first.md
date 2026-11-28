# Terminal-First Runtime

A Made workspace starts as a real shell and explorer in a folder. OpenCode is optional and starts only when the user runs `opencode` in that terminal or explicitly launches it from the Open workspace screen. This page records the runtime behavior implemented in [`app.rs`](../../crates/made-app/src/app.rs), [`workbench.rs`](../../crates/made-app/src/workbench.rs), [`session_follow.rs`](../../crates/made-app/src/session_follow.rs), [`integrations/`](../../crates/made-app/src/integrations/mod.rs), and [`made-platform`](../../crates/made-platform/src/lib.rs). Normative behavior is owned by [`specs/opencode-session-follow.md`](../../specs/opencode-session-follow.md) with the historical v1 design at [`design.md`](../../specs/changes/archive/20261029-opencode-session-follow-v1/design.md) for rationale.

## Launch and default workspace

| Launch input | Result |
|---|---|
| `--workspace <path>` or positional folder | Explicit folder wins over `--picker`; opens a normal workspace. |
| `--picker` | Open workspace screen only; no shell or explorer until a launch button is activated. |
| No workspace argument | Platform-resolved home opens as a normal workspace. |
| Missing/unreadable/unresolvable home | Visible non-blocking error, terminal-only fallback in a usable directory, no blocking screen, no crash. |
| `--smoke-test` without a folder | Test-only Open workspace surface; never starts a real home shell. |

An explicit workspace does not require OpenCode installation, detection, a version wait, plugin readiness, a probe, or automatic launch. The [Open workspace screen](../ui/layout-and-controls.md#open-workspace-screen) is also the add-workspace (`+`) surface and the state after the final tab closes.

Launch buttons follow the integrated-TUI catalog: Terminal first, then detected programs in the order of the static `PRODUCT` list in [`integrations/mod.rs`](../../crates/made-app/src/integrations/mod.rs) (OpenCode, Grok, Claude Code); root precedence is separate (see Root precedence). A TUI starts from its absolute path, never the bare name, as the PTY child of a new tab in the chosen folder or worktree (the effective launch path): no shell first, and the current tab keeps running. OpenCode keeps its launch injection; Grok gets no plugin config or Grok config write; Claude Code gets no arguments and no config edit, and its session-only plugin is composed into the environment (see [Claude Code](../integration/claude-code.md)). Launch contracts: `Open Workspace Launch Bar` in [`made-ui-ux.md`](../../specs/made-ui-ux.md) and AIA-07 in [`agent-integration-architecture.md`](../../specs/agent-integration-architecture.md). Detection is a metadata-only `PATH` lookup on a background thread started at launch; it never spawns the program, never verifies its version, and never delays shell or explorer startup. On macOS a Finder/Dock launch first corrects `PATH` (see macOS GUI launch PATH).

The home/`--picker`/explicit-path policy is an accepted product decision; see [ADR 0005](../decisions/0005-home-default-startup.md). Implementation lives in [`workspace.rs`](../../crates/made-app/src/workspace.rs) and [`cli.rs`](../../crates/made-app/src/cli.rs).

## macOS GUI launch PATH

Status: shipped in v0.1.2 (PR #20). v0.1.1 lacks it: a Finder/Dock launch shows no OpenCode, Grok or Claude Code in the catalog, and Claude Code explorer follow (`/cd`, agent view) silently does nothing because the bare-name `claude` lookup fails. That was the root cause of follow not working in Finder/Dock-launched v0.1.1 builds (a terminal launch had a full `PATH`); check `PATH` first when a Finder-launched build misses a CLI that works from a terminal.

- A Finder/Dock launch inherits launchd's `PATH` `/usr/bin:/bin:/usr/sbin:/sbin`, which hides user-installed CLIs (`~/.local/bin`, `~/.opencode/bin`, `~/.grok/bin`, Homebrew).
- First thing in `run()`, `made_platform::adopt_login_shell_path` acts only when `PATH` is unset, empty or exactly that launchd default; terminal launches (at least `path_helper`'s `PATH`), Linux and Windows are untouched.
- It runs `$SHELL -ilc` (absolute executable `SHELL`, else `/bin/zsh`) with stdin null, in its own process group, and reads `PATH` between per-call sentinel markers. It is interactive because many users export `PATH` only in `~/.zshrc`. The probe sets `MADE_RESOLVING_SHELL_ENV=1`, so rc files can skip heavy work and a nested Made skips the probe.
- After ~3 s the shell's process group is killed; on timeout or unparseable output the inherited `PATH` is the base. Missing fallback dirs are then appended: `$HOME/.local/bin`, `.opencode/bin`, `.grok/bin`, `.cargo/bin`, `/opt/homebrew/bin`, `/usr/local/bin`.
- The process `PATH` is set once, before any thread starts (it mutates the environment). Detection stays a plain `PATH` lookup and PTY children inherit the corrected `PATH`.
- Accepted cost: a Finder/Dock launch waits for the user's interactive rc files, bounded by ~3 s. Manual reproduction: [Build and Test](../operations/build-and-test.md#test-a-release-build-locally). Source: [`macos.rs`](../../crates/made-platform/src/macos.rs).

## Shell cwd reporting

`made-platform::shell_integration` prepares a launch-local plan for supported shells. It sources the user's real startup files and appends an app-owned OSC 7 emitter without writing the user's files. The launch plan can carry environment and arguments for paths; it does not interpolate commands into the running shell.

| Shell | Current capability |
|---|---|
| bash | Verified with a real installed shell, including spaces/Unicode, profile preservation, and unchanged rc hash. |
| zsh | Source/unit coverage; real installed-shell run pending. |
| fish | Source/unit coverage; real installed-shell run pending. |
| PowerShell/pwsh | Source/unit coverage; real installed-shell run pending. |
| `cmd` (Windows only) | Launch-local OSC 7 through a `PROMPT` environment variable (`$E]7;file://localhost/$P$E\` plus the user's `PROMPT`, or `$P$G`); no files written, and `/d` is the only extra argument allowed. `made-terminal` accepts the raw cmd `$P` path form (explicit `localhost`, `X:\`) and then applies the existing local-path checks. Native Windows run pending; a cmd AutoRun that resets `PROMPT` defeats it. |
| Unknown / restricted launch | Explicit unsupported capability and reason; shell remains usable at the latest known cwd or launch folder. |

`made-terminal` validates OSC 7 as a bounded local absolute path, decodes URI escapes/Unicode, and rejects remote, relative, malformed, or oversized data. Cwd is never derived from terminal titles, prompts, or injected discovery commands. The latest validated shell cwd is retained independently of OpenCode.

## SSH remote runtime

Contracts live in [`specs/ssh-remote-workspaces.md`](../../specs/ssh-remote-workspaces.md); this section records implementation facts that are not visible from one file.

- Selecting an SSH host on the Open workspace screen opens a screen-scoped browse grant (SSH-12), separate from every tab's root grant; trust boundary in [Security](../security.md#remote-browse-grant). [`remote_workspace.rs`](../../crates/made-app/src/remote_workspace.rs) must never touch the host filesystem: `tests/remote_workspace.rs` fails on `PathBuf::from(`, `std::fs`, `.canonicalize(` or `GitRunner` in that file, so remote path keys go through `open_workspace::remote_key`.

- Remote shell cwd arrives as companion `ShellCwd` frames, not OSC 7. The workbench `pump_remote_client` routes them before the `waiting_attach` match, and the supervisor re-emits the cwd on reattach.
- `tick_remote` returns before the local idle repaint cadence, so any new remote event source needs an explicit wake through `RemoteClient::set_wake`.
- Remote PTY locale: the supervisor ensures a UTF-8 `LC_CTYPE` chosen from the server's `locale -a`; it never invents one and leaves `LC_ALL` untouched. It prefers `C.UTF-8` and checks an existing UTF-8 locale against `locale -a`.
- Local PTY locale: `with_utf8_ctype` in [`workbench.rs`](../../crates/made-app/src/workbench.rs) gives the main spawn and the fallback-cwd retry a UTF-8 `LC_CTYPE` (not on Windows). A non-empty `LC_ALL` is left alone even when not UTF-8, since it overrides `LC_CTYPE`. Otherwise the first non-empty of `LC_CTYPE`/`LANG` is trusted by name if it says utf-8/utf8; else `LC_CTYPE` comes from the cached host `locale -a`, preferring `en_US.UTF-8` (macOS `ssh_config` `SendEnv LC_*` forwards it, and older servers lack `C.UTF-8`), then `C.UTF-8`, then the first UTF-8 entry. Nothing is invented when none is installed. Why: macOS `/bin/bash` 3.2 readline drops 8-bit input (`é`) without a UTF-8 locale, and Finder-launched apps get no `LANG` from launchd.
- Local and remote differ on purpose: macOS `locale -a` lists `C.UTF-8` and `en_US.UTF-8` but not plain `UTF-8`, which libc accepts and Terminal.app sets, so the local check trusts the name. Shared code is [`locale.rs`](../../crates/made-platform/src/locale.rs); `made-remote` re-exports `ensure_utf8_locale`/`installed_locales` from it.
- Reattach: the resnapshot carries SGR attributes, and a rows-1 resize nudge forces full-screen children to redraw.
- These fixes are Linux-tested only; live-SSH validation is pending (see the [platform matrix](../testing/platform-matrix.md)).

## Root precedence

Precedence is each integration's declared rank (OpenCode 100, Claude Code 200, Grok 300; lower wins), never the position in `PRODUCT`. The OpenCode session channel owns the lowest rank; Claude Code and Grok report directories ranked against each other by a declared follow policy, with differences (unusable directory, behavior while outranked) kept in each integration module. See [Adding an integration](../integration/adding-an-integration.md) and [ADR 0009](../decisions/0009-static-integration-modules.md).

1. A connected OpenCode active session directory wins while an active session exists; an unavailable active directory is shown as unavailable, not replaced by the view directory or shell cwd.
2. Otherwise, a connected OpenCode view directory selected in the Home/new-session view wins while tabs are enabled, no session is active, and the directory is usable.
3. Otherwise, while OpenCode is not connected, the directory of this tab's most recent live owned Claude Code session (or its agent-view selection) wins, local or remote ([Claude Code SSH](../integration/claude-code.md#ssh)).
4. Otherwise, the directory of this tab's newest open Grok session wins, local or remote; a prompt in an older Grok session keeps the explorer there.
5. Otherwise, the latest validated shell cwd wins.
6. Otherwise, the launch folder (from the CLI or the Open workspace screen) or platform-resolved home.
7. A stale OpenCode root is never retained ahead of a newer shell cwd.

While OpenCode is authoritative, shell cwd reports continue to be recorded. A connected view directory applies only while tabs are enabled and no session is active; an absent view directory leaves the latest shell cwd or launch folder in effect, while an invalid, missing, or unreadable one falls back to the latest shell cwd, then the launch folder, and is reported in Diagnostics. The view directory is cleared when OpenCode disconnects or exits, when integration is unsupported, when session tabs become unavailable, or on a new connection epoch. On OpenCode disconnect or normal exit, the root returns to this tab's live Claude Code session, then its newest open Grok session, if one exists, otherwise the latest shell cwd; if no shell cwd exists, it returns to the launch folder. Shell-owned roots that are deleted or become unreadable are revalidated by a low-frequency background worker, marked unavailable once, and recover when the path returns. Agent-owned roots are unaffected by bad shell reports.

The root model distinguishes effective path, display name, source, git/worktree metadata, and availability. Root changes increment a generation, clear open preview, and invalidate stale tree/preview results. See [`session_follow.rs`](../../crates/made-app/src/session_follow.rs) and [`root_switch.rs`](../../crates/made-explorer/src/root_switch.rs).

## Workspace tabs and process lifecycle

`AppModel` holds an ordered list of live workspaces and one active id. Each `Workbench` owns its own:

- PTY/process session and shell bootstrap;
- terminal state and bounded output/scrollback;
- explorer root, generation, tree, watcher, and preview;
- bridge listener, token, pane id, and optional OpenCode connection;
- session map, focus state, explorer width/visibility, and left-surface mode.

Switching tabs marks the previous workbench invisible and the target visible, resets terminal focus/composition state, and never stops either process. Hidden workspaces continue bounded polling/output consumption and bridge liveness, but they do not paint grids. Preview input is isolated from its hidden terminal.

Closing a tab terminates/reaps only that workbench's process group, stops its listener/watchers, cancels pending loads, and releases its token/bootstrap state. Late callbacks are rejected by workspace id and generation. Closing the last tab shows the Open workspace screen and starts no shell or TUI. Window close shuts down all workbenches. There is no tab persistence across restart.

Process cleanup lives in `made-platform`:

- Unix captures the shell's process group, re-queries `tcgetpgrp` at cleanup to catch a foreground job after shell exit, skips PID 1/Made's group, and sends SIGHUP then SIGKILL.
- Windows workspace-scoped Job Object cleanup is implemented and cross-compiled; native runtime evidence is still pending. Immediate-child-only behavior is not claimed as full descendant cleanup.

## Clipboard paste

Terminals have no standard inbound image-clipboard channel, so Made cannot hand raw image bytes to the child process. `Cmd+V`/`Ctrl+V` with an image on the clipboard forwards `Ctrl+V` (`0x16`) so the child TUI — OpenCode, Claude Code, and similar agent TUIs — reads the image natively; text-only clipboards keep bracketed paste. `Cmd+Shift+V`/`Ctrl+Shift+V` saves the image as a PNG under `AppPaths::temp_dir/clipboard-images/` and pastes the absolute path, which agent TUIs that attach local image paths can consume. When both text and an image are present, the image wins. Native image forwarding shows no hint; in a remote tab the first successful image upload shows the one-time notice "Image uploaded to the server; its file path was pasted." Images are capped at 128 MiB RGBA and 16384 px per side. App-generated clipboard PNGs are pruned after 7 days and capped at 64 files; cleanup runs after a successful save and when a workbench starts, and the just-saved image survives. macOS is user-confirmed (2026-11-01); Linux X11/Wayland and Windows have not yet been tested, with no known defect. The contract is the Clipboard image paste and Clipboard image retention requirements in [OpenCode Session Follow](../../specs/opencode-session-follow.md).

## Clipboard copy (OSC 52)

Programs in local and SSH panes copy to the OS clipboard with OSC 52; the contract is "OSC 52 clipboard copy" in [OpenCode Session Follow](../../specs/opencode-session-follow.md) and SSH-05 in [`ssh-remote-workspaces.md`](../../specs/ssh-remote-workspaces.md).

- Path: alacritty `Event::ClipboardStore` → `TerminalCore::take_clipboard_store` ([`emulation.rs`](../../crates/made-terminal/src/emulation.rs)) → `TerminalPane::apply_clipboard_store` ([`terminal_view.rs`](../../crates/made-app/src/terminal_view.rs)), called each tick after the output pump.
- Queries are refused through `Osc52::OnlyCopy`; the remote supervisor discards stores, so only the client applies them. Selector, size (`PASTE_BYTE_LIMIT`) and latest-wins rules are in the spec.
- Residual: vte buffers a raw OSC sequence without bound before dispatch, so the 1 MiB limit applies to the decoded store, not to buffering.

## Replacing a TerminalCore

Anything that swaps in a new `TerminalCore` (`apply_resnapshot`, relaunch) drops state buffered in the old core's event listener: title, replies, bell, and the OSC 52 store. Take what must survive before the swap and carry it on the pane, as `TerminalPane.carried_clipboard_store` does; a store fed after the swap wins over the carried one.

## URL open requests

TUIs that run `$BROWSER <url>` get an ask-before-open card instead of a browser. Contracts: "URL Open Confirmation Card" in [`made-ui-ux.md`](../../specs/made-ui-ux.md), AIA-07 in [`agent-integration-architecture.md`](../../specs/agent-integration-architecture.md), and SSH-05/SSH-08/SSH-11 in [`ssh-remote-workspaces.md`](../../specs/ssh-remote-workspaces.md). Rationale: [ADR 0011](../decisions/0011-browser-helper-url-socket.md).

- Local: a core launch contribution (applied after every integration's, so it wins) sets `BROWSER` to the `made-open` helper beside the executable, plus `MADE_URL_SOCKET` and a per-launch `MADE_URL_TOKEN`. The helper ([`made-open.rs`](../../crates/made-platform/src/bin/made-open.rs)) validates the URL and sends `{token,url}` over a same-user Unix socket or named pipe; it exits 0 only on ack, otherwise the TUI prints the URL itself. App side: [`url_open.rs`](../../crates/made-app/src/url_open.rs).
- Remote: the supervisor symlinks `<runtime>/made-open` to `made-remote`, which runs in helper mode by argv0, and forwards `Operation::OpenUrlRequest` only to the attached client. Ack means queued, not shown; a request while detached is dropped and never replayed on reattach or resnapshot.
- Card: a dialog centered over the requesting pane's terminal body with a scrim; the grid is not resized, the whole pane body is excluded from terminal pointer input while it is up, and it never takes keyboard input, so keys keep reaching the TUI. Host whole on its own line, full URL scrollable, arming delays and the narrow-pane rule are in the spec. The host opener strips `MADE_URL_*` and restores `MADE_ORIGINAL_BROWSER`.
- Coverage: Claude Code and Grok honor `BROWSER`; OpenCode does not (it calls `open`/`xdg-open`), and a `PATH` shim was rejected. A `BROWSER` set in `~/.bashrc`/`~/.zshrc` overrides Made's, so no card appears. URLs are never logged.

## Bridge isolation

Each tab owns a separate loopback bridge and credential set. Events carry the owning endpoint and connection epoch. A snapshot, focus result, disconnect, or goodbye from another workspace is ignored; a cross-workspace hello is rejected before replacement or state disclosure. A stale epoch cannot overwrite a healthy workspace. See [Security](../security.md) and [OpenCode bridge](../integration/opencode.md).

## Optional OpenCode behavior

- Normal shell use is `inactive`, not degraded; detection is a background metadata lookup only, with no version wait, probe, or automatic launch.
- A valid plugin hello and snapshot move the tab to `connected`.
- A lost connection, heartbeat timeout, or plugin error moves it to `degraded` and resumes shell cwd.
- A protocol or OpenCode major mismatch is `unsupported`; the terminal remains usable.
- Sessions are observed only. Made does not create/remove/merge/prune worktrees or replace the OpenCode TUI.
- The optional session map is hidden by default and scoped to the selected workspace.

## Practical implications

- Do not make shell startup depend on OpenCode readiness or detection.
- Keep shell cwd valid while OpenCode is authoritative.
- Do not retain an agent root after disconnect.
- Do not share bridge state, tokens, preview state, or focus across tabs.
- When adding platform behavior, keep OS code in `made-platform` and preserve the per-tab shutdown boundary.

Related: [Architecture](../architecture.md), [Testing strategy](../testing/strategy.md), [ADR 0002](../decisions/0002-terminal-first-workspace-tabs.md).
