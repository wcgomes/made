#!/usr/bin/env bash
# Bump every Made version site to one new stable version (REL-02).
#
# Usage: scripts/bump-version.sh X.Y.Z
#
# Rewrites [workspace.package].version, the OpenCode plugin's package.json and
# package-lock.json (through `npm version`), its PLUGIN_VERSION constant, the
# grok-made plugin.json, and the appVersion example in the OpenCode plugin
# README; refreshes Cargo.lock through cargo; then runs the CI version check.
# Prerelease and build versions are refused: only stable versions are released.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
new="${1:-}"
if [ "$#" -ne 1 ] || ! printf '%s' "$new" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'; then
  echo "usage: $0 X.Y.Z (a stable semantic version; got '${new}')" >&2
  exit 2
fi
for tool in cargo npm python3; do
  command -v "$tool" >/dev/null 2>&1 || { echo "bump-version: $tool is required" >&2; exit 1; }
done
cd "$root"

old="$(awk '
  /^\[/ { in_section = ($0 == "[workspace.package]") }
  in_section && /^version[[:space:]]*=/ { gsub(/^version[[:space:]]*=[[:space:]]*"|"[[:space:]]*$/, ""); print; exit }
' Cargo.toml)"
if [ -z "$old" ]; then
  echo "bump-version: [workspace.package].version not found" >&2
  exit 1
fi

# Rewrite one file through a temporary copy so a failed edit leaves it intact.
rewrite() {
  local file="$1" script="$2" tmp
  tmp="$(mktemp)"
  sed -E "$script" "$file" > "$tmp"
  cat "$tmp" > "$file"
  rm -f "$tmp"
}

awk -v new="$new" '
  /^\[/ { in_section = ($0 == "[workspace.package]") }
  in_section && !done && /^version[[:space:]]*=/ { print "version = \"" new "\""; done = 1; next }
  { print }
' Cargo.toml > Cargo.toml.bump && cat Cargo.toml.bump > Cargo.toml && rm -f Cargo.toml.bump

rewrite plugins/opencode-made/src/protocol.ts \
  "s/^(export const PLUGIN_VERSION = \")[^\"]*(\";)$/\\1${new}\\2/"
rewrite plugins/grok-made/plugin.json \
  "s/^([[:space:]]*\"version\"[[:space:]]*:[[:space:]]*\")[^\"]*(\")/\\1${new}\\2/"
old_re="$(printf '%s' "$old" | sed 's/\./\\./g')"
rewrite plugins/opencode-made/README.md \
  "s/(\"appVersion\"[[:space:]]*:[[:space:]]*\")${old_re}(\")/\\1${new}\\2/"

(cd plugins/opencode-made && npm version "$new" --no-git-tag-version --allow-same-version >/dev/null)
cargo update --workspace --quiet

python3 .github/scripts/check-versions.py
echo "bumped ${old} -> ${new}"
