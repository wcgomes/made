//! Companion launch of a remote Claude Code (SSH-11). Fixtures are temp
//! directories; nothing is spawned, version-checked or written.
use made_platform::Environment;
use made_remote::grok;

#[cfg(unix)]
fn executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
#[cfg(unix)]
fn the_launch_program_is_the_absolute_claude_on_path() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    executable(&bin.join("claude"));
    let environment = Environment::from_pairs([("PATH", bin.as_os_str().to_os_string())]);
    let program = grok::detect_on_path(&environment, "claude").expect("claude on PATH");
    assert_eq!(program, bin.join("claude"));
    assert!(program.is_absolute());
}

#[test]
#[cfg(unix)]
fn detection_ignores_relative_and_empty_path_entries_and_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    executable(&bin.join("claude"));
    for path in ["relative", "", "bin::."] {
        let environment = Environment::from_pairs([("PATH", path)]);
        assert!(
            grok::detect_on_path(&environment, "claude").is_none(),
            "{path:?}"
        );
    }
    // A vanished executable fails closed instead of substituting a program.
    let empty = temp.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let environment = Environment::from_pairs([("PATH", empty.as_os_str().to_os_string())]);
    assert!(grok::detect_on_path(&environment, "claude").is_none());
    assert!(grok::detect_on_path(&Environment::from_pairs([("HOME", "/x")]), "claude").is_none());
}
