#!/bin/sh
# Test-only end-to-end harness for Made's bridge -> session-follow -> explorer
# wiring. It launches the real Made window under Xvfb with the synthetic plugin
# in tests/e2e/fake-plugin.mjs and asserts the observed state.
#
# This is not real OpenCode tab evidence. It deliberately uses an isolated
# HOME/XDG/TMPDIR and does not touch the user's OpenCode configuration.
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
app="${MADE_APP:-$repo/target/debug/made-app}"
node_bin="${MADE_E2E_NODE:-$(command -v node)}"

if [ ! -x "$app" ]; then
  echo "session-follow-e2e: missing executable $app (run: cargo build -p made-app)" >&2
  exit 2
fi
if [ -z "$node_bin" ] || [ ! -x "$node_bin" ]; then
  echo "session-follow-e2e: node is not available" >&2
  exit 2
fi
if ! command -v xvfb-run >/dev/null 2>&1; then
  echo "session-follow-e2e: xvfb-run is not available" >&2
  exit 2
fi

tmp_base="${TMPDIR:-/tmp/opencode}"
mkdir -p "$tmp_base"
root=$(mktemp -d "$tmp_base/made-session-follow-e2e.XXXXXX")
mkdir -p "$root/home" "$root/tmp" "$root/worktree-a/src" "$root/worktree-b/src"
printf 'worktree A file\n' > "$root/worktree-a/src/a.txt"
printf 'worktree B file\n' > "$root/worktree-b/src/b.txt"

env -u OPENCODE_CLI_CONFIG_CONTENT -u WAYLAND_DISPLAY \
  HOME="$root/home" \
  XDG_CONFIG_HOME="$root/home/.config" \
  XDG_DATA_HOME="$root/home/.local/share" \
  XDG_STATE_HOME="$root/home/.local/state" \
  XDG_CACHE_HOME="$root/home/.cache" \
  TMPDIR="$root/tmp" \
  WINIT_UNIX_BACKEND=x11 \
  WGPU_BACKEND=vulkan \
  VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  LIBGL_ALWAYS_SOFTWARE=1 \
  MADE_SHELL="$repo/tests/e2e/fake-plugin-wrapper.sh" \
  MADE_E2E_NODE="$node_bin" \
  MADE_E2E_WORKTREE_A="$root/worktree-a" \
  MADE_E2E_WORKTREE_B="$root/worktree-b" \
  MADE_E2E_FAKE_LOG="$root/fake-plugin.jsonl" \
  MADE_E2E_SWITCH_DELAY_MS=1200 \
  xvfb-run -a -s "-screen 0 1280x800x24" \
  "$app" --smoke-test 7000 "$root/worktree-a" > "$root/smoke.out" 2>&1

log="$root/home/.local/state/made/log/integration.log"
fail=0
check() {
  label=$1
  file=$2
  pattern=$3
  if grep -Fq "$pattern" "$file"; then
    echo "PASS $label"
  else
    echo "FAIL $label (missing: $pattern)" >&2
    fail=1
  fi
}

check "smoke marker" "$root/smoke.out" "made smoke: OK"
check "connected status" "$root/smoke.out" "status=connected reason=connected"
check "explorer root B" "$root/smoke.out" "explorer=[root=$root/worktree-b error=none]"
check "active session B" "$log" "active session: ses_e2e_worktree_b"
check "two session rows" "$log" "shutting down; 2 session row(s), status connected"
check "revision 2" "$log" "last revision: 2"
check "root switch log" "$log" "explorer root switched to $root/worktree-b"
check "fake plugin second revision" "$root/fake-plugin.jsonl" '"revision":2,"activeSessionID":"ses_e2e_worktree_b"'

echo "session-follow-e2e root: $root"
if [ "$fail" -ne 0 ]; then
  echo "session-follow-e2e: FAIL" >&2
  echo "--- smoke ---" >&2
  cat "$root/smoke.out" >&2
  echo "--- diagnostics ---" >&2
  cat "$log" >&2
  exit 1
fi
echo "session-follow-e2e: PASS"
