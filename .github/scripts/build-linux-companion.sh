#!/usr/bin/env bash
# Build, test, and stage one Linux companion inside a glibc 2.31 userspace.
# The GitHub runner is only the Docker host. This script must observe glibc
# 2.31 itself; ubuntu-24.04's glibc 2.39 is not an acceptable link environment.
set -euo pipefail

target="${TARGET:?TARGET required}"
case "$target" in
  x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu) ;;
  *)
    echo "build-linux-companion.sh refuses $target" >&2
    exit 1
    ;;
esac

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

machine="$(uname -m)"
case "$target:$machine" in
  x86_64-unknown-linux-gnu:x86_64|aarch64-unknown-linux-gnu:aarch64) ;;
  *)
    echo "refusing to cross-compile $target on $machine" >&2
    exit 1
    ;;
esac

libc="$(getconf GNU_LIBC_VERSION)"
ldd_line="$(ldd --version | head -n 1)"
echo "companion build libc: $libc"
echo "ldd: $ldd_line"
case "$libc" in
  "glibc 2.31") ;;
  *)
    echo "refusing to link a companion against $libc; need glibc 2.31, not the ubuntu-24.04 default" >&2
    exit 1
    ;;
esac
case "$ldd_line" in
  *"GLIBC 2.31"*) ;;
  *)
    echo "ldd is not glibc 2.31: $ldd_line" >&2
    exit 1
    ;;
esac

if ! command -v objdump >/dev/null 2>&1; then
  apt-get update
  apt-get install -y --no-install-recommends binutils
fi

# Headless only. Do not install libx11, wayland, EGL, GL, or Vulkan headers.
# A missing GUI development package must fail the build rather than be added.

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

target_dir="${CARGO_TARGET_DIR:-target}"
bin="${target_dir}/${target}/release/made-remote"
test -s "$bin"
chmod 755 "$bin"

if ! command -v readelf >/dev/null 2>&1; then
  echo "readelf is required to reject a musl interpreter" >&2
  exit 1
fi
interpreter="$(readelf -l "$bin" | awk '/Requesting program interpreter/ { print $NF }' | tr -d '[]')"
echo "interpreter: $interpreter"
case "$interpreter" in
  *ld-musl*|*ld-linux-musl*)
    echo "musl companion is unsupported" >&2
    exit 1
    ;;
  *ld-linux*) ;;
  *)
    echo "unexpected program interpreter: $interpreter" >&2
    exit 1
    ;;
esac

echo "ldd:"
ldd "$bin"
if ldd "$bin" | grep -E 'not found|libX11|libwayland|libEGL|libGL\.|libgtk|libvulkan'; then
  echo "companion is missing a library or links a GUI library" >&2
  exit 1
fi

bash "$root/.github/scripts/check-glibc-max.sh" "$bin" 2.31

runtime_out="$(mktemp)"
runtime_err="$(mktemp)"
set +e
env -u DISPLAY -u WAYLAND_DISPLAY -u XDG_SESSION_TYPE \
  timeout 20s "$bin" >"$runtime_out" 2>"$runtime_err"
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

LINKAGE_KIND=glibc \
LINKAGE_SYSTEM=2.31 \
LINKAGE_MAX_SYMBOL=2.31 \
  bash "$root/.github/scripts/stage-companion.sh" \
    "$target" "$bin" "$root/dist/ssh-companion-${target}"

echo "staged glibc 2.31 companion for $target"
