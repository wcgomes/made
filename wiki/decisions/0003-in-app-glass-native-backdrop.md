# 0003: In-App Glass and Native Backdrop

**Status:** Accepted for v1; native effect runtime validation pending.

## Context

Made wants a quiet, layered dark UI with a frosted top band and Explorer card. Native blur is not portable: Linux X11/Wayland compositors differ, macOS exposes `NSVisualEffectView`, and Windows 11 offers Mica but acrylic/blur can add resize/drag lag. The UI must remain readable if the OS ignores window alpha.

## Decision

Use an always-present **in-app glass approximation** and treat a native material as an optional layer behind it.

- In-app glass: translucent tonal fills (`BG_CHROME_GLASS` 205, `BG_CARD_GLASS` 202, `BG_HEADER_GLASS` 208), painted soft shadow, 1px top highlight.
- Optional native material: macOS `NSVisualEffectView` HudWindow vibrancy; Windows 11 Mica only. No Windows acrylic/blur. No Linux native blur attempt.
- The attempt runs once on the first UI frame and never retries or switches materials.
- `MADE_NATIVE_BACKDROP=auto` attempts; `off` skips. Unsupported platforms/failures return `InAppGlass` with `applied=false` and an honest reason.
- Window clear alpha is 190 normally, 150 only when the native effect was actually applied.
- `TRANSLUCENT_WINDOW=false` is the opaque kill switch.
- Terminal, preview, picker, diagnostics, tooltips, and toast stay opaque.

## Consequences

- Linux and unsupported compositors get a successful, honest fallback rather than a degraded UI.
- macOS/Windows native paths are compile-checked only in this repository; no runtime blur/contrast claim is valid without native evidence.
- The UI has no OS branch; selection stays in [`made-platform`](../../crates/made-platform/src/native_backdrop.rs).
- If the compositor ignores alpha, the RGB clear renders solid and the UI is unchanged.
- Native material must not slow resize/drag; this is why Windows acrylic/blur are excluded.

## Evidence

[`specs/changes/made-ui-ux-v1/spec.md`](../../specs/changes/made-ui-ux-v1/spec.md), [UI design](../../specs/changes/made-ui-ux-v1/design.md), [`crates/made-platform/src/native_backdrop.rs`](../../crates/made-platform/src/native_backdrop.rs), [`crates/made-app/src/theme.rs`](../../crates/made-app/src/theme.rs). Linux fallback evidence is summarized in [v1 evidence](../records/20261026-v1-evidence.md); the native effect remains compile-only.
