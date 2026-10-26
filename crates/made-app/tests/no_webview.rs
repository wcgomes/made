//! No-webview tests: process/mapping scan, honest fallback, build identity,
//! and dependency lock-file audit.

use std::path::Path;

use made_app::runtime_guard::{
    audit_lockfile, build_identity, inspect_current_process, path_names_webview, scan_maps_text,
    scan_process_text, verify_build, DependencyAudit, WebviewCheck, BUILD_WEBVIEW_RUNTIME,
};

const LOCKFILE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"));

#[test]
fn process_and_mapping_markers_are_detected() {
    assert_eq!(
        scan_maps_text("7f00-7f01 r-xp 00000000 00:00 0 /usr/lib/x86_64-linux-gnu/libcef.so\n"),
        vec!["libcef"]
    );
    assert_eq!(
        scan_process_text("/usr/bin/electron --type=renderer"),
        vec!["electron"]
    );
    assert_eq!(
        scan_maps_text("/usr/lib/libwebkit2gtk-4.1.so.0\n"),
        vec!["webkit2gtk"]
    );
    assert!(scan_maps_text("/usr/lib/libgtk-3.so.0\n/usr/lib/libc.so.6\n").is_empty());
}

#[test]
fn path_marker_helper_matches_runtime_names() {
    assert!(path_names_webview(Path::new("/usr/lib/libcef.so")));
    assert!(path_names_webview(Path::new(
        "C:\\Program Files\\Made\\WebView2Loader.dll"
    )));
    assert!(!path_names_webview(Path::new("/usr/lib/libgtk-3.so")));
}

#[test]
fn current_process_is_clean_on_linux_and_honest_elsewhere() {
    let check = inspect_current_process();
    if Path::new("/proc/self/maps").is_file() {
        match &check {
            WebviewCheck::Clean {
                processes_checked,
                mappings_checked,
            } => {
                assert!(*processes_checked >= 1);
                assert!(*mappings_checked > 0);
            }
            other => panic!("expected a clean process check on this host, got {other:?}"),
        }
    } else {
        assert!(
            check.is_unavailable(),
            "a host without /proc must report unavailable, not clean: {check:?}"
        );
    }
}

#[test]
fn the_workspace_lockfile_has_no_webview_runtime_dependency() {
    let audit = audit_lockfile(LOCKFILE);
    match &audit {
        DependencyAudit::Clean { packages } => assert!(*packages > 0),
        other => panic!("unexpected webview dependency audit: {other:?}"),
    }
    assert!(audit.is_clean());
}

#[test]
fn a_lockfile_with_a_webview_crate_is_rejected() {
    let fake = r#"
[[package]]
name = "made-app"
version = "0.1.0"

[[package]]
name = "wry"
version = "0.50.0"

[[package]]
name = "webview2-com"
version = "0.38.0"
"#;
    let audit = audit_lockfile(fake);
    match audit {
        DependencyAudit::WebviewDependencies { packages } => {
            assert_eq!(packages, vec!["webview2-com", "wry"])
        }
        other => panic!("expected webview dependencies, got {other:?}"),
    }
}

#[test]
fn an_empty_lockfile_is_not_claimed_clean() {
    assert!(matches!(
        audit_lockfile("version = 4\n"),
        DependencyAudit::Invalid { .. }
    ));
}

#[test]
fn the_build_identity_declares_no_webview_runtime() {
    assert!(BUILD_WEBVIEW_RUNTIME.is_none());
    let identity = build_identity();
    assert_eq!(identity.app, "Made");
    assert!(!identity.version.is_empty());
    assert!(!identity.target_os.is_empty());
    assert!(!identity.target_arch.is_empty());
    assert!(identity.webview_runtime.is_none());
    assert_eq!(verify_build().unwrap(), identity);
}
