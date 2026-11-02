# Tasks: grok-session-follow

Readiness: `analyze` must pass before implementation; review runs `verify` against the persisted spec.

Contract ref: specs/changes/grok-session-follow/

The proof of concept already lives in `plugins/grok-made`, `crates/made-app/src/grok_follow.rs`, and the workbench and workspace-tab wiring. These tasks lock that behavior with tests and close only the gaps the scenarios name. Do not rediscover the design. Menu tasks add the catalog-driven `+` rows. They do not reopen session follow.

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
