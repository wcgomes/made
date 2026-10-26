#!/bin/sh
# Test-only MADE_SHELL wrapper: run the real OpenCode v2 TUI in standalone mode
# and, when MADE_E2E_OPENCODE_SESSION is set, ask the TUI to open that session
# tab. Used only by the evidence harness; it does not modify product behavior.
set -eu

opencode_bin="${MADE_E2E_OPENCODE:-opencode}"
session="${MADE_E2E_OPENCODE_SESSION:-}"
if [ -n "$session" ]; then
  exec "$opencode_bin" --standalone --session "$session" "$@"
fi
exec "$opencode_bin" --standalone "$@"
