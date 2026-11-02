# Grok

The `+` menu is one integrated-TUI catalog, not an OpenCode-only list. Catalog order is OpenCode, then Grok. Undetected programs are omitted. When no catalog TUI is detected, `+` opens the folder picker and shows no menu. When any is detected, `Terminal` stays the first row and still opens the folder picker; detected catalog rows follow. Detection remains the existing metadata-only `PATH` lookup and does not delay shell or explorer startup. The `+` tooltip stays “Open folder”.

The Grok row label is Grok Build. Its 16px leading slot paints the official mark in `crates/made-app/assets/grok.svg` (the two paths from https://grok.com/images/favicon.svg, fill #FCFCFC, viewBox "56 56 400 400"). While OpenCode is not active, a running Grok TUI — even before a session exists — or a shell tab with open Grok sessions shows that same `Grok Build` label and official mark in the workspace tab's 16px leading slot. The tooltip is the session summary when sessions exist and omits the workspace path. When that Grok is no longer running and the tab has no open Grok sessions, the tab returns to the folder label and the terminal glyph. OpenCode, when active, keeps its label, icon, and dots. OpenCode keeps `opencode.svg` and its launch injection; see [OpenCode bridge](opencode.md).

Choosing Grok starts the absolute `grok` path with the argument `dashboard` as the child of a new home-directory tab, with no shell first and no picker, and the current tab keeps running. That launch does not inject plugin config or write the user's Grok config. The user installs `plugins/grok-made`; Made does not.

The session-follow contract is [`spec.md`](../../specs/changes/grok-session-follow/spec.md). Focusing an already-open Grok session does not move the explorer.

Related: [Layout and controls](../ui/layout-and-controls.md), [Terminal-first runtime](../runtime/terminal-first.md).
