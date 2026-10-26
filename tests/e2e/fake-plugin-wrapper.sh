#!/bin/sh
# Test-only MADE_SHELL wrapper: run the fake OpenCode plugin harness.
# It reads the per-launch OPENCODE_CLI_CONFIG_CONTENT from the child
# environment, exactly like a real OpenCode plugin would receive its options.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec "${MADE_E2E_NODE:-node}" "$here/fake-plugin.mjs" "$@"
