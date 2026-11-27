//! macOS detached spawn: no `fork(2)`, same detachment, descriptor, stdio,
//! signal and environment setup as the `pre_exec` path it replaces.
//!
//! A forked child of a multithreaded process runs libSystem's atfork handlers,
//! which can trap (`os_once_t is corrupt`) when another thread was inside a
//! one-time libSystem initialization at the fork. `posix_spawn` runs none.
//!
//! Keep this the only test in this binary: it changes process-wide state (an
//! atfork handler, descriptor 77, `TMUX` in the environment) and counts every
//! fork in the process.
#![cfg(target_os = "macos")]

use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use made_platform::spawn_detached_command;

static FORKS: AtomicUsize = AtomicUsize::new(0);

extern "C" fn count_fork() {
    FORKS.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn detached_child_is_spawned_without_fork_and_keeps_its_detachment() {
    // SAFETY: the handler only increments an atomic.
    assert_eq!(
        unsafe { libc::pthread_atfork(Some(count_fork), None, None) },
        0
    );
    // SAFETY: plain descriptor calls; descriptor 77 has no close-on-exec flag.
    let leaked = unsafe {
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY);
        assert!(null >= 0);
        let leaked = libc::dup2(null, 77);
        libc::close(null);
        leaked
    };
    assert_eq!(leaked, 77);
    // No other test runs in this binary, so nothing reads the environment
    // concurrently.
    std::env::set_var("TMUX", "/tmp/tmux-test,1,0");
    std::env::set_var("TMUX_PANE", "%1");

    let temp = tempfile::tempdir().unwrap();
    let temp_root = temp.path().canonicalize().unwrap();
    // The runtime directory goes under this test's temp directory, so the run
    // leaves nothing behind.
    let runtime = temp_root.join("detach");
    let report = temp_root.join("report");
    let script = r#"
read -r payload
if [ /dev/fd/1 -ef /dev/null ] && [ /dev/fd/2 -ef /dev/null ]; then stdio=null; else stdio=other; fi
{
  printf 'payload=%s\n' "$payload"
  printf 'pid=%s\n' $$
  if ( : </dev/tty ) 2>/dev/null; then echo ctty=yes; else echo ctty=no; fi
  printf 'stdio=%s\n' "$stdio"
  printf 'fd77=%s\n' "$(ls /dev/fd | grep -cx 77)"
  printf 'tmux=%s,%s\n' "${TMUX-unset}" "${TMUX_PANE-unset}"
  printf 'cwd=%s\n' "$(pwd -P)"
  kill -HUP $$
  echo sighup=survived
} > "$1.tmp"
mv "$1.tmp" "$1"
while [ ! -e "$1.release" ] && [ -e "$1" ]; do sleep 0.05; done
"#;
    let forks_before = FORKS.load(Ordering::SeqCst);
    spawn_detached_command(
        Path::new("/bin/sh"),
        &[
            OsString::from("-c"),
            OsString::from(script),
            OsString::from("sh"),
            report.clone().into_os_string(),
        ],
        b"made-detach-payload\n",
        &runtime,
    )
    .expect("spawn detached child");
    assert_eq!(
        FORKS.load(Ordering::SeqCst),
        forks_before,
        "spawning the detached child must not fork"
    );
    // SAFETY: closes the descriptor this test opened.
    unsafe { libc::close(leaked) };

    let deadline = Instant::now() + Duration::from_secs(10);
    while !report.exists() {
        assert!(Instant::now() < deadline, "detached child never reported");
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = std::fs::read_to_string(&report).unwrap();
    let field = |name: &str| {
        output
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("no {name} in {output:?}"))
            .to_owned()
    };
    assert_eq!(field("payload"), "made-detach-payload");
    assert_eq!(field("ctty"), "no", "no controlling terminal");
    assert_eq!(field("stdio"), "null", "stdout and stderr are /dev/null");
    assert_eq!(field("fd77"), "0", "only stdio is inherited");
    assert_eq!(
        field("tmux"),
        "unset,unset",
        "TMUX and TMUX_PANE are removed"
    );
    assert_eq!(
        Path::new(&field("cwd")),
        runtime,
        "the child runs in the given runtime directory"
    );
    assert_eq!(field("sighup"), "survived", "SIGHUP is ignored");
    // The child waits for the release file, so it is alive here.
    let pid: libc::pid_t = field("pid").parse().unwrap();
    // SAFETY: getsid only reads the process's session id.
    assert_eq!(
        unsafe { libc::getsid(pid) },
        pid,
        "the child leads its own session"
    );
    std::fs::write(report.with_extension("release"), b"").unwrap();
}
