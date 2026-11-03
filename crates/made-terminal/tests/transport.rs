use made_platform::{host_platform, PtySize};
use made_terminal::{
    LocalPtyTransport, PtyError, PtyEvent, PtyOptions, PtyProcess, TerminalTransport,
    TransportStatus, WRITE_CHUNK_MAX,
};
use std::time::{Duration, Instant};

fn local() -> LocalPtyTransport {
    let platform = host_platform();
    let options = PtyOptions::new(std::env::temp_dir()).with_write_queue_capacity(1);
    LocalPtyTransport::new(PtyProcess::spawn(&platform, options).unwrap())
}

#[test]
fn local_detach_releases_only_its_child_and_discards_input_and_output() {
    let mut one = local();
    let sibling = local();
    let transport: &mut dyn TerminalTransport = &mut one;
    transport.detach().unwrap();
    assert_eq!(transport.status(), TransportStatus::Disconnected);
    assert!(matches!(
        transport.write(b"must not execute"),
        Err(PtyError::Closed)
    ));
    assert!(transport.poll().is_none());
    assert!(matches!(transport.resize(30, 100), Err(PtyError::Closed)));
    transport.detach().unwrap();
    assert_eq!(sibling.status(), TransportStatus::Connected);
    assert!(one.process().is_none());
}

#[test]
fn local_adapter_preserves_bounded_write_resize_and_real_output() {
    let mut transport = local();
    assert!(matches!(
        transport.write(&vec![0; WRITE_CHUNK_MAX + 1]),
        Err(PtyError::WouldBlock)
    ));
    transport.resize(31, 99).unwrap();
    assert_eq!(
        transport.process().unwrap().size().unwrap(),
        PtySize { rows: 31, cols: 99 }
    );
    transport.write(b"echo MADE_TRANSPORT_PARITY\r\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = Vec::new();
    while Instant::now() < deadline {
        if let Some(PtyEvent::Data(bytes)) = transport.poll() {
            // Answer Windows' console startup cursor-position query too.
            if bytes.windows(4).any(|w| w == b"\x1b[6n") {
                let _ = transport.write(b"\x1b[1;1R");
            }
            output.extend(bytes);
            if output.windows(21).any(|w| w == b"MADE_TRANSPORT_PARITY\r")
                || String::from_utf8_lossy(&output).contains("MADE_TRANSPORT_PARITY")
            {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(String::from_utf8_lossy(&output).contains("MADE_TRANSPORT_PARITY"));
    transport.detach().unwrap();
}
