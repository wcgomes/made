# Platform Matrix

Automated coverage was observed green on **Linux x64, macOS arm64/x64, and Windows x64** at the 2026-10-28 snapshot. Native runtime evidence now includes a **macOS arm64 manual rendering check** on 2026-10-28 and **user-confirmed macOS clipboard image paste** on 2026-11-01; Linux x64 WSL2 under Xvfb/llvmpipe remains software rendering. Neither is a full native UX pass, and Windows has no native UX evidence. The matrix below separates build/automated coverage from native runtime evidence. Requirements are in the [core spec](../../specs/opencode-session-follow.md) and [UI spec](../../specs/changes/archive/20261029-made-ui-ux-v1/spec.md); current gaps are summarized below and in the [2026-10-26 v1 evidence snapshot](../records/20261026-v1-evidence.md).

## Runtime status

| Platform / display | Build and automated | Runtime evidence | Status | Notes |
|---|---|---|---|---|
| Linux x64 WSL2 / Xvfb X11 | 658/0/2; fmt/clippy; plugin 29/29; Phase 0 19/19+5/5 | Terminal-first GUI, home/picker, glass fallback, tabs, preview, bash cwd, deleted-cwd, 10k startup, final software perf, bounded diagnostics log rotation/retention, 2026-10-26/27 mint/header/picker-tree and tab Xvfb checks; selected-tab stroke is in the [UI spec](../../specs/changes/archive/20261029-made-ui-ux-v1/spec.md) | **PASS in scope** | Software rendering, no WM; WSL2 is not a native desktop; Xvfb cannot show alpha blending over a light root. |
| Linux x64 native X11 | CI green 2026-10-28 (build/test); Xvfb smoke | None | **PENDING native UX** | Real WM, pointer/keyboard, IME, clipboard, resize, native home, compositor glass; CI Xvfb is not native X11. |
| Linux x64 native Wayland | CI green 2026-10-28 (build/test); headless Weston smoke | Weston smoke only | **PENDING native UX** | Native compositor input/IME/clipboard and real OpenCode TUI. |
| macOS arm64 | CI green 2026-10-28 (build/contract/app smoke) | Manual native check 2026-10-28: window/UI rendered, HudWindow vibrancy applied and visible; windowed/resized/fullscreen. Clipboard image paste user-confirmed 2026-11-01. | **PENDING native UX** | Rendering/vibrancy and clipboard image paste checked; native home, terminal, tabs, cwd, preview, IME/mouse/full clipboard, full UX pass pending. |
| macOS x64 | CI green 2026-10-28 (build/contract/app smoke) | CI smoke only | **PENDING native UX** | No native run; native backdrop and UI remain compile-checked only. |
| Windows x64 native/ConPTY | CI green 2026-10-28 (build/contract/PTY/app smoke) | CI smoke only | **PENDING native UX** | Native OpenCode, ConPTY resize, Job Object cleanup, Mica, IME/clipboard. WSL is not a substitute. |

## Shell support

| Shell | Status |
|---|---|
| bash | Real installed-shell run verified: profile preserved, rc hash unchanged, spaces/Unicode cd followed. |
| zsh | Source/unit coverage only; real run pending. |
| fish | Source/unit coverage only; real run pending. |
| PowerShell/pwsh | Source/unit coverage only; real run pending. |
| `cmd` (Windows) | Launch-local OSC 7 via `PROMPT`; source/unit coverage only, native Windows run pending. |
| Unknown/restricted launch | Explicit unsupported capability/reason; no live-follow claim. |

The shell matrix is a release-blocking gap for v1: the spec names all four supported shells, but only bash has real evidence.

## Capability status

| Capability | Linux Xvfb | Native Linux | macOS | Windows |
|---|---|---|---|---|
| In-app glass fallback (Explorer) | Verified | Pending compositor | **Rendered on arm64**; x64 compile-only | Compile-only |
| Native backdrop | N/A (honest fallback log) | N/A | **Verified on arm64**; x64 compile-only | **Compile-only** (Mica only; no acrylic) |
| Mint header, 26/24 card-edge header, 215 fallback alpha, tab/close scale per the [UI spec](../../specs/changes/archive/20261029-made-ui-ux-v1/spec.md), splitter fade, scrollbars, picker tree/path chip | Headless + temporary Xvfb checks | Pointer/compositor pending | Pointer pending | Pointer pending |
| Preview roundtrip | Native Xvfb | Pending | Pending | Pending |
| Diagnostics log rotation, `.1` retention, 0600/0700, bounded lock/refusal, final-exit stderr visibility gap | Verified Linux automated | Pending native lock/reparse check | Pending native lock/reparse check | Pending native lock/reparse check |
| Native IME/clipboard/mouse/paste | Not native (Xvfb) | Not yet tested | **Clipboard image paste user-confirmed 2026-11-01**; IME/mouse/full clipboard not yet tested | Not yet tested |
| Hardware-GPU frame pacing | Software only | Pending | Pending | Pending |
| Job Object descendant cleanup | N/A | N/A | N/A | Implemented/cross-compiled; native pending |
| Packaged artifact launch | Installed smoke on hosted runner only | Installed smoke on hosted runner only | Installed smoke on hosted runner only | Installed smoke on hosted runner only |

## CI coverage and observed status

Release workflows (`release.yml`, `upgrade-check.yml`) are summarized in [Release and CI](../operations/release-ci.md). Development workflows cover Linux lint/test, four-target build/test, Linux X11/Wayland smoke, macOS arm64/x64 smoke, Windows native smoke, and development artifacts. They can prove compilation, contract tests, window/PTY smoke markers, and artifact assembly. They do **not** prove native UX, real WM/compositor input, native shell hooks, IME/clipboard/mouse/paste, Job Object cleanup, native OpenCode TUI interaction, native backdrop rendering, or hardware-GPU pacing.

At the 2026-10-28 snapshot, all five workflows were observed green for the CI-stabilization commit: CI (all six jobs), macOS smoke x64/arm64, Windows native smoke, Linux display smoke, and Development artifacts. Green CI proves compilation, contract/PTY tests, window/PTY smoke markers, and artifact assembly; it does not prove the native UX list above, and the packaged layout has not been launched end-to-end. Visual captures are ad hoc validation artifacts and are not committed or maintained in the wiki. See [Release and CI](../operations/release-ci.md) and [v1 evidence](../records/20261026-v1-evidence.md).

## Unvalidated field fixes

The v0.1.0 field-issue fixes (Windows GUI subsystem, DX12 visual swapchain/Mica alpha, cmd.exe cwd, SSH cwd/locale/reattach) are Linux-tested and cross-compiled only. Native Windows, macOS, and live-SSH validation is pending. Details: [Terminal-first runtime](../runtime/terminal-first.md), [ADR 0003](../decisions/0003-in-app-glass-native-backdrop.md), [CI platform quirks](ci-platform-quirks.md).

## Release-blocking gaps

- Native Linux X11/Wayland with real desktop input.
- macOS arm64 full native UX pass; macOS x64 native run, including HudWindow vibrancy.
- Windows x64 native/ConPTY, including Mica, Job Object cleanup, and native OpenCode.
- Real zsh/fish/PowerShell cwd runs.
- Live real-OpenCode switching, worktree removal/indicator, and native session-map focus.
- Native IME/clipboard/mouse/paste/title/focus (macOS clipboard image paste user-confirmed 2026-11-01; Linux and Windows not yet tested; no known defect).
- Hardware-GPU pacing/reference input latency.
- Packaged artifact launch on real machines: release CI launches the installed layout only through `--smoke-test` on hosted runners; fresh-machine installs, Fedora, Wayland and click-through remain `UNVERIFIED` per the [release record](../records/20261106-v0-1-0-release.md).
- Cross-user bridge DoS acceptance or reserved slot.

Accepted residuals such as read-only preview, optional OpenCode only, no worktree lifecycle management, no code signing or notarization (installers and auto-update now exist; see the [release record](../records/20261106-v0-1-0-release.md)), and same-user token exposure are documented in the [v1 evidence record](../records/20261026-v1-evidence.md) and [Security](../security.md).

Related: [Test strategy](strategy.md), [CI platform quirks](ci-platform-quirks.md), [Performance](../performance.md), [v1 evidence](../records/20261026-v1-evidence.md).
