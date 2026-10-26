#!/bin/sh
# Test-only MADE_SHELL wrapper that records the child environment shape and
# stays alive. It proves whether Made injected OPENCODE_CLI_CONFIG_CONTENT
# without writing any user configuration.
set -eu

log="${MADE_E2E_ENV_LOG:?MADE_E2E_ENV_LOG is required}"
{
  printf 'pid=%s\n' "$$"
  printf 'cwd=%s\n' "$PWD"
  if [ "${OPENCODE_CLI_CONFIG_CONTENT+x}" = "x" ]; then
    printf 'has_config=1\n'
    printf 'config_bytes=%s\n' "${#OPENCODE_CLI_CONFIG_CONTENT}"
    printf 'config_sha256=%s\n' "$(printf '%s' "$OPENCODE_CLI_CONFIG_CONTENT" | sha256sum | cut -d' ' -f1)"
  else
    printf 'has_config=0\n'
  fi
  if [ "${MADE_SHELL+x}" = "x" ]; then
    printf 'made_shell=%s\n' "$MADE_SHELL"
  fi
} > "$log"
exec sleep 60
