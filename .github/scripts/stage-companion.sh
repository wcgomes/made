#!/usr/bin/env bash
# Stage one companion and the allowlisted plugin files the Stage stream sends.
# Usage: stage-companion.sh <target> <companion-binary> <hook-binary> <out>.
# Names match made_app::remote_session::STAGE_FILES (10 artifacts). The
# grok-made package, including the native session-status hook built for the
# same target, comes from stage-grok-plugin.sh. Empty files and files over the
# 128 MiB stage cap fail here, not at connection time.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=hash.sh
source "$root/.github/scripts/hash.sh"

target="${1:?target required}"
binary="${2:?binary required}"
hook="${3:?hook binary required}"
out="${4:?output directory required}"

case "$target" in
  x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu|x86_64-apple-darwin|aarch64-apple-darwin) ;;
  *)
    echo "unsupported companion target: $target" >&2
    exit 1
    ;;
esac

files=(
  made-remote
  plugins/opencode-made/package.json
  plugins/opencode-made/tui.ts
  plugins/opencode-made/index.ts
  plugins/opencode-made/src/protocol.ts
  plugins/opencode-made/src/integration.ts
  plugins/grok-made/plugin.json
  plugins/grok-made/README.md
  plugins/grok-made/hooks/hooks.json
  plugins/grok-made/hooks/bin/made-grok-status
)

max_bytes=$((128 * 1024 * 1024))
rm -rf "$out"
mkdir -p "$out/plugins/opencode-made/src"

cp "$binary" "$out/made-remote"
chmod 755 "$out/made-remote"
cp "$root/plugins/opencode-made/package.json" "$out/plugins/opencode-made/package.json"
cp "$root/plugins/opencode-made/tui.ts" "$out/plugins/opencode-made/tui.ts"
cp "$root/plugins/opencode-made/index.ts" "$out/plugins/opencode-made/index.ts"
cp "$root/plugins/opencode-made/src/protocol.ts" "$out/plugins/opencode-made/src/protocol.ts"
cp "$root/plugins/opencode-made/src/integration.ts" "$out/plugins/opencode-made/src/integration.ts"
# The hook runs directly; the shipped executable bit must survive packaging
# exactly like the companion binary's, and the hook must be a native binary
# for this target, never a script.
bash "$root/.github/scripts/stage-grok-plugin.sh" "$target" "$hook" "$out/plugins/grok-made"

linkage_kind="${LINKAGE_KIND:?LINKAGE_KIND required}"
case "$linkage_kind" in
  glibc)
    linkage_json=$(printf '{"kind":"glibc","system":"%s","max_symbol":"%s"}' \
      "${LINKAGE_SYSTEM:?}" "${LINKAGE_MAX_SYMBOL:?}")
    ;;
  macos)
    linkage_json=$(printf '{"kind":"macos","deployment_target":"%s","minos":"%s"}' \
      "${LINKAGE_DEPLOYMENT_TARGET:?}" "${LINKAGE_MINOS:?}")
    ;;
  *)
    echo "unsupported linkage kind: $linkage_kind" >&2
    exit 1
    ;;
esac

sums="$out/SHA256SUMS"
: > "$sums"
json="$out/manifest.json"
{
  printf '{\n'
  printf '  "schema": 1,\n'
  printf '  "target": "%s",\n' "$target"
  printf '  "profile": "release",\n'
  printf '  "linkage": %s,\n' "$linkage_json"
  printf '  "files": [\n'
} > "$json"

index=0
for name in "${files[@]}"; do
  path="$out/$name"
  if [ ! -f "$path" ] || [ -L "$path" ]; then
    echo "companion file missing: $name" >&2
    exit 1
  fi
  size=$(wc -c < "$path" | tr -d ' ')
  if [ "$size" -le 0 ] || [ "$size" -gt "$max_bytes" ]; then
    echo "companion file empty or over 128 MiB: $name ($size bytes)" >&2
    exit 1
  fi
  hash=$(sha256_file "$path")
  if [ "${#hash}" -ne 64 ] || printf '%s' "$hash" | grep -q '[^0-9a-f]'; then
    echo "sha256 for $name is not 64 lowercase hex" >&2
    exit 1
  fi
  printf '%s  %s\n' "$hash" "$name" >> "$sums"
  comma=","
  index=$((index + 1))
  if [ "$index" -eq "${#files[@]}" ]; then
    comma=""
  fi
  printf '    {"name":"%s","size":%s,"sha256":"%s"}%s\n' "$name" "$size" "$hash" "$comma" >> "$json"
done
printf '  ]\n}\n' >> "$json"

test -s "$out/made-remote"
test -s "$out/manifest.json"
test -s "$sums"
