# 0004: Alacritty Terminal Fallback

**Status:** Accepted fallback for v1.

## Context

The original terminal design targeted `libghostty-vt` behind a `TerminalCore` boundary. This environment has no Zig toolchain to build it. A terminal core is required for VT parsing, screen state, selection, title, cwd, and damage updates, but the Explorer, bridge, and UI should not depend on a specific implementation.

## Decision

Use **`alacritty_terminal` 0.26** as the VT implementation behind Made's `TerminalCore` boundary. The boundary owns the Made-facing screen/snapshot/damage/input types; `alacritty_terminal` is a private implementation detail in [`made-terminal`](../../crates/made-terminal/src/lib.rs).

## Consequences

- The boundary keeps snapshots, selection, title, cwd, and damage independent of the VT engine, so a future replacement does not change the Explorer, bridge, or UI architecture.
- Upgrade path: replace the private `Term`/`Processor`, keep Made-owned snapshot/damage types, rerun emulation tests and the real OpenCode TUI smoke.
- `alacritty_terminal` applies width per code point, not full grapheme clusters. Single emoji/CJK are correct; multi-code-point ZWJ sequences can occupy multiple cells. This is a documented fallback caveat, not a claim of full grapheme shaping.
- The renderer draws the cells it is given; native visual validation remains pending.
- The terminal still uses a real PTY/TUI path; the fallback does not replace process, input, or terminal-fidelity behavior.

## Evidence

[Platform matrix](../testing/platform-matrix.md), [v1 evidence](../records/20261026-v1-evidence.md), [`specs/changes/archive/20261029-opencode-session-follow-v1/design.md`](../../specs/changes/archive/20261029-opencode-session-follow-v1/design.md) D6, [`crates/made-terminal/tests/emulation.rs`](../../crates/made-terminal/tests/emulation.rs), [`crates/made-app/tests/opencode_tui.rs`](../../crates/made-app/tests/opencode_tui.rs). Architecture: [Architecture](../architecture.md).
