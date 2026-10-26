//! Optional native window backdrop effects with an in-app glass fallback.
//!
//! The default Made surface is the in-app frosted-glass treatment defined by
//! `made-app::theme`. On platforms with a native, compositor-backed material
//! Made can request that material once at startup; when the platform or the
//! current OS version cannot provide it, the in-app glass remains the
//! authoritative fallback.
//!
//! - macOS: `NSVisualEffectView` vibrancy with
//!   [`NSVisualEffectMaterial::HudWindow`](https://developer.apple.com/documentation/appkit/nsvisualeffectmaterial/hudwindow)
//!   in the active state.
//! - Windows 11: Mica (`DWMSBT_MAINWINDOW` or the older Mica attribute
//!   fallback used by `window-vibrancy`).
//! - Linux and other compositors: no effect is attempted. Native backdrop blur
//!   is compositor-managed and not portable across X11/Wayland sessions, so the
//!   in-app glass is used. This is a successful fallback, not an error.
//!
//! Acrylic and blur are intentionally not used on Windows: `window-vibrancy`
//! documents resize/drag lag for those paths on current Windows 11 builds. A
//! native backdrop must improve rendering, never make a window feel slower.
//!
//! # Configuration
//!
//! `MADE_NATIVE_BACKDROP` controls the startup attempt:
//!
//! - unset, empty, `auto`, or any unrecognised value (including `on`): attempt
//!   the native effect once;
//! - `off` (ASCII case-insensitive): skip the attempt and use in-app glass.
//!
//! The value is read once per [`apply`] call. The app calls this only on its
//! first UI frame, so a running window never switches materials or retries
//! after a failure.

use std::env;
use std::ffi::OsStr;
use std::fmt;

use raw_window_handle::HasWindowHandle;

/// Environment variable that controls the native backdrop attempt.
///
/// Values are documented on the [module](self) page. Invalid values fall back
/// to `auto` so a typo cannot make the app unexpectedly opaque.
pub const NATIVE_BACKDROP_ENV: &str = "MADE_NATIVE_BACKDROP";

/// The backdrop effect backing the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeBackdropKind {
    /// macOS `NSVisualEffectView` vibrancy.
    MacosVibrancy,
    /// Windows 11 Mica.
    WindowsMica,
    /// Made's portable in-app glass treatment.
    InAppGlass,
}

impl NativeBackdropKind {
    /// Human-readable label for diagnostics.
    pub const fn label(self) -> &'static str {
        match self {
            Self::MacosVibrancy => "macOS vibrancy",
            Self::WindowsMica => "Windows Mica",
            Self::InAppGlass => "in-app glass",
        }
    }
}

impl fmt::Display for NativeBackdropKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// Outcome of one native backdrop attempt.
///
/// `applied` is true only when the OS effect is active. A fallback is always
/// represented as [`NativeBackdropKind::InAppGlass`] with a non-empty reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeBackdropStatus {
    /// The effect backing the window now.
    pub kind: NativeBackdropKind,
    /// Whether the native effect is active.
    pub applied: bool,
    /// Why the effect is active or why the fallback was chosen.
    pub reason: String,
}

impl NativeBackdropStatus {
    /// The effect backing the window now.
    pub const fn kind(&self) -> NativeBackdropKind {
        self.kind
    }

    /// Whether the native effect is active.
    pub const fn is_applied(&self) -> bool {
        self.applied
    }

    /// Human-readable label for the active effect.
    pub const fn label(&self) -> &'static str {
        self.kind.label()
    }

    /// Why the effect is active or why the fallback was chosen.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    #[cfg(any(target_os = "macos", target_os = "windows", test))]
    fn applied(kind: NativeBackdropKind, reason: impl Into<String>) -> Self {
        debug_assert!(kind != NativeBackdropKind::InAppGlass);
        Self {
            kind,
            applied: true,
            reason: reason.into(),
        }
    }

    fn fallback(reason: impl Into<String>) -> Self {
        Self {
            kind: NativeBackdropKind::InAppGlass,
            applied: false,
            reason: reason.into(),
        }
    }
}

impl Default for NativeBackdropStatus {
    fn default() -> Self {
        Self::fallback("native backdrop has not been checked")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestedMode {
    Auto,
    Off,
}

impl RequestedMode {
    fn from_env() -> Self {
        Self::from_value(env::var_os(NATIVE_BACKDROP_ENV).as_deref())
    }

    fn from_value(value: Option<&OsStr>) -> Self {
        match value.and_then(OsStr::to_str) {
            Some(value) if value.eq_ignore_ascii_case("off") => Self::Off,
            _ => Self::Auto,
        }
    }
}

/// Attempt to apply a native backdrop to `window`, once, at startup.
///
/// This function never panics on an unavailable effect: it returns an
/// [`NativeBackdropStatus`] describing the active in-app fallback and an honest
/// reason. `MADE_NATIVE_BACKDROP=off` skips touching the window handle.
///
/// The call must run on the UI/main thread. Callers should apply the returned
/// status once and keep it; repeating the call could stack native views or
/// re-enable an effect the user disabled for the process.
pub fn apply(window: impl HasWindowHandle) -> NativeBackdropStatus {
    apply_with_mode(window, RequestedMode::from_env())
}

fn apply_with_mode(window: impl HasWindowHandle, mode: RequestedMode) -> NativeBackdropStatus {
    if mode == RequestedMode::Off {
        return NativeBackdropStatus::fallback(format!("disabled by {NATIVE_BACKDROP_ENV}=off"));
    }
    apply_platform(window)
}

#[cfg(target_os = "macos")]
fn apply_platform(window: impl HasWindowHandle) -> NativeBackdropStatus {
    match window_vibrancy::apply_vibrancy(
        window,
        window_vibrancy::NSVisualEffectMaterial::HudWindow,
        Some(window_vibrancy::NSVisualEffectState::Active),
        None,
    ) {
        Ok(()) => NativeBackdropStatus::applied(
            NativeBackdropKind::MacosVibrancy,
            "macOS HudWindow vibrancy applied",
        ),
        Err(error) => NativeBackdropStatus::fallback(format!(
            "macOS vibrancy unavailable: {error}; using in-app glass"
        )),
    }
}

#[cfg(target_os = "windows")]
fn apply_platform(window: impl HasWindowHandle) -> NativeBackdropStatus {
    match window_vibrancy::apply_mica(window, Some(true)) {
        Ok(()) => NativeBackdropStatus::applied(
            NativeBackdropKind::WindowsMica,
            "Windows 11 Mica applied",
        ),
        Err(error) => NativeBackdropStatus::fallback(format!(
            "Windows Mica unavailable: {error}; using in-app glass"
        )),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn apply_platform(window: impl HasWindowHandle) -> NativeBackdropStatus {
    let _ = window;
    NativeBackdropStatus::fallback(
        "native backdrop blur is compositor-managed and not portable on this platform; \
         using in-app glass",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use raw_window_handle::{HandleError, WindowHandle};

    struct NoWindow;

    impl HasWindowHandle for NoWindow {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            panic!("an in-app fallback must not query the window handle");
        }
    }

    #[test]
    fn mode_parsing_disables_only_an_exact_off_value() {
        assert_eq!(RequestedMode::from_value(None), RequestedMode::Auto);
        assert_eq!(
            RequestedMode::from_value(Some(OsStr::new("auto"))),
            RequestedMode::Auto
        );
        assert_eq!(
            RequestedMode::from_value(Some(OsStr::new("on"))),
            RequestedMode::Auto
        );
        assert_eq!(
            RequestedMode::from_value(Some(OsStr::new(" disabled "))),
            RequestedMode::Auto
        );
        assert_eq!(
            RequestedMode::from_value(Some(OsStr::new("off"))),
            RequestedMode::Off
        );
        assert_eq!(
            RequestedMode::from_value(Some(OsStr::new("OFF"))),
            RequestedMode::Off
        );
    }

    #[test]
    fn status_accessors_describe_the_effect() {
        let native = NativeBackdropStatus::applied(
            NativeBackdropKind::WindowsMica,
            "Windows 11 Mica applied",
        );
        assert!(native.is_applied());
        assert_eq!(native.kind(), NativeBackdropKind::WindowsMica);
        assert_eq!(native.label(), "Windows Mica");
        assert_eq!(native.reason(), "Windows 11 Mica applied");

        let fallback = NativeBackdropStatus::fallback("not available");
        assert!(!fallback.is_applied());
        assert_eq!(fallback.kind(), NativeBackdropKind::InAppGlass);
        assert_eq!(fallback.label(), "in-app glass");
        assert_eq!(fallback.reason(), "not available");

        let default = NativeBackdropStatus::default();
        assert!(!default.is_applied());
        assert!(!default.reason().is_empty());
    }

    #[test]
    fn off_skips_the_window_handle_on_every_platform() {
        let status = apply_with_mode(NoWindow, RequestedMode::Off);
        assert_eq!(status.kind(), NativeBackdropKind::InAppGlass);
        assert!(!status.is_applied());
        assert!(status.reason().contains("MADE_NATIVE_BACKDROP=off"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_auto_falls_back_without_querying_the_window_handle() {
        let status = apply_with_mode(NoWindow, RequestedMode::Auto);
        assert_eq!(status.kind(), NativeBackdropKind::InAppGlass);
        assert!(!status.is_applied());
        assert!(status.reason().contains("compositor-managed"));
        assert!(status.reason().contains("not portable"));
    }
}
