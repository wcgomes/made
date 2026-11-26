//! macOS PTY child spawn: no `fork(2)`, same session, terminal, descriptor and
//! signal setup as before.
//!
//! A forked child of a multithreaded process runs libSystem's atfork handlers,
//! which can trap (`os_once_t is corrupt`, `Illegal instruction: 4` on Intel)
//! when another thread was inside a one-time libSystem initialization at the
//! fork. `posix_spawn` runs no atfork handlers.
//!
//! Keep this the only test in this binary: it changes process-wide state (an
//! atfork handler, an ignored SIGINT, descriptor 77) and counts every fork in
//! the process, so another test here could fork or inherit that state.
#![cfg(target_os = "macos")]

use std::ffi::OsString;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use made_platform::{host_platform, Platform, PtyRequest, PtySize};

static FORKS: AtomicUsize = AtomicUsize::new(0);

extern "C" fn count_fork() {
    FORKS.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn pty_child_is_spawned_without_fork_and_keeps_session_tty_fd_and_signal_setup() {
    // SAFETY: the handler only increments an atomic.
    assert_eq!(
        unsafe { libc::pthread_atfork(Some(count_fork), None, None) },
        0
    );
    // Inherited state the child must not keep: an ignored SIGINT and a
    // descriptor without close-on-exec.
    // SAFETY: plain signal disposition and descriptor calls.
    let leaked = unsafe {
        assert_ne!(libc::signal(libc::SIGINT, libc::SIG_IGN), libc::SIG_ERR);
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY);
        assert!(null >= 0);
        let leaked = libc::dup2(null, 77);
        libc::close(null);
        leaked
    };
    assert_eq!(leaked, 77);

    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().canonicalize().unwrap();
    let script = "printf 'pid=%s\\n' $$; \
                  printf 'tpgid=%s\\n' $(ps -o tpgid= -p $$); \
                  printf 'tty=%s\\n' \"$(tty)\"; \
                  printf 'fd77=%s\\n' \"$(ls /dev/fd | grep -cx 77)\"; \
                  printf 'cwd=%s\\n' \"$PWD\"; \
                  printf 'END\\n'; read _; kill -INT $$; printf 'survived\\n'";
    let request = PtyRequest {
        program: OsString::from("/bin/sh"),
        args: vec![OsString::from("-c"), OsString::from(script)],
        cwd: cwd.clone(),
        env: vec![(OsString::from("PATH"), OsString::from("/usr/bin:/bin"))],
        env_remove: Vec::new(),
        size: PtySize::default(),
    };
    let forks_before = FORKS.load(Ordering::SeqCst);
    let session = host_platform()
        .spawn_pty(&request)
        .expect("spawn PTY child");
    assert_eq!(
        FORKS.load(Ordering::SeqCst),
        forks_before,
        "spawning the PTY child must not fork"
    );
    // SAFETY: closes the descriptor this test opened.
    unsafe { libc::close(leaked) };

    let mut reader = session.take_reader().unwrap();
    let mut writer = session.take_writer().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        while let Ok(read) = reader.read(&mut buffer) {
            if read == 0 || tx.send(buffer[..read].to_vec()).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut output = String::new();
    while !output.contains("END") {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let chunk = rx
            .recv_timeout(remaining)
            .unwrap_or_else(|_| panic!("child report never finished: {output:?}"));
        output.push_str(&String::from_utf8_lossy(&chunk));
    }
    let field = |name: &str| {
        output
            .lines()
            .find_map(|line| line.trim_end().strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("no {name} in {output:?}"))
            .trim()
            .to_owned()
    };
    let pid = session.pid().expect("child pid").to_string();
    assert_eq!(field("pid"), pid, "{output}");
    let child = session.pid().unwrap() as libc::pid_t;
    // SAFETY: getsid only reads the child's session id.
    assert_eq!(
        unsafe { libc::getsid(child) },
        child,
        "the child leads its own session"
    );
    assert_eq!(
        field("tpgid"),
        pid,
        "the PTY is the controlling terminal with the child in the foreground"
    );
    assert!(field("tty").starts_with("/dev/ttys"), "{output}");
    assert_eq!(field("fd77"), "0", "only stdio is inherited");
    assert_eq!(field("cwd"), cwd.display().to_string());

    writer.write_all(b"\n").unwrap();
    writer.flush().unwrap();
    let status = loop {
        if let Some(status) = session.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "child did not exit");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        status.signal.as_deref(),
        Some("Interrupt: 2"),
        "SIGINT is reset to its default action: {status:?}"
    );
}
