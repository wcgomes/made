#!/bin/sh
# Test-only MADE_SHELL wrapper: run the real OpenCode v2 TUI in its default
# managed-service mode (no --standalone). The E2E harness configures an
# isolated service port in the isolated HOME/XDG profile before launching.
set -eu

opencode_bin="${MADE_E2E_OPENCODE:-opencode}"
exec "$opencode_bin" "$@"
