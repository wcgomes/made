use made_bridge::remote_rpc::{RpcConnection, RpcListener};
use std::{
    io::Write,
    net::{Ipv4Addr, SocketAddr, TcpStream},
    thread,
    time::Duration,
};
#[test]
fn loopback_auth_rejects_foreign_capability_before_state() {
    let listener = RpcListener::bind([7; 32], [3; 16]).unwrap();
    let addr = listener.local_addr().unwrap();
    assert!(addr.ip().is_loopback());
    let mut client = TcpStream::connect(addr).unwrap();
    client.write_all(&[9; 32]).unwrap();
    client.write_all(&[3; 16]).unwrap();
    client.write_all(&1u64.to_be_bytes()).unwrap();
    let stream = loop {
        if let Some(stream) = listener.accept().unwrap() {
            break stream;
        }
        thread::sleep(Duration::from_millis(1));
    };
    assert!(listener.authenticate(stream).is_err());
}
#[test]
fn authenticated_frame_roundtrip_is_bounded_and_epoch_bound() {
    let listener = RpcListener::bind([7; 32], [3; 16]).unwrap();
    let addr = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let stream = loop {
            if let Some(stream) = listener.accept().unwrap() {
                break stream;
            }
            thread::sleep(Duration::from_millis(1));
        };
        let mut connection = listener.authenticate(stream).unwrap();
        assert_eq!(connection.epoch(), 4);
        assert_eq!(connection.read_frame().unwrap(), b"hello");
        connection.write_frame(b"reply").unwrap();
    });
    let mut client = RpcConnection::connect(addr, &[7; 32], &[3; 16], 4).unwrap();
    client.write_frame(b"hello").unwrap();
    assert_eq!(client.read_frame().unwrap(), b"reply");
    worker.join().unwrap();
    let wildcard = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 1234));
    assert!(RpcConnection::connect(wildcard, &[7; 32], &[3; 16], 1).is_err());
}
