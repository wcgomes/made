//! macOS platform backend.
//!
//! Uses a POSIX PTY through `portable-pty` with the child started by
//! `posix_spawn` (see [`pty_spawn`]), standard macOS Library paths, the
//! `arboard` clipboard, and the shared IME normalizer. Builds for arm64 and x64
//! from the same source.

mod pty_spawn;
pub(crate) use pty_spawn::spawn_pty_child;

use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::{
    host_environment, is_executable_file, kill_child_and_group, resolve_shell_for,
    spawn_portable_pty, AppPaths, Clipboard, Environment, PathsInputs, Platform, PlatformError,
    PlatformFamily, PtyRequest, PtySession, ShellSpec, SystemClipboard,
};

/// macOS backend implementing [`Platform`].
#[derive(Debug, Clone, Copy, Default)]
pub struct MacOsPlatform;

impl MacOsPlatform {
    pub fn new() -> Self {
        Self
    }
}

impl Platform for MacOsPlatform {
    fn family(&self) -> PlatformFamily {
        PlatformFamily::MacOs
    }

    fn resolve_shell(&self, requested: Option<&OsStr>) -> Result<ShellSpec, PlatformError> {
        resolve_shell_for(PlatformFamily::MacOs, requested)
    }

    fn environment(&self, overrides: &[(OsString, OsString)], removed: &[OsString]) -> Environment {
        host_environment(overrides, removed)
    }

    fn app_paths(&self) -> Result<AppPaths, PlatformError> {
        AppPaths::resolve(&PathsInputs::from_env(PlatformFamily::MacOs))
    }

    fn clipboard(&self) -> Result<Box<dyn Clipboard>, PlatformError> {
        Ok(Box::new(SystemClipboard::new()?))
    }

    fn spawn_pty(&self, request: &PtyRequest) -> Result<Box<dyn PtySession>, PlatformError> {
        spawn_portable_pty(request)
    }
}

/// The `PATH` launchd gives an app started from Finder, the Dock or `open`.
pub(crate) const LAUNCHD_DEFAULT_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// Set to `1` for the login shell started by [`adopt_login_shell_path`], so
/// shell startup files can tell the probe apart and a nested Made skips it.
pub(crate) const RESOLVING_SHELL_ENV: &str = "MADE_RESOLVING_SHELL_ENV";

/// How long the login shell may take before it is killed and `PATH` is kept.
pub(crate) const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(3);

/// The shell used when `SHELL` is unset, relative or not executable.
const FALLBACK_SHELL: &str = "/bin/zsh";

/// Largest probe output read back; anything longer is not a `PATH`.
const PROBE_OUTPUT_LIMIT: u64 = 1024 * 1024;

/// Per-user install directories appended to a launchd `PATH`, relative to the
/// home directory: where the Claude Code, OpenCode and Grok installers (and
/// cargo) put their programs.
const HOME_FALLBACK_DIRS: [&str; 4] = [".local/bin", ".opencode/bin", ".grok/bin", ".cargo/bin"];

/// Homebrew prefixes appended to a launchd `PATH`.
const SYSTEM_FALLBACK_DIRS: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

/// Replace launchd's minimal `PATH` with the one the user's interactive login
/// shell builds, followed by the well-known install directories it lacks, so
/// programs installed into directories added by shell startup files (for
/// example `~/.zshrc`) are found, and Made's lookups and every child it starts
/// (PTY shells and TUIs included) share that one `PATH`.
///
/// Rule: `PATH` is changed only when the inherited one is unset, empty or
/// exactly [`LAUNCHD_DEFAULT_PATH`]. A terminal launch always inherits a
/// richer `PATH` (at least `path_helper`'s), so it is left untouched, as is a
/// run started by this probe itself ([`RESOLVING_SHELL_ENV`] set). When the
/// probe fails (timeout, missing markers, empty or non-UTF-8 output) the
/// fallback directories are appended to the inherited `PATH`, or to
/// [`LAUNCHD_DEFAULT_PATH`] when that is unset or empty.
/// Startup waits at most [`LOGIN_SHELL_TIMEOUT`] plus the time to kill the
/// shell's process group.
///
/// Must be called before the process starts any thread: it changes the
/// process environment.
pub(crate) fn adopt_login_shell_path() {
    let inherited = env::var_os("PATH");
    if !should_resolve_login_shell_path(
        inherited.as_deref(),
        env::var_os(RESOLVING_SHELL_ENV).is_some(),
    ) {
        return;
    }
    let shell = login_shell(env::var_os("SHELL").as_deref());
    let base = login_path_base(
        resolve_login_shell_path(&shell, LOGIN_SHELL_TIMEOUT),
        inherited,
    );
    let path = with_fallback_dirs(&base, env::var_os("HOME").as_deref());
    // Single-threaded at this point (see above), so no concurrent
    // environment read can observe the change.
    env::set_var("PATH", path);
}

/// The `PATH` the fallback directories are appended to: the login shell's
/// `resolved` value, else a non-empty `inherited` one, else
/// [`LAUNCHD_DEFAULT_PATH`] so the system directories are never lost.
pub(crate) fn login_path_base(resolved: Option<OsString>, inherited: Option<OsString>) -> OsString {
    resolved
        .or(inherited.filter(|path| !path.is_empty()))
        .unwrap_or_else(|| OsString::from(LAUNCHD_DEFAULT_PATH))
}

/// `path` followed by the fallback directories it does not already contain:
/// `home`'s per-user install directories, then the Homebrew prefixes. Empty
/// `path` entries are dropped. A missing, empty or relative `home` adds only
/// the Homebrew prefixes, and a `home` that cannot appear in a `PATH` (it
/// contains `:`) adds only those too. When the directories cannot be joined
/// into one value, `path` is returned unchanged.
pub(crate) fn with_fallback_dirs(path: &OsStr, home: Option<&OsStr>) -> OsString {
    let mut directories: Vec<PathBuf> = env::split_paths(path)
        .filter(|directory| !directory.as_os_str().is_empty())
        .collect();
    let home = home.map(Path::new).filter(|home| home.is_absolute());
    let fallbacks = home
        .into_iter()
        .flat_map(|home| HOME_FALLBACK_DIRS.iter().map(move |dir| home.join(dir)))
        .filter(|directory| env::join_paths([directory]).is_ok())
        .chain(SYSTEM_FALLBACK_DIRS.iter().map(PathBuf::from));
    for directory in fallbacks {
        if !directories.contains(&directory) {
            directories.push(directory);
        }
    }
    env::join_paths(directories).unwrap_or_else(|_| path.to_os_string())
}

/// Whether [`adopt_login_shell_path`] should consult the login shell for an
/// inherited `path`.
pub(crate) fn should_resolve_login_shell_path(path: Option<&OsStr>, resolving: bool) -> bool {
    if resolving {
        return false;
    }
    match path {
        None => true,
        Some(path) => path.is_empty() || path == OsStr::new(LAUNCHD_DEFAULT_PATH),
    }
}

/// The user's login shell: `shell` when it is an absolute path to an
/// executable file, else `/bin/zsh`.
pub(crate) fn login_shell(shell: Option<&OsStr>) -> PathBuf {
    shell
        .map(Path::new)
        .filter(|shell| shell.is_absolute() && is_executable_file(shell))
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(FALLBACK_SHELL))
}

/// The `PATH` an interactive login `shell` exports, or `None` on any failure.
/// A correctly marked value is accepted whatever the exit status, since a
/// startup file's last failing command must not discard it.
///
/// Runs `shell -ilc` with stdin null and stderr discarded, in a new session
/// (its own process group, no controlling terminal), with
/// [`RESOLVING_SHELL_ENV`] set. The command prints `PATH` between
/// markers unique to this call, so output from startup files cannot be taken
/// for it. Output goes to a private temporary file rather than a pipe, so no
/// reader thread is needed and a background process left holding the output
/// cannot block. After `timeout` the shell's process group is killed.
pub(crate) fn resolve_login_shell_path(shell: &Path, timeout: Duration) -> Option<OsString> {
    let nonce = format!(
        "{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default()
    );
    let begin = format!("__MADE_PATH_BEGIN_{nonce}__");
    let end = format!("__MADE_PATH_END_{nonce}__");
    // printf and printenv behave the same in zsh, bash, sh and fish; printenv
    // prints the exported, colon-joined value.
    let script = format!("printf '%s' '{begin}'; /usr/bin/printenv PATH; printf '%s' '{end}'");
    let output_path = env::temp_dir().join(format!("made-login-path-{nonce}"));
    let mut output = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&output_path)
        .ok()?;
    let _ = std::fs::remove_file(&output_path);
    let bytes = output.try_clone().ok().and_then(|stdout| {
        run_probe(shell, &script, stdout, timeout)?;
        output.seek(SeekFrom::Start(0)).ok()?;
        let mut bytes = Vec::new();
        output
            .take(PROBE_OUTPUT_LIMIT)
            .read_to_end(&mut bytes)
            .ok()?;
        Some(bytes)
    })?;
    parse_marked_path(&bytes, begin.as_bytes(), end.as_bytes())
}

/// Run the probe and wait for it; `Some` when it exits in time, whatever its
/// exit status.
fn run_probe(shell: &Path, script: &str, output: std::fs::File, timeout: Duration) -> Option<()> {
    let mut command = Command::new(shell);
    command
        .arg("-ilc")
        .arg(script)
        .env(RESOLVING_SHELL_ENV, "1")
        .stdin(Stdio::null())
        .stdout(output)
        .stderr(Stdio::null());
    // A new session: no controlling terminal, so an interactive shell started
    // from a terminal cannot stop on SIGTTIN/SIGTTOU, and the child still leads
    // its own process group for `kill_child_and_group`.
    // SAFETY: setsid is async-signal-safe and touches no parent state.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Some(()),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => {
                kill_child_and_group(&mut child);
                return None;
            }
        }
    }
}

/// The value between the first `begin` marker and the next `end` marker,
/// without the trailing newline `printenv` adds. `None` when a marker is
/// missing or the value is empty, not UTF-8, or spans lines.
pub(crate) fn parse_marked_path(output: &[u8], begin: &[u8], end: &[u8]) -> Option<OsString> {
    let find = |haystack: &[u8], needle: &[u8]| {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    };
    if begin.is_empty() || end.is_empty() {
        return None;
    }
    let rest = &output[find(output, begin)? + begin.len()..];
    let value = std::str::from_utf8(&rest[..find(rest, end)?]).ok()?;
    let value = value.strip_suffix('\n').unwrap_or(value);
    if value.is_empty() || value.contains(['\n', '\0']) {
        return None;
    }
    Some(OsString::from(value))
}

#[cfg(test)]
mod login_shell_path_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// [`resolve_login_shell_path`] holding [`crate::FORK_TEST_LOCK`]: its
    /// `pre_exec` spawn forks.
    fn resolve(shell: &Path, timeout: Duration) -> Option<OsString> {
        let _fork = crate::FORK_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        resolve_login_shell_path(shell, timeout)
    }

    fn fake_shell(dir: &Path, body: &str) -> PathBuf {
        let shell = dir.join("fake-shell");
        std::fs::write(&shell, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        shell
    }

    #[test]
    fn parses_the_path_between_markers_despite_noise() {
        let output = b"motd noise\nBEGIN/a/bin:/usr/bin\nEND trailing noise\n";
        assert_eq!(
            parse_marked_path(output, b"BEGIN", b"END"),
            Some(OsString::from("/a/bin:/usr/bin"))
        );
        assert_eq!(
            parse_marked_path(b"BEGIN/a/bin\nENDEND", b"BEGIN", b"END"),
            Some(OsString::from("/a/bin"))
        );
    }

    #[test]
    fn rejects_missing_markers_and_empty_values() {
        assert_eq!(parse_marked_path(b"/a/bin END", b"BEGIN", b"END"), None);
        assert_eq!(parse_marked_path(b"BEGIN/a/bin", b"BEGIN", b"END"), None);
        assert_eq!(parse_marked_path(b"END/a/binBEGIN", b"BEGIN", b"END"), None);
        assert_eq!(parse_marked_path(b"BEGINEND", b"BEGIN", b"END"), None);
        assert_eq!(parse_marked_path(b"BEGIN\nEND", b"BEGIN", b"END"), None);
        assert_eq!(
            parse_marked_path(b"BEGIN/a\n/b\nEND", b"BEGIN", b"END"),
            None
        );
        assert_eq!(
            parse_marked_path(b"BEGIN\xff\xfeEND", b"BEGIN", b"END"),
            None
        );
        assert_eq!(parse_marked_path(b"", b"BEGIN", b"END"), None);
    }

    #[test]
    fn resolves_only_from_the_launchd_default_path() {
        assert!(should_resolve_login_shell_path(None, false));
        assert!(should_resolve_login_shell_path(Some(OsStr::new("")), false));
        assert!(should_resolve_login_shell_path(
            Some(OsStr::new(LAUNCHD_DEFAULT_PATH)),
            false
        ));
        assert!(!should_resolve_login_shell_path(
            Some(OsStr::new(LAUNCHD_DEFAULT_PATH)),
            true
        ));
        assert!(!should_resolve_login_shell_path(
            Some(OsStr::new("/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin")),
            false
        ));
    }

    #[test]
    fn login_shell_requires_an_absolute_executable() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), "exit 0");
        assert_eq!(login_shell(Some(shell.as_os_str())), shell);
        assert_eq!(login_shell(Some(OsStr::new("zsh"))), Path::new("/bin/zsh"));
        assert_eq!(login_shell(None), Path::new("/bin/zsh"));
        let missing = dir.path().join("missing");
        assert_eq!(
            login_shell(Some(missing.as_os_str())),
            Path::new("/bin/zsh")
        );
    }

    #[test]
    fn resolves_the_path_a_shell_exports_past_startup_noise() {
        let dir = tempfile::tempdir().unwrap();
        // $1 is -ilc, $2 the probe command.
        let shell = fake_shell(
            dir.path(),
            "echo rc noise\n[ \"$MADE_RESOLVING_SHELL_ENV\" = 1 ] || exit 1\n\
             PATH=/fake/bin:/usr/bin; export PATH\neval \"$2\"\necho more noise",
        );
        assert_eq!(
            resolve(&shell, Duration::from_secs(5)),
            Some(OsString::from("/fake/bin:/usr/bin"))
        );
    }

    #[test]
    fn failing_shell_keeps_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let failing = fake_shell(dir.path(), "echo partial\nexit 3");
        assert_eq!(resolve(&failing, Duration::from_secs(5)), None);
        let silent = fake_shell(dir.path(), "echo no markers");
        assert_eq!(resolve(&silent, Duration::from_secs(5)), None);
        let missing = dir.path().join("missing-shell");
        assert_eq!(resolve(&missing, Duration::from_secs(5)), None);
    }

    #[test]
    fn marked_path_is_kept_despite_a_non_zero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(
            dir.path(),
            "PATH=/fake/bin:/usr/bin; export PATH\neval \"$2\"\nexit 3",
        );
        assert_eq!(
            resolve(&shell, Duration::from_secs(5)),
            Some(OsString::from("/fake/bin:/usr/bin"))
        );
    }

    #[test]
    fn appends_missing_fallback_dirs_after_path() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".grok/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let program = bin.join("made-fallback-probe");
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let local = home.path().join(".local/bin");
        let path = env::join_paths([Path::new("/usr/bin"), &local, Path::new("/bin")]).unwrap();

        assert_eq!(
            crate::find_executable_on_path(OsStr::new("made-fallback-probe"), &path),
            None
        );
        let extended = with_fallback_dirs(&path, Some(home.path().as_os_str()));
        assert_eq!(
            crate::find_executable_on_path(OsStr::new("made-fallback-probe"), &extended),
            Some(program)
        );
        // PATH keeps its order; fallbacks it already holds are not repeated.
        let entries: Vec<PathBuf> = env::split_paths(&extended).collect();
        assert_eq!(
            entries,
            [
                PathBuf::from("/usr/bin"),
                local,
                PathBuf::from("/bin"),
                home.path().join(".opencode/bin"),
                home.path().join(".grok/bin"),
                home.path().join(".cargo/bin"),
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/usr/local/bin"),
            ]
        );
    }

    #[test]
    fn relative_or_missing_home_adds_only_homebrew_prefixes() {
        for home in [
            None,
            Some(OsStr::new("")),
            Some(OsStr::new("relative/home")),
        ] {
            let entries: Vec<PathBuf> =
                env::split_paths(&with_fallback_dirs(OsStr::new("/usr/bin"), home)).collect();
            assert_eq!(
                entries,
                [
                    PathBuf::from("/usr/bin"),
                    PathBuf::from("/opt/homebrew/bin"),
                    PathBuf::from("/usr/local/bin"),
                ]
            );
        }
    }

    #[test]
    fn base_falls_back_to_the_launchd_default_path() {
        let resolved = Some(OsString::from("/shell/bin:/usr/bin"));
        let inherited = Some(OsString::from("/inherited/bin"));
        assert_eq!(
            login_path_base(resolved.clone(), inherited.clone()),
            OsString::from("/shell/bin:/usr/bin")
        );
        assert_eq!(
            login_path_base(None, inherited),
            OsString::from("/inherited/bin")
        );
        assert_eq!(
            login_path_base(None, Some(OsString::new())),
            OsString::from(LAUNCHD_DEFAULT_PATH)
        );
        assert_eq!(
            login_path_base(None, None),
            OsString::from(LAUNCHD_DEFAULT_PATH)
        );
    }

    #[test]
    fn home_with_a_separator_keeps_the_homebrew_prefixes() {
        let entries: Vec<PathBuf> = env::split_paths(&with_fallback_dirs(
            OsStr::new("/usr/bin"),
            Some(OsStr::new("/Users/odd:name")),
        ))
        .collect();
        assert_eq!(
            entries,
            [
                PathBuf::from("/usr/bin"),
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/usr/local/bin"),
            ]
        );
    }

    #[test]
    fn hung_shell_is_killed_at_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        // The background sleep stays in the shell's process group.
        let shell = fake_shell(dir.path(), "sleep 30 &\neval \"$2\"\nsleep 30");
        let started = Instant::now();
        assert_eq!(resolve(&shell, Duration::from_millis(300)), None);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    /// Manual check against the real login shell. Run the test binary under
    /// `env -i HOME=$HOME SHELL=$SHELL PATH=/usr/bin:/bin:/usr/sbin:/sbin`
    /// with `--ignored`. Prints only which of the program directories the
    /// resolved `PATH` contains, never the value.
    #[test]
    #[ignore = "depends on the user's login shell"]
    fn reports_program_directories_from_the_real_login_shell() {
        let shell = login_shell(env::var_os("SHELL").as_deref());
        let path = resolve(&shell, LOGIN_SHELL_TIMEOUT).expect("resolved PATH");
        let home = PathBuf::from(env::var_os("HOME").expect("HOME"));
        let entries: Vec<PathBuf> = env::split_paths(&path).collect();
        for relative in [".local/bin", ".opencode/bin", ".grok/bin"] {
            println!(
                "~/{relative} in resolved PATH: {}",
                entries.contains(&home.join(relative))
            );
        }
        for program in ["claude", "opencode", "grok"] {
            println!(
                "{program} found: {}",
                crate::find_executable_on_path(OsStr::new(program), &path).is_some()
            );
        }
    }
}
