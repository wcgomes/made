# Made

**Minimal Agentic Development Environment**

A native, terminal-first desktop workspace for developers: one window, independent live workspace tabs, and a real shell plus file explorer in every tab. Use it without an agent installed; when you run OpenCode v2 in a tab — yourself or from the `+` menu — Made follows the active session and worktree.

## Why Made

- **Terminal-first, not agent-gated.** A shell and explorer open immediately. Made never installs, version-probes, or waits for an agent, and never starts one automatically.
- **Your workspace follows you.** `cd` in a supported shell and the explorer follows. If OpenCode v2 is running with an active session, that session's directory and worktree take precedence; disconnect returns to the shell cwd.
- **Independent tabs for real work.** Each workspace tab owns its own shell, process, explorer, preview, and focus. Close one without disturbing the others.
- **Native and fast.** Rust, `egui`, and a real PTY-backed terminal renderer. No webview.
- **Quiet by design.** Full-height explorer, full-left read-only preview, icon-only controls, and a mint-accented, card-edge-aligned floating header over a frosted explorer that keeps the terminal readable.

## What you get

- **Independent live workspace tabs** — switch folders, keep every shell running, and close only what you mean to close.
- **Shell cwd follow** — launch-local bash, zsh, fish, and PowerShell hooks with spaces and Unicode support. Real bash is verified; other shell runs are pending.
- **Optional OpenCode session follow** — run `opencode` yourself or start it from the `+` menu; Made follows the active session directory and worktree when available.
- **Full-height explorer** — hide, restore, and pointer-resize it without stopping the shell.
- **Full-left read-only preview** — inspect a file over the still-running terminal, then return without relaunching anything.
- **Home startup and inline folder picker** — no argument opens your home workspace; `--picker` browses an inline expandable folder tree; an explicit path wins over both.
- **Diagnostics** — inspect workspace, cwd capability, effective root, integration state, logs, and metrics without routine status chrome.
- **Mint accent and glass surfaces** — the wordmark, tabs, and icon actions sit directly on the window clear/native backdrop across the content edges. Workspace tabs use fill-only idle/hover/selected states, with a dark green-gray selected fill; the full-height explorer keeps the in-app glass. The calm mint accent marks the wordmark, primary actions, focus, and splitter drag; optional macOS vibrancy or Windows 11 Mica may layer behind the window. Linux and unsupported compositors use the in-app glass fallback.

## Quick start

### Prerequisites

- **Rust stable** via `rustup`.
- **Linux x64 on X11 or Wayland**, **macOS arm64/x64**, or **Windows x64**.
- **OpenCode v2** is optional. Install the native distribution for your OS if you want session follow. Made never launches it automatically; when `opencode` is on `PATH`, the `+` menu can start it in a home-directory tab.
- **Node.js 24** is only needed for the OpenCode plugin tests.

### Platform setup

Made is built natively on each supported OS; there is no supported cross-compilation step.

#### Linux (x64, X11 or Wayland)

The wgpu renderer and the PTY layer need X11/Wayland/EGL/Vulkan development packages before the first build.

```bash
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  libxkbcommon-dev \
  libwayland-dev \
  libx11-dev \
  libegl1-mesa-dev \
  libgl1-mesa-dev \
  libvulkan-dev
```

Other distributions need the equivalent X11/Wayland/EGL/Vulkan development packages.

Build with Rust from [rustup](https://rustup.rs); `rust-toolchain.toml` selects stable with clippy and rustfmt, and rustup installs that toolchain on first use.

**Native Linux:** a working X11 or Wayland session is required; `wgpu` selects a hardware Vulkan or GL adapter when one is available.

**WSL2 with WSLg:** install the same packages inside the distribution and the window opens on the Windows desktop. GPU support depends on the WSL version and its Vulkan stack, so a machine without a usable adapter falls back to software rendering with `LIBGL_ALWAYS_SOFTWARE=1`. WSL2 is a Linux host, not a native Linux desktop, and it does not stand in for the Windows build.

CI produces the Linux artifacts on Ubuntu 24.04, so building on a newer distribution links a newer glibc than older targets can run.

#### macOS (arm64 or x64)

```bash
xcode-select --install
```

That provides the linker. Build with Rust from [rustup](https://rustup.rs); `rust-toolchain.toml` selects stable, which rustup installs on first use.

No extra system libraries are needed: Metal, CoreText, and the clipboard come from the OS.

#### Windows (x64)

Build the Windows target natively, or run the Linux build under WSL2 (see [Linux](#linux-x64-x11-or-wayland)) — WSL2 is a Linux host and does not exercise the Windows-specific paths: ConPTY, Mica, native clipboard, and IME.

Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the **Desktop development with C++** workload; the MSVC linker is required because `x86_64-pc-windows-msvc` is the only supported Windows target.

Install Rust from [rustup](https://rustup.rs), then confirm the host toolchain and build:

```powershell
rustup show   # expect x86_64-pc-windows-msvc
cargo build -p made-app
```

The binary lands at `target\debug\made-app.exe`.

### Build and run

```bash
cargo build -p made-app

# Open your home workspace
cargo run -p made-app

# Choose a folder
cargo run -p made-app -- --picker

# Open a specific workspace
cargo run -p made-app -- /path/to/project

# Disable the optional native window backdrop
MADE_NATIVE_BACKDROP=off cargo run -p made-app
```

On PowerShell, set `$env:MADE_NATIVE_BACKDROP="off"` before launching.

Start `opencode` inside a tab — or choose OpenCode from the `+` menu when it is on `PATH` — when you want agent session follow.

### Test

```bash
cargo test --workspace

cd plugins/opencode-made
npm ci
npm test
npm run typecheck
```

## Status

Made is an **initial UX validation build**, not a production-ready release.

Automated tests and smoke checks pass in CI across the supported targets, and development artifacts are produced. Interactive native desktop validation remains pending, including IME/clipboard/pointer behavior, hardware-GPU pacing, live OpenCode workflows, and launching the packaged artifacts. Native macOS vibrancy and Windows 11 Mica have not been validated interactively.

The file preview is read-only: no editing, saving, LSP, or diff view. Builds are unsigned; there are no installers, signing, notarization, or auto-update channels.

## Supported targets

- `aarch64-apple-darwin` — macOS arm64
- `x86_64-apple-darwin` — macOS x64
- `x86_64-unknown-linux-gnu` — Linux x64 on X11 and Wayland
- `x86_64-pc-windows-msvc` — Windows x64 with native OpenCode

## Documentation

- [Made wiki](wiki/index.md)
- [Core product spec](specs/changes/archive/20261029-opencode-session-follow-v1/)
- [UI/UX spec](specs/changes/archive/20261029-made-ui-ux-v1/)
- [End-to-end test checklist](tests/e2e/ux_validation.md)
