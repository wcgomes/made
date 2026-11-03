#!/usr/bin/env bash
# Native macOS companion. Deployment target 12.0 is mandatory; the runner OS
# is newer and must not become the binary's minimum.
set -euo pipefail

target="${TARGET:?TARGET required}"
case "$target" in
  x86_64-apple-darwin|aarch64-apple-darwin) ;;
  *)
    echo "build-macos-companion.sh refuses $target" >&2
    exit 1
    ;;
esac

if [ "${MACOSX_DEPLOYMENT_TARGET:-}" != "12.0" ]; then
  echo "MACOSX_DEPLOYMENT_TARGET must be 12.0" >&2
  exit 1
fi
export MACOSX_DEPLOYMENT_TARGET=12.0
export CFLAGS="${CFLAGS:+$CFLAGS }-mmacosx-version-min=12.0"
export LDFLAGS="${LDFLAGS:+$LDFLAGS }-mmacosx-version-min=12.0"

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

cargo test -p made-remote --locked --target "$target"
cargo test -p made-platform --test headless --locked --no-default-features --target "$target"
cargo build -p made-remote --locked --release --target "$target"

tree="$(cargo tree -p made-remote --locked --target "$target" -e features)"
if printf '%s\n' "$tree" | grep -F 'feature "desktop"'; then
  echo "made-platform desktop feature leaked into the companion" >&2
  exit 1
fi
packages="$(cargo tree -p made-remote --locked --target "$target" --prefix none --format '{p}')"
if printf '%s\n' "$packages" | grep -E '^(arboard|image|raw-window-handle|objc2|objc2-app-kit|window-vibrancy|wayland-client|wayland-sys|x11rb|x11-dl|winit|gtk) '; then
  echo "GUI crate leaked into the companion graph" >&2
  exit 1
fi

bin="target/${target}/release/made-remote"
test -s "$bin"
chmod 755 "$bin"

minos="$(otool -l "$bin" | awk '/minos / { print $2 }')"
if [ -z "$minos" ]; then
  echo "LC_BUILD_VERSION minos missing; deployment target was not recorded" >&2
  exit 1
fi
while IFS= read -r version; do
  if [ "$version" != "12.0" ]; then
    echo "macOS minos is $version, want 12.0" >&2
    exit 1
  fi
done <<< "$minos"

if otool -L "$bin" | grep -E 'AppKit|Cocoa\.framework|Metal|OpenGL|libEGL|libGL|Vulkan|QuartzCore'; then
  echo "companion links a GUI framework" >&2
  exit 1
fi

runtime_out="$(mktemp)"
runtime_err="$(mktemp)"
set +e
env -u DISPLAY -u WAYLAND_DISPLAY \
  "$bin" >"$runtime_out" 2>"$runtime_err"
status=$?
set -e
if [ "$status" -ne 1 ]; then
  echo "headless runtime smoke expected exit 1, got $status" >&2
  cat "$runtime_err" >&2 || true
  exit 1
fi
if [ -s "$runtime_out" ]; then
  echo "companion wrote protocol bytes without a frame" >&2
  exit 1
fi
if ! grep -q 'made-remote: operation failed' "$runtime_err"; then
  echo "companion did not report the headless failure" >&2
  cat "$runtime_err" >&2 || true
  exit 1
fi

# Askpass is a desktop helper, built only after the companion checks pass.
bash "$root/.github/scripts/stage-askpass.sh"

LINKAGE_KIND=macos \
LINKAGE_DEPLOYMENT_TARGET=12.0 \
LINKAGE_MINOS=12.0 \
  bash "$root/.github/scripts/stage-companion.sh" \
    "$target" "$bin" "$root/dist/ssh-companion-${target}"

echo "staged macOS 12.0 companion for $target"
