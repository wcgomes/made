#!/usr/bin/env bash
# TEMP release/v0.1.0: run the workspace tests one package at a time, each under
# its own timeout, and publish the outcome of every package as an annotation, so
# a hung test names its package and last test. Usage: temp-test-by-package.sh <target>
set -uo pipefail
target="$1"
status=0
for package in made-app made-bridge made-explorer made-grok-hook made-platform made-remote made-terminal; do
  log="$(mktemp)"
  timeout -k 20 420 cargo test -p "$package" --locked --no-fail-fast --target "$target" \
    -- --test-threads=1 --nocapture >"$log" 2>&1
  code=$?
  if [ "$code" -eq 0 ]; then
    printf '::notice title=%s::passed\n' "$package"
  else
    status=1
    {
      echo "exit $code"
      grep -E '^ +Running |FAILED|panicked at' "$log" | tail -n 14
      echo '--- tail ---'
      tail -n 12 "$log"
    } | cut -c1-260 | head -c 5000 > "$log.msg"
    printf '::error title=%s::%s\n' "$package" "$(sed ':a;N;$!ba;s/%/%25/g;s/\r//g;s/\n/%0A/g' "$log.msg")"
  fi
  pkill -f "target/$target/debug/deps" 2>/dev/null || true
done
exit "$status"
