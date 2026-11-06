//! Companion launch of a remote Claude Code (SSH-11). Fixtures are temp
//! directories; nothing is spawned, version-checked or written.
use made_platform::Environment;
use made_remote::claude;

#[cfg(unix)]
fn executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
#[cfg(unix)]
fn launch_is_the_absolute_claude_on_path_with_no_arguments() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    executable(&bin.join("claude"));
    let environment = Environment::from_pairs([("PATH", bin.as_os_str().to_os_string())]);
    let launch = claude::launch(&environment).expect("claude on PATH");
    assert_eq!(launch.program, bin.join("claude"));
    assert!(launch.program.is_absolute());
    assert!(launch.args.is_empty(), "claude never receives an argument");
}

#[test]
#[cfg(unix)]
fn launch_ignores_relative_and_empty_path_entries_and_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    executable(&bin.join("claude"));
    for path in ["relative", "", "bin::."] {
        let environment = Environment::from_pairs([("PATH", path)]);
        assert!(claude::launch(&environment).is_none(), "{path:?}");
    }
    // A vanished executable fails closed instead of substituting a program.
    let empty = temp.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let environment = Environment::from_pairs([("PATH", empty.as_os_str().to_os_string())]);
    assert!(claude::launch(&environment).is_none());
    assert!(claude::launch(&Environment::from_pairs([("HOME", "/x")])).is_none());
}
