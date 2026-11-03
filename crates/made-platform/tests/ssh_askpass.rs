#![cfg(unix)]
use made_platform::ssh_askpass::{run_helper, AskpassBroker, PromptAnswer};
use std::time::{Duration, Instant};
fn prompt(broker: &mut AskpassBroker) -> made_platform::ssh_askpass::PromptRequest {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(p) = broker.poll() {
            return p;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn one_use_answers_repeated_mfa_and_cancellation_are_request_bound() {
    let temp = tempfile::tempdir().unwrap();
    let mut broker = AskpassBroker::new(temp.path(), Duration::from_secs(2)).unwrap();
    for text in [
        "Password:",
        "Verification code:",
        "Verification code again:",
    ] {
        let socket = broker.socket_path().to_owned();
        let thread = std::thread::spawn(move || {
            let mut out = Vec::new();
            run_helper(&socket, text, &mut out).unwrap();
            out
        });
        let p = prompt(&mut broker);
        assert_eq!(p.text, text);
        assert!(!format!("{p:?}").contains(text));
        assert!(broker
            .respond(
                p.id + 1,
                Some(PromptAnswer::new(b"wrong".to_vec()).unwrap())
            )
            .is_err());
        broker
            .respond(p.id, Some(PromptAnswer::new(b"one-use".to_vec()).unwrap()))
            .unwrap();
        assert!(broker
            .respond(
                p.id,
                Some(PromptAnswer::new(b"duplicate".to_vec()).unwrap())
            )
            .is_err());
        assert_eq!(thread.join().unwrap(), b"one-use\n");
    }
    let socket = broker.socket_path().to_owned();
    let thread = std::thread::spawn(move || run_helper(&socket, "Password:", &mut Vec::new()));
    let p = prompt(&mut broker);
    broker.cancel();
    assert!(broker.respond(p.id, None).is_err());
    assert!(thread.join().unwrap().is_err());
}
#[test]
fn packaged_helper_answers_the_broker_prompt() {
    let temp = tempfile::tempdir().unwrap();
    let mut broker = AskpassBroker::new(temp.path(), Duration::from_secs(5)).unwrap();
    let socket = broker.socket_path().to_owned();
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_made-askpass"))
        .env("MADE_ASKPASS_SOCKET", &socket)
        .arg("vscode@example's password:")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let prompt = prompt(&mut broker);
    assert_eq!(prompt.text, "vscode@example's password:");
    broker
        .respond(
            prompt.id,
            Some(PromptAnswer::new(b"plantsuite".to_vec()).unwrap()),
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"plantsuite\n");
}

#[test]
fn private_directory_and_redacted_answer() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let broker = AskpassBroker::new(temp.path(), Duration::from_secs(1)).unwrap();
    assert_eq!(
        std::fs::metadata(broker.socket_path().parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::symlink_metadata(broker.socket_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "askpass socket was published with a loose mode"
    );
    let answer = PromptAnswer::new(b"private-secret".to_vec()).unwrap();
    assert!(!format!("{answer:?}").contains("private-secret"));
    assert!(PromptAnswer::new(b"line\nbreak".to_vec()).is_err());
    let socket = broker.socket_path().to_owned();
    drop(broker);
    assert!(!socket.exists());
}
