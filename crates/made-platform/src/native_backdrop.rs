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
//!   in the active state, placed below the content view so the egui surface
//!   stays visible (see the `macos` module).
//! - Windows 11 22H2 (build 22621) and newer: system Acrylic through the
//!   documented `DWMWA_SYSTEMBACKDROP_TYPE` attribute set to
//!   `DWMSBT_TRANSIENTWINDOW`, with the immersive dark mode attribute so the
//!   tint is dark. The window surface must carry alpha (see
//!   [`requires_composition_swapchain`]) for the material to show.
//! - Linux and other compositors: no effect is attempted. Native backdrop blur
//!   is compositor-managed and not portable across X11/Wayland sessions, so the
//!   in-app glass is used. This is a successful fallback, not an error.
//!
//! Older Windows builds, including Windows 10, get the in-app glass fallback.
//! The undocumented `SetWindowCompositionAttribute` acrylic path is not used:
//! it is known to lag during resize and drag. A native backdrop must improve
//! rendering, never make a window feel slower.
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
    /// Windows 11 system Acrylic.
    WindowsAcrylic,
    /// Made's portable in-app glass treatment.
    InAppGlass,
}

impl NativeBackdropKind {
    /// Human-readable label for diagnostics.
    pub const fn label(self) -> &'static str {
        match self {
            Self::MacosVibrancy => "macOS vibrancy",
            Self::WindowsAcrylic => "Windows Acrylic",
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

    /// Append a note to the reason when the applied effect cannot be seen
    /// because the window surface has no alpha. The status stays applied.
    pub fn note_surface_caveat(&mut self, caveat: Option<&str>) {
        if let (true, Some(caveat)) = (self.applied, caveat) {
            self.reason = format!("{}; {caveat}", self.reason);
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

/// Whether the GPU surface must be a composition swapchain to carry alpha.
///
/// True on Windows: wgpu's default DX12 swapchain (`DxgiFromHwnd`) exposes only
/// `CompositeAlphaMode::Opaque`, so egui-wgpu falls back to an opaque surface
/// and the translucent clear color never reaches DWM, even with Acrylic applied.
/// A DirectComposition swapchain (`DxgiFromVisual`) exposes pre-multiplied
/// alpha. Other platforms need nothing special.
pub const fn requires_composition_swapchain() -> bool {
    cfg!(target_os = "windows")
}

/// Why the window surface probably carries no alpha although the native
/// backdrop was applied, or `None` when nothing is known to be wrong.
///
/// `dx12_backend` is whether wgpu ended up on DX12. Only meaningful where
/// [`requires_composition_swapchain`] is true.
pub fn opaque_surface_caveat(dx12_backend: bool) -> Option<&'static str> {
    surface_caveat(
        requires_composition_swapchain(),
        dx12_backend,
        env::var("WGPU_DX12_PRESENTATION_SYSTEM").ok().as_deref(),
    )
}

fn surface_caveat(
    required: bool,
    dx12_backend: bool,
    presentation_system: Option<&str>,
) -> Option<&'static str> {
    if !required {
        return None;
    }
    if !dx12_backend {
        return Some("GPU backend is not DX12, so the surface is probably opaque");
    }
    match presentation_system.map(str::to_ascii_lowercase).as_deref() {
        Some("dxgifromhwnd" | "hwnd") => Some(
            "WGPU_DX12_PRESENTATION_SYSTEM selects an opaque HWND swapchain, so the surface is opaque",
        ),
        _ => None,
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
    match macos::apply_vibrancy_behind_content(&window) {
        Ok(()) => NativeBackdropStatus::applied(
            NativeBackdropKind::MacosVibrancy,
            "macOS HudWindow vibrancy applied behind the content view",
        ),
        Err(error) => NativeBackdropStatus::fallback(format!(
            "macOS vibrancy unavailable: {error}; using in-app glass"
        )),
    }
}

/// macOS `NSVisualEffectView` placement.
///
/// `window-vibrancy` adds the effect view as a subview of the window's content
/// view. eframe renders the app into that same content view's `CAMetalLayer`,
/// and AppKit composites subviews above the parent view's own layer, so the
/// effect view hides the whole UI.
///
/// The effect view therefore goes into the content view's superview, directly
/// below the content view: the native material stays behind the window content
/// and shows through the translucent clear color, while egui draws on top.
#[cfg(target_os = "macos")]
mod macos {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSAutoresizingMaskOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
        NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
    };
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    pub(super) fn apply_vibrancy_behind_content(
        window: &impl HasWindowHandle,
    ) -> Result<(), String> {
        let handle = window.window_handle().map_err(|error| error.to_string())?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err("window handle is not AppKit".into());
        };
        let mtm = MainThreadMarker::new().ok_or("not running on the main thread")?;
        // SAFETY: the AppKit handle comes from the live eframe window and this
        // one-shot call runs on the UI thread. `NSView::superview` is unsafe
        // because AppKit returns the parent view without transferring
        // ownership, so the caller must keep the hierarchy alive; `content`
        // belongs to that live window, and binding the result to `superview`
        // retains it for the duration of the insertion. The effect view is
        // retained by `superview`; no AppKit reference escapes this call.
        unsafe {
            let content: &NSView = handle.ns_view.cast::<NSView>().as_ref();
            let superview = content
                .superview()
                .ok_or("window content view has no superview")?;
            // Cover the superview, not the content view. Native fullscreen
            // gives the content view the titlebar height in addition to the
            // superview's own resize, so a content-view frame ends up one
            // titlebar short at the top.
            let effect = NSVisualEffectView::initWithFrame(mtm.alloc(), superview.bounds());
            effect.setMaterial(NSVisualEffectMaterial::HUDWindow);
            effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            effect.setState(NSVisualEffectState::Active);
            effect.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            superview.addSubview_positioned_relativeTo(
                &effect,
                NSWindowOrderingMode::Below,
                Some(content),
            );
        }
        Ok(())
    }
}

/// First Windows build with the documented `DWMWA_SYSTEMBACKDROP_TYPE`
/// attribute (Windows 11 22H2).
#[cfg(any(target_os = "windows", test))]
const ACRYLIC_MIN_BUILD: u32 = 22621;

#[cfg(any(target_os = "windows", test))]
const fn acrylic_supported(build: u32) -> bool {
    build >= ACRYLIC_MIN_BUILD
}

#[cfg(target_os = "windows")]
fn apply_platform(window: impl HasWindowHandle) -> NativeBackdropStatus {
    let build = windows_dwm::build_number();
    if !acrylic_supported(build) {
        return NativeBackdropStatus::fallback(format!(
            "Windows Acrylic needs Windows 11 22H2 (build {ACRYLIC_MIN_BUILD}) or newer, \
             found build {build}; using in-app glass"
        ));
    }
    match windows_dwm::apply_acrylic(&window) {
        Ok(()) => {
            // The system backdrop only shows through the client area when the
            // frame is extended over it (`DwmExtendFrameIntoClientArea`, -1).
            let extended = windows_dwm::extend_frame_into_client_area(&window);
            NativeBackdropStatus::applied(
                NativeBackdropKind::WindowsAcrylic,
                format!(
                    "Windows 11 Acrylic applied (build {build}){}",
                    if extended {
                        ""
                    } else {
                        "; extending the frame into the client area failed"
                    }
                ),
            )
        }
        Err(error) => NativeBackdropStatus::fallback(format!(
            "Windows Acrylic unavailable: {error} (build {build}); using in-app glass"
        )),
    }
}

#[cfg(target_os = "windows")]
mod windows_dwm {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Dwm::{
        DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMSBT_TRANSIENTWINDOW,
        DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    };
    use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
    use windows_sys::Win32::UI::Controls::MARGINS;

    /// The real OS build number (`RtlGetVersion` is not subject to manifest
    /// compatibility shims). Zero when it cannot be read.
    pub(super) fn build_number() -> u32 {
        let mut info = OSVERSIONINFOW {
            dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
            dwMajorVersion: 0,
            dwMinorVersion: 0,
            dwBuildNumber: 0,
            dwPlatformId: 0,
            szCSDVersion: [0; 128],
        };
        // SAFETY: `info` is a valid, correctly sized OSVERSIONINFOW.
        let status = unsafe { RtlGetVersion(&mut info) };
        if status >= 0 {
            info.dwBuildNumber
        } else {
            0
        }
    }

    fn hwnd(window: &impl HasWindowHandle) -> Result<HWND, String> {
        let handle = window.window_handle().map_err(|error| error.to_string())?;
        match handle.as_raw() {
            RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get() as HWND),
            _ => Err("window handle is not Win32".into()),
        }
    }

    fn set_attribute(hwnd: HWND, attribute: i32, value: i32) -> Result<(), String> {
        // SAFETY: the HWND belongs to the live eframe window and `value` is a
        // 4-byte integer that outlives the call, matching the size passed.
        let result = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                attribute as u32,
                std::ptr::from_ref(&value).cast(),
                std::mem::size_of::<i32>() as u32,
            )
        };
        if result >= 0 {
            Ok(())
        } else {
            Err(format!(
                "DwmSetWindowAttribute({attribute}) failed: HRESULT 0x{result:08X}"
            ))
        }
    }

    /// Dark immersive mode first (a cosmetic tint hint whose failure is not
    /// fatal), then the system Acrylic backdrop.
    pub(super) fn apply_acrylic(window: &impl HasWindowHandle) -> Result<(), String> {
        let hwnd = hwnd(window)?;
        let _ = set_attribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, 1);
        set_attribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_TRANSIENTWINDOW)
    }

    pub(super) fn extend_frame_into_client_area(window: &impl HasWindowHandle) -> bool {
        let Ok(hwnd) = hwnd(window) else {
            return false;
        };
        let margins = MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        };
        // SAFETY: the HWND belongs to the live eframe window and `margins`
        // outlives the call.
        unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins) >= 0 }
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
            NativeBackdropKind::WindowsAcrylic,
            "Windows 11 Acrylic applied",
        );
        assert!(native.is_applied());
        assert_eq!(native.kind(), NativeBackdropKind::WindowsAcrylic);
        assert_eq!(native.label(), "Windows Acrylic");
        assert_eq!(native.reason(), "Windows 11 Acrylic applied");

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
    fn surface_caveat_is_honest_about_opaque_surfaces() {
        assert_eq!(surface_caveat(false, false, Some("hwnd")), None);
        assert_eq!(surface_caveat(true, true, None), None);
        assert_eq!(surface_caveat(true, true, Some("DxgiFromVisual")), None);
        assert!(surface_caveat(true, false, None)
            .unwrap()
            .contains("not DX12"));
        for value in ["hwnd", "DxgiFromHwnd"] {
            assert!(surface_caveat(true, true, Some(value))
                .unwrap()
                .contains("opaque HWND"));
        }
        let mut status =
            NativeBackdropStatus::applied(NativeBackdropKind::WindowsAcrylic, "Acrylic applied");
        status.note_surface_caveat(Some("surface is opaque"));
        assert!(status.is_applied());
        assert_eq!(status.reason(), "Acrylic applied; surface is opaque");
        let mut fallback = NativeBackdropStatus::fallback("none");
        fallback.note_surface_caveat(Some("ignored"));
        assert_eq!(fallback.reason(), "none");
    }

    #[test]
    fn acrylic_is_gated_on_the_documented_backdrop_build() {
        assert!(!acrylic_supported(0));
        assert!(!acrylic_supported(19045)); // Windows 10 22H2
        assert!(!acrylic_supported(22000)); // Windows 11 21H2
        assert!(!acrylic_supported(22620));
        assert!(acrylic_supported(22621)); // Windows 11 22H2
        assert!(acrylic_supported(26300));
    }

    #[test]
    fn only_windows_requires_a_composition_swapchain() {
        assert_eq!(
            requires_composition_swapchain(),
            cfg!(target_os = "windows")
        );
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
