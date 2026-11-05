#!/usr/bin/env bash
# Stage the shipped grok-made package: the repository's source files plus the
# built session-status hook. Usage: stage-grok-plugin.sh <target> <hook-binary>
# <out>, where <out> becomes the package directory (`plugins/grok-made`).
#
# The staged package holds exactly plugin.json, README.md, hooks/hooks.json and
# hooks/bin/<hook> (mode 755). plugin.json carries the workspace version. Grok
# on Windows is only documented to start `.exe` paths, so the Windows target
# gets a hooks.json whose command names the `.exe`.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
source_dir="${MADE_GROK_PLUGIN_SOURCE:-$root/plugins/grok-made}"

target="${1:?target required}"
hook="${2:?hook binary required}"
out="${3:?output directory required}"

hook_name="made-grok-status"
case "$target" in
  x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu|x86_64-apple-darwin|aarch64-apple-darwin) ;;
  x86_64-pc-windows-msvc) hook_name="made-grok-status.exe" ;;
  *)
    echo "unsupported grok-made target: $target" >&2
    exit 1
    ;;
esac

if [ ! -f "$hook" ] || [ -L "$hook" ] || [ ! -s "$hook" ]; then
  echo "hook binary missing or empty: $hook" >&2
  exit 1
fi
for name in plugin.json README.md hooks/hooks.json; do
  if [ ! -f "$source_dir/$name" ]; then
    echo "plugin source file missing: $source_dir/$name" >&2
    exit 1
  fi
done

version="$(awk '
  /^\[/ { in_section = ($0 == "[workspace.package]") }
  in_section && /^version[[:space:]]*=/ {
    gsub(/^version[[:space:]]*=[[:space:]]*"|"[[:space:]]*$/, "")
    print
    exit
  }' "$root/Cargo.toml")"
case "$version" in
  ""|*[!0-9.]*)
    echo "workspace version is not dotted numeric: '$version'" >&2
    exit 1
    ;;
esac

rm -rf "$out"
mkdir -p "$out/hooks/bin"

sed -E 's/^([[:space:]]*"version"[[:space:]]*:[[:space:]]*")[^"]*(")/\1'"$version"'\2/' \
  "$source_dir/plugin.json" > "$out/plugin.json"
if [ "$(grep -c "\"version\"[[:space:]]*:[[:space:]]*\"$version\"" "$out/plugin.json")" -ne 1 ]; then
  echo "plugin.json must carry exactly one version field" >&2
  exit 1
fi
cp "$source_dir/README.md" "$out/README.md"
if [ "$hook_name" = "made-grok-status.exe" ]; then
  sed 's#bin/made-grok-status"#bin/made-grok-status.exe"#g' "$source_dir/hooks/hooks.json" > "$out/hooks/hooks.json"
else
  cp "$source_dir/hooks/hooks.json" "$out/hooks/hooks.json"
fi
cp "$hook" "$out/hooks/bin/$hook_name"
chmod 644 "$out/plugin.json" "$out/README.md" "$out/hooks/hooks.json"
# Grok starts the hook directly, so the executable bit must survive packaging.
chmod 755 "$out/hooks/bin/$hook_name"

echo "staged grok-made $version for $target at $out"
