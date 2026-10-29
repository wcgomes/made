# 0003: In-App Glass and Native Backdrop

**Status:** Accepted for v1; macOS arm64 HudWindow vibrancy/UI rendering verified natively 2026-10-28; Phase 15 glass alphas and Phase 16 conditional Explorer tokens headless/token-verified; post-fix native terminal pixel parity, native contrast certification, and full native UX validation pending.

## Context

Made wants a quiet, layered dark UI with a frosted Explorer card and a transparent floating header. Native blur is not portable: Linux X11/Wayland compositors differ, macOS exposes `NSVisualEffectView`, and Windows 11 offers Mica but acrylic/blur can add resize/drag lag. The UI must remain readable if the OS ignores window alpha.

## Decision

Use an always-present **in-app glass approximation** on the Explorer, and treat a native material as an optional layer behind the window.

- In-app glass: conditional Explorer fills using the same RGB pairs: `BG_CARD_GLASS` 188 / `BG_HEADER_GLASS` 194 without a native backdrop (off, unsupported, or failed), or `BG_CARD_GLASS_NATIVE` 24 / `BG_HEADER_GLASS_NATIVE` 32 when macOS HudWindow or Windows 11 Mica is actually applied; border, shadow, and 1px top highlight stay unchanged. Windows uses the same applied/fallback selection; Windows native runtime is not claimed.
- Workspace tabs do not use Explorer glass. Fills follow the [UI spec](../../specs/changes/archive/20261029-made-ui-ux-v1/spec.md). Selected-tab stroke follows the same spec. The earlier glass-plus-accent-border treatment stays retired.
- The workspace header is transparent and floats directly on the clear/native backdrop; it paints no band fill, border, shadow, or highlight. The wordmark, tabs, and icon actions sit directly over that backdrop.
- Optional native material: macOS `NSVisualEffectView` HudWindow vibrancy; Windows 11 Mica only. No Windows acrylic/blur. No Linux native blur attempt.
- The attempt runs once on the first UI frame and never retries or switches materials.
- `MADE_NATIVE_BACKDROP=auto` attempts; `off` skips. Unsupported platforms/failures return `InAppGlass` with `applied=false` and an honest reason.
- Window clear alpha is 215 normally, 135 only when the native effect was actually applied; the solid fallback remains `#0A0B0E`.
- `TRANSLUCENT_WINDOW=false` is the opaque kill switch.
- The terminal card paints `BG_TERMINAL_GLASS` (160) on the primary screen and opaque `BG_TERMINAL` (255) on the VT alternate screen (`ALT_SCREEN`, used by OpenCode/vim/less). It is the sole primary terminal glass layer; body/path-line/file-slot chrome add no second translucent fill, and the optional session strip/header stays opaque `BG_HEADER`. Preview, picker, diagnostics, tooltips, and toast stay opaque.

## Consequences

- Linux and unsupported compositors get a successful, honest fallback rather than a degraded UI.
- Explorer glass is conditional: the native 24/32 pair lets macOS HudWindow or Windows 11 Mica read through, while the 188/194 fallback preserves legibility without blur; Windows uses the same selection without a native-runtime claim.
- The transparent floating header avoids a second competing glass slab against the terminal while keeping the Explorer and primary-screen terminal card as the in-app glass surfaces; workspace tabs are not a glass surface, and selected-tab stroke follows the [UI spec](../../specs/changes/archive/20261029-made-ui-ux-v1/spec.md). One surface owns each glass layer, and overlays are opaque or absent; stacking the same 160 fill would composite to ~219 and show a seam.
- macOS arm64 HudWindow vibrancy and UI rendering were manually verified natively on 2026-10-28 in windowed, resized, and fullscreen states; Phase 15 glass alphas and Phase 16 conditional Explorer tokens are headless/token-verified. The 1280×800 native Explorer tint was visually approved by the user on 2026-10-28; post-fix native terminal pixel parity and native contrast certification remain pending (the display was locked for the post-fix capture). Full native macOS UX remains pending, and Windows/macOS x64 remain compile-checked only.
- The macOS effect view must sit in the content view's superview directly below the content view; a content-view subview composites above the parent view's layer, covers the wgpu `CAMetalLayer`, and hides the UI.
- The UI has no OS branch; selection stays in [`made-platform`](../../crates/made-platform/src/native_backdrop.rs).
- If the compositor ignores alpha, the RGB clear renders solid and the UI is unchanged.
- Native material must not slow resize/drag; this is why Windows acrylic/blur are excluded.

## Evidence

[`specs/changes/archive/20261029-made-ui-ux-v1/spec.md`](../../specs/changes/archive/20261029-made-ui-ux-v1/spec.md), [UI design](../../specs/changes/archive/20261029-made-ui-ux-v1/design.md), [`crates/made-platform/src/native_backdrop.rs`](../../crates/made-platform/src/native_backdrop.rs), [`crates/made-app/src/theme.rs`](../../crates/made-app/src/theme.rs). Phase 15 clear/terminal tokens (215/135 and 160 primary/opaque alternate) and Phase 16 Explorer pair (fallback 188/194, native 24/32) are headless/token-verified; the fallback `MADE_NATIVE_BACKDROP=off` rendered card 17,22,28 / header 18,23,29 with primary text 196,201,210 and tertiary header 128,134,147, with primary/tertiary contrast at 10.93:1 and 4.93:1, so it passed and the fallback was not darkened; Windows Mica uses the same selection in headless coverage, with no native-runtime claim. The macOS arm64 1280×800 native Explorer tint was visually approved by the user on 2026-10-28; native Explorer tint contrast remains uncertified. macOS arm64 HudWindow vibrancy and UI rendering were manually verified natively on 2026-10-28 in windowed, resized, and fullscreen states; the effect view must sit in the content view's superview directly below the content view so it does not cover the wgpu layer. Post-fix native terminal pixel parity and native contrast certification remain pending (the display was locked for the post-fix capture); Windows and macOS x64 native effects remain compile-only. Linux fallback evidence is summarized in [v1 evidence](../records/20261026-v1-evidence.md).
