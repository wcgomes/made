#![cfg(windows)]
use made_platform::open_url::{send_request, UrlRequestSink, UrlToken};
use std::time::{Duration, Instant};

#[test]
fn the_url_pipe_serves_sequential_requests_on_its_single_instance() {
    let temp = tempfile::tempdir().unwrap();
    let mut sink = UrlRequestSink::new(temp.path()).unwrap();
    let token = UrlToken::generate().unwrap();
    for url in ["https://example.com/first", "https://example.com/second"] {
        let socket = sink.socket_path().to_path_buf();
        let presented = token.as_str().to_owned();
        let client = std::thread::spawn(move || {
            send_request(&socket, &presented, url, Duration::from_secs(5))
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let request = loop {
            if let Some(request) = sink.poll().into_iter().next() {
                break request;
            }
            assert!(Instant::now() < deadline, "no request for {url}");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(token.matches(request.token()));
        assert_eq!(request.url().as_str(), url);
        request.accept();
        client.join().unwrap().expect("the request is acknowledged");
    }
}
