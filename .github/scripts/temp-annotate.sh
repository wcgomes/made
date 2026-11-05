#!/usr/bin/env bash
# TEMP release/v0.1.0: run a command and, when it fails, publish the failing
# lines and the log tail as one error annotation, because job logs are not
# readable without an authenticated session. Usage: temp-annotate.sh <title> -- <command...>
set -uo pipefail
title="$1"
shift
[ "$1" = "--" ] && shift
log="$(mktemp)"
# Write to a file, not a pipe: a hung test child must not keep this script
# waiting after `timeout` killed the command.
"$@" >"$log" 2>&1
code=$?
cat "$log"
if [ "$code" -ne 0 ]; then
  {
    echo "exit code $code"
    grep -E 'FAILED|FAIL |panicked at|assertion|^error|Error:|WARNING|BLOCKED|first-fail' "$log" | head -n 40
    echo '--- tail ---'
    grep -E '^ +Running |^test ' "$log" | tail -n 12
    echo '--- raw tail ---'
    tail -n 25 "$log"
  } | cut -c1-300 | head -c 6000 > "$log.msg"
  printf '::error title=%s::%s\n' "$title" "$(sed ':a;N;$!ba;s/%/%25/g;s/\r//g;s/\n/%0A/g' "$log.msg")"
fi
exit "$code"
