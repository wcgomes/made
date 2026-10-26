#!/bin/sh
# Test-only MADE_SHELL wrapper: run the real OpenCode v2 TUI in standalone
# mode. It exists so an E2E run can control the terminal command without
# changing product code. It does not write user configuration.
set -eu

opencode_bin="${MADE_E2E_OPENCODE:-opencode}"
exec "$opencode_bin" --standalone "$@"
