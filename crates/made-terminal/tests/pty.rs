//! PTY adapter tests. They run the host's real default shell through the
//! platform layer and use shell commands common to POSIX shells and cmd.exe:
//! `echo`, redirection, and `exit`.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(target_os = "linux")]
use made_terminal::{InputEncoder, InputModes};

use made_platform::{
    host_platform, Clipboard, Environment, ExitStatus, Platform, PlatformError, PlatformFamily,
    PtyRequest, PtySession, PtySize, ShellSpec,
};
use made_terminal::{PtyError, PtyEvent, PtyOptions, PtyProcess, TerminalCore, WRITE_CHUNK_MAX};

static UNIQUE: AtomicU64 = AtomicU64::new(0);

fn assert_send<T: Send>() {}

#[test]
fn core_and_pty_are_sendable_across_threads() {
    assert_send::<TerminalCore>();
    assert_send::<PtyProcess>();
}

fn unique_temp(label: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "made-pty-test-{label}-{}-{nanos}-{}",
        std::process::id(),
        UNIQUE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).unwrap();
    #[cfg(unix)]
    let path = path.canonicalize().unwrap();
    path
}

fn options(cwd: &std::path::Path) -> PtyOptions {
    PtyOptions::new(cwd)
        .with_size(PtySize { rows: 24, cols: 80 })
        .with_channel_capacity(64)
}

/// Build one shell command line. ConPTY needs a line terminator on the input
/// pipe; this whole-line CRLF submission is distinct from the single `\r` the
/// product sends for a physical Enter key. POSIX shells accept LF.
fn command(text: &str) -> Vec<u8> {
    let mut bytes = text.as_bytes().to_vec();
    bytes.extend_from_slice(if cfg!(windows) { b"\r\n" } else { b"\n" });
    bytes
}

/// Answer the console host's cursor-position query. Windows conhost and
/// PowerShell wait for the reply before accepting the next command.
fn answer_console_dsr(pty: &mut PtyProcess, bytes: &[u8], carry: &mut Vec<u8>) -> usize {
    let mut scan = Vec::with_capacity(carry.len() + bytes.len());
    scan.extend_from_slice(carry);
    scan.extend_from_slice(bytes);
    let requests = scan
        .windows(4)
        .filter(|window| *window == b"\x1b[6n")
        .count();
    for _ in 0..requests {
        let _ = pty.write(b"\x1b[1;1R");
    }
    *carry = scan[scan.len().saturating_sub(3)..].to_vec();
    requests
}

/// Drain immediately queued PTY events while answering DSR queries. Returns
/// whether the terminal event stream ended and how many queries were answered.
/// `poll` repeats `Exited` after the child is reaped, so terminal events must
/// end the drain loop instead of being ignored.
fn drain_console_events(pty: &mut PtyProcess, carry: &mut Vec<u8>) -> (bool, usize) {
    let mut requests = 0;
    while let Some(event) = pty.poll() {
        match event {
            PtyEvent::Data(bytes) => requests += answer_console_dsr(pty, &bytes, carry),
            PtyEvent::Exited(_)
            | PtyEvent::Eof
            | PtyEvent::ReadError(_)
            | PtyEvent::MonitorError(_) => return (true, requests),
        }
    }
    (false, requests)
}

/// Wait for Windows conhost/PowerShell to issue its startup cursor-position
/// query before sending commands, so the reply is not queued behind input.
/// A cold PowerShell can take several seconds to reach its first prompt.
#[cfg(windows)]
fn settle_console(pty: &mut PtyProcess) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut carry = Vec::new();
    loop {
        let (ended, requests) = drain_console_events(pty, &mut carry);
        if requests > 0 {
            return;
        }
        assert!(
            !ended,
            "console host exited before issuing its cursor-position query"
        );
        assert!(
            Instant::now() < deadline,
            "console host did not issue a cursor-position query within 5s"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(not(windows))]
fn settle_console(_pty: &mut PtyProcess) {}

fn read_until(
    pty: &mut PtyProcess,
    output: &mut Vec<u8>,
    needle: &str,
    occurrences: usize,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    let mut carry = Vec::new();
    while Instant::now() < deadline {
        if String::from_utf8_lossy(output).matches(needle).count() >= occurrences {
            return true;
        }
        match pty.recv_timeout(Duration::from_millis(100)) {
            Ok(PtyEvent::Data(bytes)) => {
                let _ = answer_console_dsr(pty, &bytes, &mut carry);
                output.extend_from_slice(&bytes);
            }
            Ok(
                PtyEvent::Eof
                | PtyEvent::ReadError(_)
                | PtyEvent::Exited(_)
                | PtyEvent::MonitorError(_),
            ) => break,
            Err(_) => {
                if pty.exit_status().is_some() && pty.poll().is_none() {
                    break;
                }
            }
        }
    }
    String::from_utf8_lossy(output).matches(needle).count() >= occurrences
}

fn wait_for_file(pty: &mut PtyProcess, path: &std::path::Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let mut carry = Vec::new();
    loop {
        if path.is_file() {
            return true;
        }
        // A repeated `Exited` poll must end the wait rather than livelock.
        let (ended, _) = drain_console_events(pty, &mut carry);
        if ended {
            return path.is_file();
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_exit(pty: &mut PtyProcess, timeout: Duration) -> Result<ExitStatus, PtyError> {
    let deadline = Instant::now() + timeout;
    let mut carry = Vec::new();
    loop {
        while let Some(event) = pty.poll() {
            match event {
                PtyEvent::Data(bytes) => {
                    let _ = answer_console_dsr(pty, &bytes, &mut carry);
                }
                PtyEvent::Exited(status) => return Ok(status),
                PtyEvent::MonitorError(message) => return Err(PtyError::Monitor(message)),
                PtyEvent::Eof | PtyEvent::ReadError(_) => {}
            }
        }
        if let Some(status) = pty.exit_status() {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(PtyError::Timeout);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn pty_spawns_shell_with_cwd_echo_resize_and_exit_event() {
    let root = unique_temp("shell");
    std::fs::create_dir_all(&root).expect("create cwd");
    let platform = host_platform();
    let mut pty = PtyProcess::spawn(&platform, options(&root)).expect("spawn");
    settle_console(&mut pty);
    assert!(pty.is_running());
    assert!(pty.pid().is_some());
    assert_eq!(pty.size().expect("size"), PtySize { rows: 24, cols: 80 });

    pty.write(&command("echo made-pty-ok")).expect("write echo");
    let mut output = Vec::new();
    assert!(
        read_until(
            &mut pty,
            &mut output,
            "made-pty-ok",
            2,
            Duration::from_secs(5)
        ),
        "no echo output: {}",
        String::from_utf8_lossy(&output)
    );

    let marker = root.join("made-cwd-probe.txt");
    pty.write(&command("echo cwd-ok > made-cwd-probe.txt"))
        .expect("write cwd probe");
    assert!(
        wait_for_file(&mut pty, &marker, Duration::from_secs(5)),
        "shell did not run in the requested cwd: {}",
        root.display()
    );

    pty.resize(31, 101).expect("resize");
    assert_eq!(
        pty.size().expect("resized size"),
        PtySize {
            rows: 31,
            cols: 101
        }
    );

    pty.write(&command("exit")).expect("exit");
    let status = wait_for_exit(&mut pty, Duration::from_secs(5)).expect("exit status");
    assert!(status.success(), "unexpected status: {status}");
    assert!(!pty.is_running());
    assert!(pty.exit_status().is_some());

    let mut saw_exit_event = false;
    for _ in 0..100 {
        match pty.poll() {
            Some(PtyEvent::Exited(status)) => {
                assert!(status.success());
                saw_exit_event = true;
                break;
            }
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    assert!(saw_exit_event, "poll must surface the child-exit event");

    drop(pty);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn pty_terminate_reaps_the_child() {
    let root = unique_temp("terminate");
    std::fs::create_dir_all(&root).expect("create cwd");
    let platform = host_platform();
    let mut pty = PtyProcess::spawn(&platform, options(&root)).expect("spawn");
    assert!(pty.is_running());

    let status = pty.terminate().expect("terminate");
    assert!(!pty.is_running());
    assert!(pty.exit_status().is_some());
    let _ = status;

    drop(pty);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn pty_can_be_relaunched_after_exit() {
    let root = unique_temp("relaunch");
    std::fs::create_dir_all(&root).expect("create cwd");
    let platform = host_platform();
    let mut pty = PtyProcess::spawn(&platform, options(&root)).expect("spawn");
    settle_console(&mut pty);

    pty.write(&command("exit")).expect("exit first shell");
    wait_for_exit(&mut pty, Duration::from_secs(5)).expect("first exit");
    assert!(!pty.is_running());

    pty.relaunch(&platform).expect("relaunch");
    settle_console(&mut pty);
    assert!(pty.is_running());
    assert!(pty.pid().is_some());

    pty.write(&command("echo relaunched-ok")).expect("write");
    let mut output = Vec::new();
    assert!(
        read_until(
            &mut pty,
            &mut output,
            "relaunched-ok",
            2,
            Duration::from_secs(5)
        ),
        "no relaunched output: {}",
        String::from_utf8_lossy(&output)
    );

    pty.terminate().expect("terminate relaunched shell");
    drop(pty);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn pty_options_are_cloneable_and_carry_the_target_shell() {
    let root = unique_temp("options");
    let options = options(&root).with_env("MADE_PTY_TEST", "1");
    let clone = options.clone();
    assert_eq!(clone.cwd, root);
    assert_eq!(clone.size, PtySize { rows: 24, cols: 80 });
    assert!(clone
        .env
        .iter()
        .any(|(key, value)| key == "MADE_PTY_TEST" && value == "1"));
    assert_eq!(PlatformFamily::current(), host_platform().family());
}

#[test]
fn queued_writes_are_delivered_in_order() {
    let root = unique_temp("write-order");
    std::fs::create_dir_all(&root).expect("create cwd");
    let platform = host_platform();
    let mut pty = PtyProcess::spawn(&platform, options(&root)).expect("spawn");
    settle_console(&mut pty);

    pty.write(&command("echo ORDER-ONE >> order.txt"))
        .expect("first ordered write");
    pty.write(&command("echo ORDER-TWO >> order.txt"))
        .expect("second ordered write");

    let order_file = root.join("order.txt");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut carry = Vec::new();
    let mut contents;
    loop {
        contents = std::fs::read_to_string(&order_file).unwrap_or_default();
        if contents.contains("ORDER-TWO") {
            break;
        }
        // PowerShell can issue DSR queries after the commands were queued;
        // answer them so the shell is not blocked waiting for a reply.
        let (ended, _) = drain_console_events(&mut pty, &mut carry);
        if ended || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let contents = std::fs::read_to_string(&order_file).unwrap_or_default();
    let first = contents
        .find("ORDER-ONE")
        .expect("first queued command ran");
    let second = contents
        .find("ORDER-TWO")
        .expect("second queued command ran");
    assert!(first < second, "queued writes were reordered: {contents:?}");

    pty.terminate().expect("terminate");
    drop(pty);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn wait_for_file_returns_after_the_shell_exits() {
    let root = unique_temp("exit-before-file");
    std::fs::create_dir_all(&root).expect("create cwd");
    let missing = root.join("never-created.txt");
    let platform = host_platform();
    let mut pty = PtyProcess::spawn(&platform, options(&root)).expect("spawn");
    settle_console(&mut pty);
    pty.write(&command("exit")).expect("exit");

    let (sender, receiver) = std::sync::mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let found = wait_for_file(&mut pty, &missing, Duration::from_secs(5));
        let _ = sender.send(found);
    });
    let found = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("wait_for_file hung after the shell exited");
    assert!(!found, "the shell exited before creating the file");
    waiter.join().expect("waiter thread");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(target_os = "linux")]
fn spawn_bash(cwd: &std::path::Path) -> PtyProcess {
    PtyProcess::spawn(
        &host_platform(),
        PtyOptions::new(cwd)
            .with_shell("/bin/bash")
            .with_args(["--noprofile", "--norc", "-i"])
            .with_channel_capacity(64),
    )
    .expect("spawn interactive bash")
}

#[cfg(target_os = "linux")]
fn prepare_bash_bracketed_paste(pty: &mut PtyProcess) {
    pty.write(b"bind 'set enable-bracketed-paste on'\necho PASTE_READY\n")
        .expect("enable bracketed paste");
    let mut output = Vec::new();
    assert!(
        read_until(pty, &mut output, "PASTE_READY", 2, Duration::from_secs(5)),
        "interactive bash did not become ready: {}",
        String::from_utf8_lossy(&output)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn bracketed_paste_reconstruction_cannot_execute_injected_command() {
    let root = unique_temp("paste-injection");
    std::fs::create_dir_all(&root).expect("create cwd");
    let marker = root.join("injected-fixed-ran");
    let attack = format!("\x1b[20\x1b[201~1~\rtouch '{}'\r", marker.display());
    let encoder = InputEncoder::new(InputModes {
        bracketed_paste: true,
        ..InputModes::default()
    });

    // Sanitized path: the bracketed paste must contain the exact review
    // reconstruction payload without executing its CR-separated command.
    let mut pty = spawn_bash(&root);
    prepare_bash_bracketed_paste(&mut pty);
    let encoded = encoder.encode_paste(&attack);
    pty.write(&encoded).expect("write sanitized paste");
    std::thread::sleep(Duration::from_millis(700));
    assert!(
        !marker.exists(),
        "sanitized bracketed paste executed the injected command"
    );
    let _ = pty.write(b"\x03");
    let _ = pty.write(b"exit\n");
    let _ = pty.terminate();
    drop(pty);

    // Control: the same CR-separated command with an unsafe early terminator
    // must execute, proving the harness would catch a bypass.
    let control_marker = root.join("injected-unsafe-ran");
    let control_attack = format!("\x1b[20\x1b[201~1~\rtouch '{}'\r", control_marker.display());
    let mut control_pty = spawn_bash(&root);
    prepare_bash_bracketed_paste(&mut control_pty);
    let mut raw = Vec::new();
    raw.extend_from_slice(b"\x1b[200~");
    raw.extend_from_slice(control_attack.as_bytes());
    raw.extend_from_slice(b"\x1b[201~");
    control_pty.write(&raw).expect("write unsafe control paste");

    let deadline = Instant::now() + Duration::from_secs(3);
    while !control_marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        control_marker.exists(),
        "control payload did not execute; the integration harness is not exercising bracketed paste"
    );

    let _ = control_pty.write(b"\x03");
    let _ = control_pty.write(b"exit\n");
    let _ = control_pty.terminate();
    drop(control_pty);
    let _ = std::fs::remove_dir_all(&root);
}

#[derive(Default)]
struct BlockingState {
    writer_started: bool,
    release_writer: bool,
    killed: bool,
}

type BlockingShared = Arc<(Mutex<BlockingState>, Condvar)>;

struct BlockingPlatform {
    state: BlockingShared,
}

struct BlockingSession {
    state: BlockingShared,
}

struct BlockingWriter {
    state: BlockingShared,
}

impl Platform for BlockingPlatform {
    fn family(&self) -> PlatformFamily {
        PlatformFamily::current()
    }

    fn resolve_shell(&self, _requested: Option<&OsStr>) -> Result<ShellSpec, PlatformError> {
        Ok(ShellSpec::new("/fake/shell"))
    }

    fn environment(
        &self,
        _overrides: &[(OsString, OsString)],
        _removed: &[OsString],
    ) -> Environment {
        Environment::from_pairs([("PATH", "/bin")])
    }

    fn app_paths(&self) -> Result<made_platform::AppPaths, PlatformError> {
        Err(PlatformError::message("not used"))
    }

    fn clipboard(&self) -> Result<Box<dyn Clipboard>, PlatformError> {
        Err(PlatformError::message("not used"))
    }

    fn spawn_pty(&self, _request: &PtyRequest) -> Result<Box<dyn PtySession>, PlatformError> {
        Ok(Box::new(BlockingSession {
            state: self.state.clone(),
        }))
    }
}

impl PtySession for BlockingSession {
    fn take_reader(&self) -> Result<Box<dyn Read + Send>, PlatformError> {
        Ok(Box::new(io::empty()))
    }

    fn take_writer(&self) -> Result<Box<dyn Write + Send>, PlatformError> {
        Ok(Box::new(BlockingWriter {
            state: self.state.clone(),
        }))
    }

    fn resize(&self, _size: PtySize) -> Result<(), PlatformError> {
        Ok(())
    }

    fn size(&self) -> Result<PtySize, PlatformError> {
        Ok(PtySize::default())
    }

    fn try_wait(&self) -> Result<Option<ExitStatus>, PlatformError> {
        let (lock, _) = &*self.state;
        let state = lock.lock().unwrap_or_else(|error| error.into_inner());
        Ok(state.killed.then_some(ExitStatus {
            code: 0,
            signal: None,
        }))
    }

    fn kill(&self) -> Result<(), PlatformError> {
        let (lock, ready) = &*self.state;
        let mut state = lock.lock().unwrap_or_else(|error| error.into_inner());
        state.killed = true;
        state.release_writer = true;
        drop(state);
        ready.notify_all();
        Ok(())
    }

    fn pid(&self) -> Option<u32> {
        Some(4242)
    }
}

impl Write for BlockingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        let (lock, ready) = &*self.state;
        let mut state = lock.lock().unwrap_or_else(|error| error.into_inner());
        state.writer_started = true;
        ready.notify_all();
        while !state.release_writer {
            state = ready.wait(state).unwrap_or_else(|error| error.into_inner());
        }
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "blocked writer released",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn nonblocking_write_reports_would_block_while_writer_is_blocked() {
    let root = unique_temp("write-block");
    let shared: BlockingShared = Arc::new((Mutex::new(BlockingState::default()), Condvar::new()));
    let platform = BlockingPlatform {
        state: shared.clone(),
    };
    let mut pty = PtyProcess::spawn(
        &platform,
        PtyOptions::new(&root)
            .with_write_queue_capacity(1)
            .with_size(PtySize::default()),
    )
    .expect("spawn fake PTY");

    pty.write(&[b'a'; WRITE_CHUNK_MAX])
        .expect("first chunk accepted");

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let (lock, _) = &*shared;
        if lock
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .writer_started
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "writer thread never reached the blocking write"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    // The writer holds the first chunk. One more chunk fills the queue; the
    // third cannot fit and must be rejected without touching the PTY.
    pty.write(&[b'b'; WRITE_CHUNK_MAX])
        .expect("second chunk queues");
    let started = Instant::now();
    let error = pty
        .write(&[b'c'; WRITE_CHUNK_MAX])
        .expect_err("full queue must reject the write");
    assert!(
        matches!(error, PtyError::WouldBlock),
        "unexpected error: {error:?}"
    );
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "write blocked instead of reporting WouldBlock"
    );

    let drop_started = Instant::now();
    drop(pty);
    assert!(
        drop_started.elapsed() < Duration::from_secs(5),
        "shutdown hung with a blocked writer"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn debug_output_redacts_bridge_secrets_and_output_chunks() {
    let token = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
    let config = format!(
        r#"{{"plugin":[{{"package":"made-opencode","options":{{"made":{{"token":"{token}"}}}}}}]}}"#
    );
    let root = unique_temp("redaction");
    std::fs::create_dir_all(&root).expect("create cwd");

    let options = options(&root)
        .with_args(["--token", token])
        .with_env("OPENCODE_CLI_CONFIG_CONTENT", config.as_str())
        .with_env("MADE_BRIDGE_TOKEN", token)
        .with_env("PATH", "/usr/bin:/bin");

    let options_debug = format!("{options:?}");
    assert!(
        !options_debug.contains(token),
        "token leaked through PtyOptions: {options_debug}"
    );
    assert!(
        !options_debug.contains(&config),
        "config content leaked through PtyOptions"
    );
    assert!(!options_debug.contains("--token"));
    assert!(options_debug.contains("<redacted>"));
    assert!(!format!("{options}").contains(token));

    let pty = PtyProcess::spawn(&host_platform(), options).expect("spawn");
    let process_debug = format!("{pty:?}");
    assert!(
        !process_debug.contains(token),
        "token leaked through PtyProcess: {process_debug}"
    );
    assert!(!process_debug.contains(&config));
    assert!(process_debug.contains("<redacted>"));
    assert!(!format!("{pty}").contains(token));
    drop(pty);

    let event = PtyEvent::Data(b"agent-output-secret".to_vec());
    let event_debug = format!("{event:?}");
    assert!(!event_debug.contains("agent-output-secret"));
    assert!(event_debug.contains("bytes"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn monitor_failure_is_surfaced_instead_of_reporting_a_running_child() {
    use std::ffi::{OsStr, OsString};
    use std::io::{Read, Write};
    use std::sync::atomic::AtomicBool;

    use made_platform::{
        AppPaths, Clipboard, Environment, ExitStatus, PlatformError, PtyRequest, PtySession,
        ShellSpec,
    };
    use made_terminal::PtyError;

    type EofGate = Arc<(Mutex<bool>, Condvar)>;

    struct FailingMonitorPlatform {
        eof_gate: EofGate,
        eof_returned: Arc<AtomicBool>,
    }

    impl Platform for FailingMonitorPlatform {
        fn family(&self) -> PlatformFamily {
            PlatformFamily::current()
        }

        fn resolve_shell(&self, _requested: Option<&OsStr>) -> Result<ShellSpec, PlatformError> {
            Ok(ShellSpec::new("/bin/sh"))
        }

        fn environment(
            &self,
            _overrides: &[(OsString, OsString)],
            _removed: &[OsString],
        ) -> Environment {
            Environment::from_pairs([("PATH", "/bin")])
        }

        fn app_paths(&self) -> Result<AppPaths, PlatformError> {
            Err(PlatformError::message("not used"))
        }

        fn clipboard(&self) -> Result<Box<dyn Clipboard>, PlatformError> {
            Err(PlatformError::message("not used"))
        }

        fn spawn_pty(&self, _request: &PtyRequest) -> Result<Box<dyn PtySession>, PlatformError> {
            Ok(Box::new(FailingMonitorSession {
                eof_gate: self.eof_gate.clone(),
                eof_returned: self.eof_returned.clone(),
            }))
        }
    }

    struct FailingMonitorSession {
        eof_gate: EofGate,
        eof_returned: Arc<AtomicBool>,
    }

    struct GatedEofReader {
        gate: EofGate,
        returned: Arc<AtomicBool>,
    }

    impl Read for GatedEofReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            let (lock, ready) = &*self.gate;
            let mut open = lock.lock().unwrap_or_else(|error| error.into_inner());
            while !*open {
                open = ready.wait(open).unwrap_or_else(|error| error.into_inner());
            }
            self.returned.store(true, Ordering::SeqCst);
            Ok(0)
        }
    }

    impl PtySession for FailingMonitorSession {
        fn take_reader(&self) -> Result<Box<dyn Read + Send>, PlatformError> {
            Ok(Box::new(GatedEofReader {
                gate: self.eof_gate.clone(),
                returned: self.eof_returned.clone(),
            }))
        }

        fn take_writer(&self) -> Result<Box<dyn Write + Send>, PlatformError> {
            Ok(Box::new(std::io::sink()))
        }

        fn resize(&self, _size: PtySize) -> Result<(), PlatformError> {
            Ok(())
        }

        fn size(&self) -> Result<PtySize, PlatformError> {
            Ok(PtySize::default())
        }

        fn try_wait(&self) -> Result<Option<ExitStatus>, PlatformError> {
            Err(PlatformError::message("injected monitor failure"))
        }

        fn kill(&self) -> Result<(), PlatformError> {
            Ok(())
        }

        fn pid(&self) -> Option<u32> {
            Some(4242)
        }
    }

    let eof_gate: EofGate = Arc::new((Mutex::new(false), Condvar::new()));
    let eof_returned = Arc::new(AtomicBool::new(false));
    let pty = PtyProcess::spawn(
        &FailingMonitorPlatform {
            eof_gate: eof_gate.clone(),
            eof_returned: eof_returned.clone(),
        },
        PtyOptions::new("/work"),
    )
    .expect("spawn fake PTY");

    let deadline = Instant::now() + Duration::from_secs(2);
    let error = loop {
        if let Some(error) = pty.monitor_error() {
            break error;
        }
        assert!(
            Instant::now() < deadline,
            "monitor failure was never surfaced"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(error.contains("injected monitor failure"));

    // Hold reader EOF until after the monitor failure is visible, then let it
    // land. This deterministically creates the reported race: a buffered Eof
    // must not hide a failed monitor.
    {
        let (lock, ready) = &*eof_gate;
        *lock.lock().unwrap_or_else(|error| error.into_inner()) = true;
        ready.notify_all();
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while !eof_returned.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "reader EOF was never released");
        std::thread::sleep(Duration::from_millis(5));
    }
    // Give the reader thread time to enqueue Eof on the bounded channel.
    std::thread::sleep(Duration::from_millis(20));

    assert!(
        matches!(
            pty.recv_timeout(Duration::from_millis(100)),
            Err(PtyError::Monitor(message)) if message.contains("injected")
        ),
        "a buffered Eof must be reported as a monitor failure"
    );
    assert!(
        !pty.is_running(),
        "a monitor failure must not be reported as a running child"
    );
    assert!(matches!(
        pty.try_is_running(),
        Err(PtyError::Monitor(message)) if message.contains("injected")
    ));
    assert!(matches!(
        pty.wait_timeout(Some(Duration::from_millis(50))),
        Err(PtyError::Monitor(_))
    ));

    let mut saw_monitor_error = false;
    for _ in 0..100 {
        match pty.poll() {
            Some(PtyEvent::MonitorError(message)) => {
                assert!(message.contains("injected"));
                saw_monitor_error = true;
                break;
            }
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    assert!(
        saw_monitor_error,
        "poll must surface PtyEvent::MonitorError"
    );
    assert!(matches!(
        pty.recv_timeout(Duration::from_millis(50)),
        Err(PtyError::Monitor(_))
    ));
}
#[test]
fn real_bash_reports_initial_and_cd_preserving_rc_hooks_and_relaunch_cleanup() {
    if std::process::Command::new("bash")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("PENDING: bash unavailable");
        return;
    }
    for hook in [
        "PROMPT_COMMAND='printf USER_HOOK'",
        "PROMPT_COMMAND=('printf USER_HOOK' 'true')",
    ] {
        let root = unique_temp("cwd");
        let target = root.join("spaces é 日本");
        std::fs::create_dir_all(&target).unwrap();
        let rc = format!("export MADE_RC_VALUE=kept\nPS1='USER_PROMPT> '\n{hook}\n");
        std::fs::write(root.join(".bashrc"), &rc).unwrap();
        let mut opts = options(&root);
        opts.shell = Some("bash".into());
        opts.env
            .push(("HOME".into(), root.clone().into_os_string()));
        opts.env.push(("MADE_ENV_VALUE".into(), "preserved".into()));
        let platform = host_platform();
        let mut pty = PtyProcess::spawn(&platform, opts).unwrap();
        if !pty.shell_cwd_capability().is_supported() {
            eprintln!(
                "PENDING (not passed): {}",
                pty.shell_cwd_capability().reason
            );
            pty.terminate().unwrap();
            drop(pty);
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
        let bootstrap = pty.bootstrap_directory().unwrap().to_owned();
        let mut output = Vec::new();
        assert!(
            read_until(&mut pty, &mut output, "\x1b]7;", 1, Duration::from_secs(5)),
            "{}",
            String::from_utf8_lossy(&output)
        );
        assert!(read_until(
            &mut pty,
            &mut output,
            "USER_PROMPT>",
            1,
            Duration::from_secs(5)
        ));
        let mut core = TerminalCore::new();
        core.feed(&output);
        assert_eq!(core.cwd(), Some(root.as_path()));
        assert!(String::from_utf8_lossy(&output).contains("USER_HOOK"));
        assert!(String::from_utf8_lossy(&output).contains("USER_PROMPT>"));
        output.clear();
        pty.write(
            "cd 'spaces é 日本'\nprintf 'VALUES:%s:%s\\n' \"$MADE_RC_VALUE\" \"$MADE_ENV_VALUE\"\n"
                .as_bytes(),
        )
        .unwrap();
        assert!(
            read_until(&mut pty, &mut output, "\x1b]7;", 2, Duration::from_secs(5)),
            "{}",
            String::from_utf8_lossy(&output)
        );
        core.feed(&output);
        assert_eq!(core.cwd(), Some(target.as_path()));
        assert!(String::from_utf8_lossy(&output).contains("VALUES:kept:preserved"));
        assert!(bootstrap.exists());
        assert_eq!(std::fs::read(root.join(".bashrc")).unwrap(), rc.as_bytes());
        pty.terminate().unwrap();
        // Relaunch replaces this launch's resource owner only.
        pty.relaunch(&platform).unwrap();
        assert_ne!(pty.bootstrap_directory(), Some(bootstrap.as_path()));
        assert!(!bootstrap.exists());
        let next_bootstrap = pty.bootstrap_directory().unwrap().to_owned();
        assert!(next_bootstrap.exists());
        pty.terminate().unwrap();
        drop(pty);
        assert!(!next_bootstrap.exists());
        assert_eq!(std::fs::read(root.join(".bashrc")).unwrap(), rc.as_bytes());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn bash_prompt_command_array_keeps_every_hook_in_order() {
    // macOS /bin/bash is 3.2.57, where PROMPT_COMMAND arrays execute only the
    // first element. MADE_TEST_BASH32 lets a local run exercise both a real
    // 3.2 shell and the platform bash (5.x on Linux).
    let mut candidates = vec![OsString::from("bash")];
    if let Some(path) = std::env::var_os("MADE_TEST_BASH32") {
        candidates.push(path);
    }

    for shell in candidates {
        let available = std::process::Command::new(&shell)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success());
        if !available {
            eprintln!(
                "PENDING (not passed): {} unavailable",
                shell.to_string_lossy()
            );
            continue;
        }

        let root = unique_temp("bash-hooks");
        std::fs::create_dir_all(&root).expect("create cwd");
        let markers = root.join("hooks.txt");
        let rc = r#"PS1='USER_PROMPT> '
PROMPT_COMMAND=('printf "FIRST\n" >> "$HOME/hooks.txt"' 'printf "SECOND\n" >> "$HOME/hooks.txt"')
"#;
        std::fs::write(root.join(".bashrc"), rc).expect("write bashrc");

        let mut opts = options(&root);
        opts.shell = Some(shell.clone());
        opts.env
            .push(("HOME".into(), root.clone().into_os_string()));
        let platform = host_platform();
        let mut pty = PtyProcess::spawn(&platform, opts).expect("spawn bash");
        let mut output = Vec::new();
        assert!(
            read_until(
                &mut pty,
                &mut output,
                "USER_PROMPT>",
                1,
                Duration::from_secs(5)
            ),
            "{}: no prompt: {}",
            shell.to_string_lossy(),
            String::from_utf8_lossy(&output)
        );
        assert!(
            String::from_utf8_lossy(&output).contains("\x1b]7;"),
            "{}: missing OSC 7: {}",
            shell.to_string_lossy(),
            String::from_utf8_lossy(&output)
        );

        let contents = std::fs::read_to_string(&markers).expect("hook markers");
        let observed: Vec<&str> = contents.lines().take(2).collect();
        assert_eq!(
            observed,
            vec!["FIRST", "SECOND"],
            "{}: hooks were reordered or swallowed: {contents:?}",
            shell.to_string_lossy()
        );

        pty.terminate().expect("terminate");
        drop(pty);
        let _ = std::fs::remove_dir_all(&root);
        eprintln!("PASSED bash hook order: {}", shell.to_string_lossy());
    }
}

struct CaptureLaunchPlatform {
    host: Box<dyn Platform>,
    requests: Mutex<Vec<PtyRequest>>,
}

impl Platform for CaptureLaunchPlatform {
    fn family(&self) -> PlatformFamily {
        self.host.family()
    }
    fn resolve_shell(&self, requested: Option<&OsStr>) -> Result<ShellSpec, PlatformError> {
        let mut spec = self.host.resolve_shell(requested)?;
        spec.args.push("--noprofile".into());
        Ok(spec)
    }
    fn environment(&self, overrides: &[(OsString, OsString)], removed: &[OsString]) -> Environment {
        self.host.environment(overrides, removed)
    }
    fn app_paths(&self) -> Result<made_platform::AppPaths, PlatformError> {
        self.host.app_paths()
    }
    fn clipboard(&self) -> Result<Box<dyn Clipboard>, PlatformError> {
        self.host.clipboard()
    }
    fn spawn_pty(&self, request: &PtyRequest) -> Result<Box<dyn PtySession>, PlatformError> {
        self.requests.lock().unwrap().push(request.clone());
        self.host.spawn_pty(request)
    }
}

#[test]
fn launch_boundary_keeps_resolved_arguments_environment_and_rebuilds_resources() {
    if std::process::Command::new("bash")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("PENDING: bash unavailable");
        return;
    }
    let platform = CaptureLaunchPlatform {
        host: Box::new(host_platform()),
        requests: Mutex::new(Vec::new()),
    };
    let root = unique_temp("launch-plan");
    std::fs::create_dir_all(&root).unwrap();
    let mut opts = options(&root);
    opts.shell = Some("bash".into());
    opts.args.push("-i".into());
    opts.env
        .push(("MADE_TEST_LAUNCH_ENV".into(), "preserved".into()));
    opts.env_remove.push("MADE_TEST_REMOVED_ENV".into());
    let mut pty = PtyProcess::spawn(&platform, opts.clone()).unwrap();
    if !pty.shell_cwd_capability().is_supported() {
        eprintln!(
            "PENDING (not passed): {}",
            pty.shell_cwd_capability().reason
        );
        pty.terminate().unwrap();
        drop(pty);
        std::fs::remove_dir_all(root).unwrap();
        return;
    }
    let old = pty.bootstrap_directory().unwrap().to_owned();
    pty.relaunch(&platform).unwrap();
    let requests = platform.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        assert_eq!(request.program, OsStr::new("bash"));
        assert_eq!(request.args[0], OsStr::new("--rcfile"));
        assert_eq!(
            &request.args[2..],
            &[OsString::from("--noprofile"), "-i".into(), "-i".into()]
        );
        assert_eq!(request.cwd, root);
        assert!(request
            .env
            .iter()
            .any(|(k, v)| k == "MADE_TEST_LAUNCH_ENV" && v == "preserved"));
        assert!(!request
            .env
            .iter()
            .any(|(k, _)| k == "MADE_TEST_REMOVED_ENV"));
    }
    assert_eq!(pty.options(), &opts);
    assert_ne!(requests[0].args[1], requests[1].args[1]);
    assert!(!old.exists());
    drop(requests);
    pty.terminate().unwrap();
    drop(pty);
    std::fs::remove_dir_all(root).unwrap();
}
