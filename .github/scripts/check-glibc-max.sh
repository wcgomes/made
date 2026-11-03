#!/usr/bin/env bash
# Fail if a glibc symbol version is newer than the allowed maximum.
# The maximum is major.minor (GLIBC_2.31). Patch components are ignored.
#
# Usage:
#   check-glibc-max.sh <binary> <max>
#   check-glibc-max.sh --versions <max>   # one version per line on stdin
set -euo pipefail

reject_if_newer() {
  local version="$1"
  local max="$2"
  local major minor max_major max_minor rest max_rest
  major="${version%%.*}"
  rest="${version#*.}"
  minor="${rest%%.*}"
  max_major="${max%%.*}"
  max_rest="${max#*.}"
  max_minor="${max_rest%%.*}"
  case "$major$minor$max_major$max_minor" in
    *[!0-9]*)
      echo "unparseable glibc version: $version (max $max)" >&2
      return 1
      ;;
  esac
  if [ "$major" -gt "$max_major" ] || { [ "$major" -eq "$max_major" ] && [ "$minor" -gt "$max_minor" ]; }; then
    echo "GLIBC_$version exceeds $max" >&2
    return 1
  fi
}

check_versions() {
  local max="$1"
  local version found=0
  while IFS= read -r version; do
    [ -n "$version" ] || continue
    found=1
    reject_if_newer "$version" "$max"
  done
  if [ "$found" -ne 1 ]; then
    echo "no GLIBC_ symbol versions found; refusing a musl or fully static companion" >&2
    return 1
  fi
}

if [ "${1:-}" = "--versions" ]; then
  check_versions "${2:?max version required}"
  exit
fi

binary="${1:?binary required}"
max="${2:?max version required}"
if ! command -v objdump >/dev/null 2>&1; then
  echo "objdump is required to verify glibc symbol versions" >&2
  exit 1
fi
dump="$(mktemp)"
objdump -T "$binary" >"$dump"
versions="$(grep -o 'GLIBC_[0-9.]*' "$dump" || true)"
printf '%s\n' "$versions" | sed 's/^GLIBC_//' | sort -u | check_versions "$max"
