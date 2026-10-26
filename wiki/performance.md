# Performance

Made's v1 performance evidence is a final release-build pass on Linux x64 under **Xvfb/llvmpipe software rendering**. It is explicitly not a hardware-GPU, native-desktop, macOS, or Windows result. The measurements and method are recorded below and summarized in the [v1 evidence record](records/20261026-v1-evidence.md). Product targets are SHOULD-level in [`specs/changes/opencode-session-follow-v1/spec.md`](../specs/changes/opencode-session-follow-v1/spec.md).

## Final measured results

Environment: AMD Ryzen 9 7900X3D (12C/24T), 15.9 GB RAM; no `/dev/dri`; Vulkan lavapipe + llvmpipe; Xvfb 1280×800 and 720×480; release build; isolated `HOME`/`XDG_*`/`TMPDIR`; 10,000-file workspace and three live tabs.

| Measurement | Result | Budget / note |
|---|---|---|
| Startup, 10,000 files, first tab | 108.7–115.3 ms | 2,000 ms; release/Xvfb software |
| Startup, later tabs | 8.9–14.4 ms | 2,000 ms; per-tab preparation |
| `cwd_to_root` | 3.29–3.74 ms | 500 ms; report to Explorer root |
| Tab activation | 2.94–4.68 ms | Product activation to submitted paint pass |
| Terminal input-to-render | 5.11–5.39 ms | 50 ms; release path |
| Preview roundtrip | 7.5–10.7 ms | Open plus close |
| Explorer hide | 3–6 ms | No explicit spec budget |
| Frame p95 after 14 ms repaint calibration | 14.86–15.80 ms | Under 16.667 ms; idle, normal, flood, hidden-output, 3 tabs |
| Hidden grid paints | 0 | Hidden tabs consume bounded output without painting |
| Hidden 3-tab memory | Bounded | Observation from the run, not a long-duration leak test |

## How the numbers are produced

- Active repaint cadence is calibrated to 14 ms.
- Activation, cwd, preview, and layout metrics end at the **first submitted paint pass** via an end-of-pass callback, not at the start of UI work.
- Startup is measured to an interactive terminal/root listing for a 10,000-file workspace; tree loading is lazy and off the UI thread.
- `cwd_to_root` measures validated OSC 7 report to Explorer root change, not a shell prompt scrape.
- Frame pacing is measured with harness frame intervals in software rendering; it is not a native click-to-pixel or hardware 60 fps claim.
- Hidden-work evidence includes three live tabs with sustained output, zero hidden grid paints, and bounded hidden queues/scrollback.

## Interpretation rules

- **Software versus hardware:** all final p95/frame values are software-rendered. Hardware-GPU pacing and a reference-machine input latency remain pending. Do not claim hardware 60 fps from this pass.
- **Product metric versus native input:** tab activation and input-to-render are product-path measurements. They are not native keypress-to-photon measurements.
- **Bounded memory:** “bounded” means no unbounded queue/scrollback growth was observed in the measured run; it is not a soak/leak test.
- **Earlier passes:** the historical 16.675–20.195 ms software p95 and 172–277 ms 10k startup predate the final 14 ms repaint/paint-timing fixes. Use the final release table above for the current build.
- **Scope:** Xvfb has no window manager; WSL2 is not a native Linux desktop. Native X11/Wayland, macOS, Windows/ConPTY, native IME/clipboard/mouse, and live real-OpenCode switching remain pending.

## Commands and evidence

```bash
cargo build --release -p made-app
cargo test -p made-app --test performance

# General smoke shape used for software evidence; isolate HOME/XDG/TMPDIR.
Xvfb :99 -screen 0 1280x800x24 -ac -listen tcp &
DISPLAY=127.0.0.1:99 LIBGL_ALWAYS_SOFTWARE=1 WINIT_UNIX_BACKEND=x11 \
  target/release/made-app --smoke-test 2500 /path/to/workspace
```

Method and results are recorded above; current results and caveats are also summarized in the [v1 evidence record](records/20261026-v1-evidence.md). Platform interpretation: [Platform matrix](testing/platform-matrix.md). The raw evidence tree and historical run logs were removed after summarization; do not read old pre-terminal-first numbers as current certification.

Related: [Testing strategy](testing/strategy.md), [Platform matrix](testing/platform-matrix.md), [Architecture](architecture.md).
