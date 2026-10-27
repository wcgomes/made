# Platform Matrix

The current implementation is verified on **Linux x64 WSL2 under Xvfb/llvmpipe software rendering**. This is not a native Linux desktop, macOS, or Windows pass. The matrix below separates build/automated coverage from native runtime evidence. Requirements are in the [core spec](../../specs/changes/opencode-session-follow-v1/spec.md) and [UI spec](../../specs/changes/made-ui-ux-v1/spec.md); current gaps are summarized below and in the [v1 evidence record](../records/20261026-v1-evidence.md).

## Runtime status

| Platform / display | Build and automated | Runtime evidence | Status | Notes |
|---|---|---|---|---|
| Linux x64 WSL2 / Xvfb X11 | 631/0/2; fmt/clippy; plugin 29/29; Phase 0 19/19+5/5 | Terminal-first GUI, home/picker, glass fallback, tabs, preview, bash cwd, deleted-cwd, 10k startup, final software perf, 2026-10-26/27 mint/header/picker-tree and fill-only-tab Xvfb checks | **PASS in scope** | Software rendering, no WM; WSL2 is not a native desktop; Xvfb cannot show alpha blending over a light root. |
| Linux x64 native X11 | CI build defined | None | **PENDING** | Real WM, pointer/keyboard, IME, clipboard, resize, native home, compositor glass. |
| Linux x64 native Wayland | CI build defined; headless Weston smoke | WSLg/Weston smoke only | **PENDING** | Native compositor input/IME/clipboard and real OpenCode TUI. |
| macOS arm64 | CI build/test/smoke defined | None | **PENDING** | Native home resolution, terminal, tabs, cwd, preview, input, HudWindow vibrancy. |
| macOS x64 | CI build/test/smoke defined | None | **PENDING** | Same as arm64; native backdrop is compile-checked only. |
| Windows x64 native/ConPTY | CI build/test/PTY contract/smoke defined | None | **PENDING** | Native OpenCode, ConPTY resize, Job Object cleanup, Mica, IME/clipboard. WSL is not a substitute. |

## Shell support

| Shell | Status |
|---|---|
| bash | Real installed-shell run verified: profile preserved, rc hash unchanged, spaces/Unicode cd followed. |
| zsh | Source/unit coverage only; real run pending. |
| fish | Source/unit coverage only; real run pending. |
| PowerShell/pwsh | Source/unit coverage only; real run pending. |
| `cmd` | Explicit unsupported cwd capability; terminal remains usable at latest known cwd/launch folder. |
| Unknown/restricted launch | Explicit unsupported capability/reason; no live-follow claim. |

The shell matrix is a release-blocking gap for v1: the spec names all four supported shells, but only bash has real evidence.

## Capability status

| Capability | Linux Xvfb | Native Linux | macOS | Windows |
|---|---|---|---|---|
| In-app glass fallback (Explorer) | Verified | Pending compositor | Compile-only | Compile-only |
| Native backdrop | N/A (honest fallback log) | N/A | **Compile-only** | **Compile-only** (Mica only; no acrylic) |
| Mint header, 26/24 card-edge header, 225 fallback alpha, fill-only tab/close scale, splitter fade, scrollbars, picker tree/path chip | Headless + temporary Xvfb checks | Pointer/compositor pending | Pointer pending | Pointer pending |
| Preview roundtrip | Native Xvfb | Pending | Pending | Pending |
| Native IME/clipboard/mouse/paste | Not native | Pending | Pending | Pending |
| Hardware-GPU frame pacing | Software only | Pending | Pending | Pending |
| Job Object descendant cleanup | N/A | N/A | N/A | Implemented/cross-compiled; native pending |
| Packaged artifact launch | Not launched | Not launched | Not launched | Not launched |

## CI coverage and observed status

Workflows exist for Linux lint/test, four-target build/test, Linux X11/Wayland smoke, macOS arm64/x64 smoke, Windows native smoke, and development artifacts. They can prove compilation, contract tests, window/PTY smoke markers, and artifact assembly. They do **not** prove native UX, real WM/compositor input, native shell hooks, IME/clipboard/mouse/paste, Job Object cleanup, native OpenCode TUI interaction, native backdrop rendering, or hardware-GPU pacing.

At the 2026-10-26 snapshot, CI runs/artifacts had **not been observed** from this workspace, and the staged packaged layout had not been launched end-to-end. Visual captures are ad hoc validation artifacts and are not committed or maintained in the wiki. See [Release and CI](../operations/release-ci.md) and [v1 evidence](../records/20261026-v1-evidence.md).

## Release-blocking gaps

- Native Linux X11/Wayland with real desktop input.
- macOS arm64/x64 native run, including HudWindow vibrancy.
- Windows x64 native/ConPTY, including Mica, Job Object cleanup, and native OpenCode.
- Real zsh/fish/PowerShell cwd runs.
- Live real-OpenCode switching, worktree removal/indicator, and native session-map focus.
- Native IME/clipboard/mouse/paste/title/focus.
- Hardware-GPU pacing/reference input latency.
- Packaged layout launch and observed CI artifacts.
- Cross-user bridge DoS acceptance or reserved slot.

Accepted residuals such as read-only preview, optional OpenCode only, no worktree lifecycle management, no installers/signing/auto-update, and same-user token exposure are documented in the [v1 evidence record](../records/20261026-v1-evidence.md) and [Security](../security.md).

Related: [Test strategy](strategy.md), [Performance](../performance.md), [v1 evidence](../records/20261026-v1-evidence.md).
