//! The packaged `made-open` helper against a live request sink.
#![cfg(unix)]

use made_platform::open_url::{
    UrlRequest, UrlRequestSink, UrlToken, HELPER_ACK_TIMEOUT, URL_SOCKET_ENV, URL_TOKEN_ENV,
};
use std::{
    io::Write,
    os::unix::net::UnixStream,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn helper(socket: Option<&Path>, token: Option<&str>, args: &[&str]) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_made-open"));
    command
        .args(args)
        .env_remove(URL_SOCKET_ENV)
        .env_remove(URL_TOKEN_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(socket) = socket {
        command.env(URL_SOCKET_ENV, socket);
    }
    if let Some(token) = token {
        command.env(URL_TOKEN_ENV, token);
    }
    command.spawn().unwrap()
}

fn next_request(sink: &mut UrlRequestSink) -> UrlRequest {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(request) = sink.poll().into_iter().next() {
            return request;
        }
        assert!(Instant::now() < deadline, "no request arrived");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn finish(child: Child) -> (bool, Vec<u8>) {
    let output = child.wait_with_output().unwrap();
    let mut printed = output.stdout;
    printed.extend(output.stderr);
    (output.status.success(), printed)
}

#[test]
fn helper_exits_zero_only_on_ack_and_prints_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let mut sink = UrlRequestSink::new(temp.path()).unwrap();
    let token = UrlToken::generate().unwrap();
    let child = helper(
        Some(sink.socket_path()),
        Some(token.as_str()),
        &["https://example.com/login?state=1"],
    );
    let request = next_request(&mut sink);
    assert!(token.matches(request.token()));
    assert_eq!(request.url().as_str(), "https://example.com/login?state=1");
    assert_eq!(request.url().host(), "example.com");
    request.accept();
    let (ok, printed) = finish(child);
    assert!(ok, "an acknowledged request exits 0");
    assert!(printed.is_empty(), "the helper never prints");
}

#[test]
fn helper_exits_non_zero_on_refusal_and_on_ack_timeout() {
    let temp = tempfile::tempdir().unwrap();
    let mut sink = UrlRequestSink::new(temp.path()).unwrap();
    let token = UrlToken::generate().unwrap();
    let refused = helper(
        Some(sink.socket_path()),
        Some(token.as_str()),
        &["https://example.com/"],
    );
    next_request(&mut sink).reject();
    assert!(!finish(refused).0);

    // A dropped request closes the connection: also a refusal.
    let dropped = helper(
        Some(sink.socket_path()),
        Some(token.as_str()),
        &["https://example.com/"],
    );
    drop(next_request(&mut sink));
    assert!(!finish(dropped).0);

    // Nobody polls: the helper gives up at its one-second budget.
    let started = Instant::now();
    let unanswered = helper(
        Some(sink.socket_path()),
        Some(token.as_str()),
        &["https://example.com/"],
    );
    let (ok, printed) = finish(unanswered);
    assert!(!ok);
    assert!(printed.is_empty());
    let elapsed = started.elapsed();
    assert!(elapsed >= HELPER_ACK_TIMEOUT - Duration::from_millis(50));
    assert!(elapsed < HELPER_ACK_TIMEOUT + Duration::from_secs(2));
}

#[test]
fn helper_exits_non_zero_without_environment_socket_or_valid_input() {
    let temp = tempfile::tempdir().unwrap();
    let mut sink = UrlRequestSink::new(temp.path()).unwrap();
    let token = UrlToken::generate().unwrap();
    let url = "https://example.com/";
    let missing_socket = temp.path().join("absent.sock");
    let cases = [
        helper(None, None, &[url]),
        helper(None, Some(token.as_str()), &[url]),
        helper(Some(sink.socket_path()), None, &[url]),
        helper(Some(&missing_socket), Some(token.as_str()), &[url]),
        helper(Some(sink.socket_path()), Some(token.as_str()), &[]),
        helper(Some(sink.socket_path()), Some(token.as_str()), &[url, url]),
        helper(
            Some(sink.socket_path()),
            Some(token.as_str()),
            &["file:///etc/passwd"],
        ),
        helper(
            Some(sink.socket_path()),
            Some(token.as_str()),
            &["javascript:alert(1)"],
        ),
        helper(Some(sink.socket_path()), Some("not-a-token"), &[url]),
    ];
    for child in cases {
        assert!(!finish(child).0);
    }
    // None of the refused invocations reached the sink.
    assert!(sink.poll().is_empty());
}

#[test]
fn sink_drops_malformed_and_oversized_frames() {
    let temp = tempfile::tempdir().unwrap();
    let mut sink = UrlRequestSink::new(temp.path()).unwrap();
    let token = UrlToken::generate().unwrap();
    let frame = |body: &[u8]| {
        let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(body);
        bytes
    };
    let bodies = [
        frame(b"garbage"),
        frame(format!("{}\nfile:///etc/passwd", token.as_str()).as_bytes()),
        frame(format!("{}\nhttps://a.example/\u{202e}", token.as_str()).as_bytes()),
        (u32::MAX).to_be_bytes().to_vec(),
        frame(&vec![b'a'; 64 * 1024]),
    ];
    let mut streams = Vec::new();
    for bytes in bodies {
        let mut stream = UnixStream::connect(sink.socket_path()).unwrap();
        stream.set_nonblocking(true).unwrap();
        let _ = stream.write_all(&bytes);
        streams.push(stream);
    }
    let deadline = Instant::now() + Duration::from_millis(700);
    while Instant::now() < deadline {
        assert!(sink.poll().is_empty());
        std::thread::sleep(Duration::from_millis(10));
    }
    // A well-formed request still gets through afterwards.
    let child = helper(
        Some(sink.socket_path()),
        Some(token.as_str()),
        &["https://example.com/ok"],
    );
    let request = next_request(&mut sink);
    assert_eq!(request.url().as_str(), "https://example.com/ok");
    request.accept();
    assert!(finish(child).0);
}

#[test]
fn sink_endpoint_is_owner_only_and_removed_on_drop() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let sink = UrlRequestSink::new(temp.path()).unwrap();
    let path = sink.socket_path().to_path_buf();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let parent = path.parent().unwrap().to_path_buf();
    let dir_mode = std::fs::metadata(&parent).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700);
    drop(sink);
    assert!(!path.exists());
    assert!(!parent.exists());
}

#[test]
fn a_long_parent_falls_back_to_the_temp_root() {
    let temp = tempfile::tempdir().unwrap();
    let long = temp.path().join("x".repeat(120));
    std::fs::create_dir_all(&long).unwrap();
    let mut sink = UrlRequestSink::new(&long).unwrap();
    assert!(sink.socket_path().as_os_str().len() <= 103);
    let token = UrlToken::generate().unwrap();
    let child = helper(
        Some(sink.socket_path()),
        Some(token.as_str()),
        &["https://example.com/"],
    );
    next_request(&mut sink).accept();
    assert!(finish(child).0);
}
