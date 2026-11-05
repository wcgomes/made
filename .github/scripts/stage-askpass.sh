#!/usr/bin/env bash
# Package the native askpass helper for one desktop host.
# Default platform features stay off so this helper does not pull the desktop
# clipboard or backdrop crates. It is not a remote companion.
# PROFILE=release builds the release profile for installers; the default stays
# debug so development artifacts and ci-ssh-remote.yml are unchanged.
set -euo pipefail

target="${TARGET:?TARGET required}"
case "$target" in
  x86_64-unknown-linux-gnu|x86_64-apple-darwin|aarch64-apple-darwin|x86_64-pc-windows-msvc) ;;
  *)
    echo "no desktop askpass target for $target" >&2
    exit 1
    ;;
esac

profile="${PROFILE:-debug}"
case "$profile" in
  debug) profile_args=() ;;
  release) profile_args=(--release) ;;
  *)
    echo "PROFILE must be debug or release, got $profile" >&2
    exit 1
    ;;
esac

root="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=hash.sh
source "$root/.github/scripts/hash.sh"
cd "$root"

exe=""
name="made-askpass"
case "$target" in
  *windows*)
    exe=".exe"
    name="made-askpass.exe"
    ;;
esac

cargo build -p made-platform --bin made-askpass --locked --no-default-features --target "$target" \
  ${profile_args[@]+"${profile_args[@]}"}
bin="target/${target}/${profile}/made-askpass${exe}"
if [ ! -f "$bin" ] || [ ! -s "$bin" ]; then
  echo "askpass binary missing or empty: $bin" >&2
  exit 1
fi

out="$root/dist/ssh-askpass-${target}"
rm -rf "$out"
mkdir -p "$out"
cp "$bin" "$out/$name"
if [ -z "$exe" ]; then
  chmod 755 "$out/$name"
fi
hash=$(sha256_file "$out/$name")
size=$(wc -c < "$out/$name" | tr -d ' ')
if [ "$size" -le 0 ] || [ "${#hash}" -ne 64 ]; then
  echo "askpass hash or size is unusable" >&2
  exit 1
fi
cat > "$out/manifest.json" <<EOF
{
  "schema": 1,
  "kind": "askpass",
  "target": "${target}",
  "name": "${name}",
  "size": ${size},
  "sha256": "${hash}"
}
EOF
test -s "$out/$name"
test -s "$out/manifest.json"
