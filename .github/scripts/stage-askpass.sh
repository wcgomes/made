#!/usr/bin/env bash
# Package the native askpass helper and the made-open URL helper (the BROWSER
# target for terminal children) for one desktop host.
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
open_name="made-open"
case "$target" in
  *windows*)
    exe=".exe"
    name="made-askpass.exe"
    open_name="made-open.exe"
    ;;
esac

cargo build -p made-platform --bin made-askpass --bin made-open --locked --no-default-features --target "$target" \
  ${profile_args[@]+"${profile_args[@]}"}
bin="target/${target}/${profile}/made-askpass${exe}"
open_bin="target/${target}/${profile}/made-open${exe}"
for built in "$bin" "$open_bin"; do
  if [ ! -f "$built" ] || [ ! -s "$built" ]; then
    echo "helper binary missing or empty: $built" >&2
    exit 1
  fi
done

out="$root/dist/ssh-askpass-${target}"
rm -rf "$out"
mkdir -p "$out"
cp "$bin" "$out/$name"
cp "$open_bin" "$out/$open_name"
if [ -z "$exe" ]; then
  chmod 755 "$out/$name" "$out/$open_name"
fi
hash=$(sha256_file "$out/$name")
size=$(wc -c < "$out/$name" | tr -d ' ')
open_hash=$(sha256_file "$out/$open_name")
open_size=$(wc -c < "$out/$open_name" | tr -d ' ')
if [ "$size" -le 0 ] || [ "${#hash}" -ne 64 ] || [ "$open_size" -le 0 ] || [ "${#open_hash}" -ne 64 ]; then
  echo "helper hash or size is unusable" >&2
  exit 1
fi
cat > "$out/manifest.json" <<EOF
{
  "schema": 1,
  "kind": "askpass",
  "target": "${target}",
  "name": "${name}",
  "size": ${size},
  "sha256": "${hash}",
  "open_url": {
    "name": "${open_name}",
    "size": ${open_size},
    "sha256": "${open_hash}"
  }
}
EOF
test -s "$out/$name"
test -s "$out/$open_name"
test -s "$out/manifest.json"
