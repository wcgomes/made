# Tasks: grok-session-follow

Readiness: `analyze` must pass before implementation; review runs `verify` against the persisted spec.

Contract ref: specs/changes/grok-session-follow/

The proof of concept already lives in `plugins/grok-made`, `crates/made-app/src/grok_follow.rs`, and the workbench and workspace-tab wiring. These tasks lock that behavior with tests and close only the gaps the scenarios name. Do not rediscover the design. Menu tasks add the catalog-driven `+` rows. They do not reopen session follow. Install tasks ensure the shipped plugin after PATH detection reports grok. They do not change the hook status line or the Grok menu launch. Diagnostics tasks add one Grok Build row to the revealed About diagnostics. They do not change the Grok Build tab label or the menu row. Section 7 replaces only the ready rule for that row. Section 8 changes only the OpenCode visible pill label and the named reason sentences.

## 1. Lock the hook contract
- [x] 1.1 Write a failing test that one JSON object on stdin, with stdin left open, appends `sessionId`, `cwd`, `event`, and `status`, and exits 0 (path: plugins/grok-made/hooks/bin/status.py)
- [x] 1.2 Confirm the test fails for a missing `ask_user_question` mapping, then keep PreToolUse attention and PostToolUse working (path: plugins/grok-made/hooks/hooks.json)

## Checkpoint
- [x] verify Question and permission show attention
- [x] verify Turn end and session end

## 2. Lock inventory and hook follow
- [x] 2.1 Write a failing test that only rows whose pid is the terminal process or its descendant count, and that the newest `opened_at` is the newly attached directory (path: crates/made-app/src/grok_follow.rs)
- [x] 2.2 Write a failing test that a prompt hook for an older open session is not replaced by a later inventory refresh of a newer attachment (path: crates/made-app/src/session_follow.rs)
- [x] 2.3 Write a failing test that lines already in the status log when Made starts are ignored, and later lines apply (path: crates/made-app/src/grok_follow.rs)

## Checkpoint
- [x] verify Newly attached session moves the explorer
- [x] verify Inventory does not steal the explorer back
- [x] verify Session start and prompt move the explorer immediately
- [x] verify Grok exit restores the shell
- [x] verify Focus alone does not move the explorer
- [x] verify OpenCode remains first

## 3. Lock the tab dots
- [x] 3.1 Write a failing UI test that a non-OpenCode tab with working, attention, and idle Grok sessions paints those three dots, the Grok Build label, and the official grok.svg mark, and returns to the folder label and terminal glyph when Grok is not open (path: crates/made-app/src/ui.rs)
- [x] 3.2 Confirm an OpenCode-connected tab still paints OpenCode dots and ignores Grok events (path: crates/made-app/src/workbench.rs)

## Checkpoint
- [x] verify Dots match session state
- [x] verify Parallel sessions stay distinct

## 4. Offer every integrated TUI from the catalog menu
- [x] 4.1 Write a failing test that the integrated-TUI catalog is OpenCode then Grok and nothing else, detection returns only programs on PATH, a PATH with only `grok` returns only Grok, empty and relative PATH segments are ignored, and the lookup does not spawn a program or read a version [P] (path: crates/made-platform/tests/contract.rs)
- [x] 4.2 Write a failing test that a menu-launched Grok child is the absolute `grok` path plus the argument `dashboard` in the home directory, receives no plugin configuration, and does not write the user's Grok config, while a menu-launched OpenCode child still receives its existing composed plugin configuration and no arguments [P] (path: crates/made-app/tests/workspace_tabs.rs)
- [x] 4.3 Write failing UI tests that no detected TUI opens the folder picker and shows no menu; OpenCode and Grok show Terminal, then OpenCode, then Grok; only Grok shows Terminal, then Grok; the `+` tooltip and accessible name stay Open folder; Terminal still opens the folder picker; choosing Grok starts a home-directory tab with no picker and leaves the current tab running; startup does not wait on the existing background detection; detection alone does not start Grok, so a shell-typed grok is not replaced [P] (path: crates/made-app/tests/ui_model.rs)
- [x] 4.4 Write a failing UI test that the Grok row is labeled Grok Build, its 16px leading slot paints the official mark from grok.svg, the terminal-window glyph is not that mark, and the OpenCode row still paints the vendored 16px opencode.svg (path: crates/made-app/tests/ui_model.rs)
- [x] 4.5 Write a failing test that the offered integrations are OpenCode v2 and Grok Build only, Claude Code, Codex, and ACP are not offered, the Grok integration is not a forbidden mention, and there is no chat UI, no worktree management, and no separate server [P] (path: crates/made-app/tests/scope_boundary.rs)
- [x] 4.6 Confirm 4.1 fails, then add Grok to the catalog as program `grok` and label `Grok Build`. Keep the lookup metadata-only, with no version check and no second detection path (path: crates/made-platform/src/lib.rs)
- [x] 4.7 Confirm 4.2 fails, then keep OpenCode and shell launch injection unchanged and do not inject plugin configuration or write Grok config for a Grok child (path: crates/made-app/src/workbench.rs)
- [x] 4.8 Confirm 4.3 and 4.4 fail, then render the `+` menu from catalog order with Terminal first, keep the OpenCode image unchanged, and paint the Grok row with the official mark from grok.svg. Do not change OpenCode follow or dots (path: crates/made-app/src/ui.rs)
- [x] 4.9 Confirm 4.5 fails, then offer OpenCode v2 and Grok Build and no other agent integration (path: crates/made-app/src/integration_status.rs)

## Checkpoint
- [x] verify No detected TUI opens the folder picker
- [x] verify Catalog menu lists detected TUIs in order
- [x] verify Only Grok is installed
- [x] verify Grok row uses the official mark
- [x] verify OpenCode row keeps its icon and launch
- [x] verify Choosing Grok starts grok at home with the argument `dashboard`
- [x] verify Terminal row still opens the folder picker
- [x] verify Detection does not launch or block startup
- [x] verify Menu does not replace a shell launch
- [x] verify Unsupported agent not offered
- [x] verify No replacement chat UI
- [x] verify Worktree lifecycle remains outside Made
- [x] verify Direct launch

## 5. Install the shipped plugin after detection
- [x] 5.1 Write a failing test that after PATH detection reports grok, a missing grok-made install and an installed `plugin.json` version lower than the shipped dotted numeric version each run `grok plugin install` with the absolute shipped `plugins/grok-made` path and `--trust` through the detected grok, an equal version skips that command, a higher installed version is left in place, shell and explorer startup and Grok startup do not wait, Made does not hand-edit `config.toml`, and Made does not pass OpenCode's plugin environment to Grok [P] (path: crates/made-app/tests/grok_plugin_install.rs)
- [x] 5.2 Write a test that an explicit Grok catalog entry, with PATH detection left off, does not run `grok plugin install` [P] (path: crates/made-app/tests/ui_model.rs)
- [x] 5.3 Write a test that the metadata-only PATH lookup does not spawn grok, does not check a version, and does not install the plugin [P] (path: crates/made-platform/tests/contract.rs)
- [x] 5.4 Write a failing test that a failed `grok plugin install` still lets a menu-launched Grok and a shell-typed grok start, Made records for Grok session follow that session-status hooks are unavailable, Made does not hand-edit `config.toml`, Made does not pass OpenCode's plugin environment to Grok, and a shell-typed grok is not given `dashboard` [P] (path: crates/made-app/tests/workspace_tabs.rs)
- [x] 5.5 Confirm 5.1 and 5.4 fail, keep 5.2 and 5.3 passing, then after PATH detection reports grok run that versioned install on separate background work. Invoke only `grok plugin install <absolute shipped plugins/grok-made path> --trust` with the detected grok. That path is the `grok-made` directory beside the already resolved `opencode-made` package, whether that package is the development repository copy or the install-root copy. Do not install from `detect_compatible_tuis` or `detect_compatible_tuis_in_path`, or from a catalog supplied without detection. On install failure, still allow Grok to start and record for Grok session follow that session-status hooks are unavailable. Do not hand-edit `config.toml`, do not pass OpenCode's plugin environment to Grok, and do not add `dashboard` to a shell-typed grok (path: crates/made-app/src/app.rs)

## Checkpoint
- [x] verify Missing or older plugin is installed
- [x] verify Same version skips install
- [x] verify Newer installed copy stays
- [x] verify Ensuring the plugin does not block startup
- [x] verify Catalog without detection does not install
- [x] verify PATH lookup does not install
- [x] verify Install failure leaves hooks unavailable

## 6. Reveal one Grok Build diagnostics row
- [x] 6.1 Write a failing UI test that revealed About diagnostics show exactly one Grok Build row directly with the existing OpenCode status pill and not in the sectioned detail; the row is hidden until Show diagnostics; its pill and sentence are absent from the header, workspace tabs, explorer, and terminal; the Grok Build tab label and menu row stay unchanged; and the row is inactive `Checking for Grok Build.` while PATH detection is still running and is not shown as not on PATH, inactive `Grok Build is not on PATH.` when detection has finished without grok or an explicit catalog omits Grok and detection is not running, inactive `Installing the Grok Build plugin.` while grok was detected and the shipped-plugin install is still running, degraded `Grok Build session-status hooks are unavailable.` when the install failed, and inactive `Grok Build plugin was not installed for this launch.` when an explicit catalog includes Grok and PATH detection did not run, without claiming ready or that an install is running. Drive each state deterministically, without spawning grok and without waiting on a real PATH lookup or install. Copying the diagnostics report includes that same Grok sentence (path: crates/made-app/tests/ui_model.rs)
- [x] 6.2 Confirm 6.1 fails, then carry the Grok pill label and sentence on the diagnostics snapshot, include that sentence in the copied diagnostics text (path: crates/made-app/src/diagnostics.rs)
- [x] 6.3 Confirm 6.1 fails, then keep the current Grok pill and sentence on the workspace and attach them when building the diagnostics snapshot (path: crates/made-app/src/workbench.rs)
- [x] 6.4 Confirm 6.1 fails, then set the checking, not-on-PATH, installing, hooks-unavailable, and explicit-catalog pill and sentence from PATH detection and the shipped-plugin install outcome. Product startup still uses the background detection and install and must not block shell, explorer, or Grok startup, and a catalog that did not run detection must not install. Add a test-only control that can hold detection still running, detection finished without grok, install still running, install succeeded, already current, or install failed so the UI test does not need a real grok process (path: crates/made-app/src/app.rs)
- [x] 6.5 Confirm 6.1 fails, then paint exactly one Grok row from that snapshot directly with the OpenCode status pill only after Show diagnostics, using the existing inactive and degraded pill treatments and the success tone for ready, and keep that pill and sentence out of workspace chrome. Do not add a second copy of the sentence (path: crates/made-app/src/ui.rs)

## Checkpoint
- [x] verify Grok diagnostics stay hidden until revealed
- [x] verify Grok diagnostics stay out of workspace chrome
- [x] verify Checking for Grok Build
- [x] verify Grok Build is not on PATH
- [x] verify Installing the Grok Build plugin
- [x] verify Grok row shows hooks unavailable
- [x] verify Grok plugin was not installed for this launch
- [x] verify Copied diagnostics include the Grok sentence

## 7. Ready only while Grok is running in that workspace

Replace only the ready rule. When the shipped plugin install succeeded or the installed version is already current, the row for that workspace is ready only while Grok is running there. When that install is in place and Grok is not running there, the pill is inactive and the sentence is `Grok Build is not running.` Checking, not on PATH, installing, hooks unavailable, and the explicit-catalog sentence stay as section 6 left them. Do not change the Grok Build tab label or the menu row.

- [x] 7.1 Write a failing UI test that, for both a succeeded shipped-plugin install and an already-current installed version, the row is ready in the success tone and the sentence is `Grok Build session follow is ready.` when this workspace's terminal target is Grok and that process is still alive, and also when the target is not a living Grok process but this workspace's shell has at least one open Grok session; OpenCode owning the tab label does not by itself make that ready row inactive; the pill is inactive and the sentence is `Grok Build is not running.` when that install is in place and this workspace has neither a living Grok target nor an open Grok session; Grok running only in another workspace leaves this workspace on that same inactive sentence; checking, not on PATH, installing, hooks unavailable, and the explicit-catalog sentence stay unchanged whether or not this workspace's Grok is running; the row stays on that workspace, hidden until Show diagnostics, and out of the header, workspace tabs, explorer, and terminal; the Grok Build tab label and menu row stay unchanged; and copying the diagnostics report includes the same Grok sentence, including `Grok Build is not running.` Drive each state without spawning grok and without waiting on a real PATH lookup or install. The running condition is a test-controlled per-workspace terminal target or open Grok session, not a live grok process (path: crates/made-app/tests/ui_model.rs)
- [x] 7.2 Confirm 7.1 fails, then add the inactive sentence `Grok Build is not running.` as a fixed Grok row sentence. Do not change `Grok Build session follow is ready.` or the checking, not-on-PATH, installing, hooks-unavailable, or explicit-catalog sentences (path: crates/made-app/src/diagnostics.rs)
- [x] 7.3 Confirm 7.1 fails, then while the install succeeded or the installed version is already current, set each workspace's row from that workspace alone and keep it current as that running condition changes: ready in the success tone and `Grok Build session follow is ready.` when either its terminal target is Grok and that process is still alive, or its shell has at least one open Grok session; otherwise inactive and `Grok Build is not running.` Do not use the tab label, or `workspace_grok_branded`, as the running signal, because OpenCode owning the label must not hide a running Grok. Another workspace's Grok must not change this row. Leave checking, not on PATH, installing, hooks unavailable, and the explicit-catalog sentence unchanged. Extend the test-only hold so the UI test can mark this workspace's living Grok target or open Grok sessions without spawning grok, looking up PATH, or installing. Product startup still must not block on detection or install, and a catalog that did not run detection must not install (path: crates/made-app/src/app.rs)

## Checkpoint
- [x] verify Grok session follow is ready
- [x] verify OpenCode label does not hide a running Grok
- [x] verify Grok Build is not running
- [x] verify Another workspace's Grok does not ready this row

## 8. Match the OpenCode diagnostics wording to the Grok row

Change only the OpenCode visible pill label and the named reason sentences. While session follow is not connected and no other OpenCode reason applies, the pill stays inactive and the sentence is `OpenCode is not running.` While connected, including after a reconnect, the visible pill says `ready` in the existing success tone and the sentence is `OpenCode session follow is ready.` The machine-readable status code may stay `connected`. When OpenCode is not on PATH, the sentence is `OpenCode is not on PATH.` and the pill keeps the treatment already used for that reason. Leave every other OpenCode reason sentence unchanged, including focus, heartbeat, protocol, and directory messages. Leave every Grok sentence, the Grok Build tab label, and the menu row unchanged. Do not change OpenCode launch, follow, or dots.

- [x] 8.1 Write a failing UI test that revealed About diagnostics show an inactive OpenCode pill and `OpenCode is not running.` while session follow is not connected and no other OpenCode reason applies; a visible pill `ready` in the existing success tone and `OpenCode session follow is ready.` while connected and again after a reconnect, while the machine-readable status code may stay `connected`; and `OpenCode is not on PATH.` when OpenCode is not on PATH, with that pill keeping the degraded or unsupported treatment already used for that reason. The row does not show `Optional OpenCode integration is inactive.` Copying the diagnostics report includes the OpenCode sentence shown. Grok sentences, the Grok Build tab label, and the menu row stay unchanged [P] (path: crates/made-app/tests/ui_model.rs)
- [x] 8.2 Write a failing test that the inactive sentence is `OpenCode is not running.`, the connected and reconnected sentences are `OpenCode session follow is ready.`, and the not-on-PATH sentence is `OpenCode is not on PATH.`; the copied reason uses those sentences; a connected snapshot keeps the machine-readable status code `connected`; and focus, heartbeat, protocol, directory, and every other OpenCode reason sentence stay unchanged. Replace the assertion that a connected reason contains `Connected` [P] (path: crates/made-app/tests/diagnostics.rs)
- [x] 8.3 Confirm 8.1 fails, then paint the visible OpenCode pill label `ready` in the existing success tone while session follow is connected, including after a reconnect, and keep the inactive pill while it is not connected and no other reason applies. Do not change the not-on-PATH pill treatment, the machine-readable status code, or any Grok pill (path: crates/made-app/src/ui.rs)
- [x] 8.4 Confirm 8.2 fails, then set the inactive sentence to `OpenCode is not running.`, the connected and reconnected sentences to `OpenCode session follow is ready.`, and the not-on-PATH sentence to `OpenCode is not on PATH.` Leave every other OpenCode reason sentence unchanged (path: crates/made-app/src/integration_status.rs)

## Checkpoint
- [x] verify OpenCode is not running
- [x] verify OpenCode session follow is ready
- [x] verify OpenCode session follow is ready after a reconnect
- [x] verify OpenCode is not on PATH
- [x] verify Other OpenCode reason sentences stay
- [x] verify Grok sentences stay
