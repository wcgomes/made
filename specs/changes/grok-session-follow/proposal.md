# Proposal: Grok session follow

## Intent

Made already follows an OpenCode session inside a terminal tab. The same workspace should follow a Grok Build session's directory and show that session's status on the workspace tab, using the behavior validated in the proof of concept. When Grok is installed it should appear in the `+` menu the same way OpenCode does, and that detect, list, and launch behavior is the catalog behavior for every integrated TUI.

## Scope

Observable Grok directory follow and workspace-tab status dots for sessions open in that tab. The `+` menu is where OpenCode and Grok are offered, from one integrated-TUI catalog, and other agents are not. Affects the OpenCode session-follow contract where it currently forbids any agent except OpenCode, and the shell-versus-agent root precedence. Workspace-tab dots reuse the existing OpenCode dot treatment. The OpenCode row keeps its current launch.

## Non-goals

Claude Code, Codex, and any other agent TUI. OSC 7501. ACP `session/list` or `x.ai/sessions/list` as the status source. A Grok session map. Focusing or switching a Grok session from Made. Made writing the user's Grok config. Changing the OpenCode row's launch behavior, follow, or dots. The shared `+` menu may gain catalog rows.

## Approach

Keep two sources. `active_sessions.json` is the inventory of sessions still open in the terminal's Grok process. The app-owned `plugins/grok-made` hook appends one status line per session event, and Made applies only lines written after it starts. The inventory refreshes about once a second and moves the explorer when a newly attached session appears. The hook moves the explorer immediately when that session starts or receives a prompt, and it sets the dot. OpenCode, when connected, keeps the root and the dots.

The `+` menu is catalog-driven. Detection stays a metadata-only PATH lookup on the existing background thread: it never spawns the program, never checks a version, ignores empty and relative PATH segments, and must not delay shell or explorer startup. With no detected integrated TUI, `+` opens the folder picker and shows no menu. Otherwise the menu is Terminal first, then one row per detected integrated TUI in catalog order.

## Decision index

- **Two sources:** the JSON inventory lists open sessions and directories; the hook supplies immediate path updates and the only status. Neither source alone is the product.
- **Plugin install:** the user installs `plugins/grok-made` with `grok plugin install`. Interactive Grok cannot load that directory for one launch. Made does not edit Grok config.
- **Ask is attention:** `ask_user_question` is the yellow dot. A permission notification is the same dot. Answering the question returns the dot to working.
- **No yank:** a one-second inventory refresh must not move the explorer back to a session that is merely the newest attachment after the user has prompted a different open session.
- **Focus limit:** selecting an already-open session, with no new attach and no new prompt, does not move the explorer. Accepted.
- **OpenCode first:** a connected OpenCode session owns the explorer root and the workspace-tab dots. This is follow precedence, not menu order.
- **Catalog order:** OpenCode, then Grok. Omit whichever is not on PATH. If only Grok is installed, the menu is Terminal, then Grok.
- **Detection:** metadata-only PATH lookup on the existing background thread. It never spawns the program, never checks a version, ignores empty and relative PATH segments, and must not delay shell or explorer startup. Made never auto-launches a TUI.
- **Grok row:** the label is Grok Build. The 16px leading slot paints the official mark in `grok.svg`: the two paths from https://grok.com/images/favicon.svg, fill #FCFCFC, viewBox "56 56 400 400". Do not redraw the mark. While OpenCode is not active, an open Grok tab — a running Grok TUI, even before a session, or a shell tab with open Grok sessions — uses that same label and mark. The tooltip is the session summary when sessions exist and omits the workspace path. When Grok is not open, the tab uses the folder label and the terminal glyph. OpenCode, when active, keeps its label, icon, and dots.
- **Grok launch:** choosing Grok starts that executable's absolute path plus the argument `dashboard` directly as the child of a new home-directory tab, with no shell first and no picker, and the current tab keeps running. That launch does not inject plugin configuration and does not write the user's Grok config. Typing `grok` in a shell stays valid and is not given that argument.
- **OpenCode row:** the shared menu may gain catalog rows. The OpenCode row keeps its vendored 16px `opencode.svg` and its existing launch behavior, including launch injection, follow, and dots.

## Clarifications

- **Q:** Should merely focusing an already-open Grok session move the explorer?
  **A:** No. Neither validated source updates on focus alone. Recorded as an accepted limit.
- **Q:** What does the Grok row in the `+` menu show, and should Made invent a Grok logo?
  **A:** The label is Grok Build. Its 16px leading slot paints the official mark in `grok.svg`, the two paths from https://grok.com/images/favicon.svg, fill #FCFCFC, viewBox "56 56 400 400", with no squircle, filter, foreignObject, or blur. That file is not in the vendored Material icons. An open Grok tab uses that same label and mark. When Grok is not open, the tab uses the folder label and the terminal glyph.
