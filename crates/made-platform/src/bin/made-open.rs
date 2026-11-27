//! `BROWSER` target for Made terminal children. Forwards the URL to the owning
//! pane for user confirmation and exits 0 only on its ack; any other outcome
//! exits non-zero so the caller prints the URL instead. Never opens anything
//! and never prints: the URL must not reach terminal output or logs.
fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if made_platform::open_url::run_helper(&args).is_err() {
        std::process::exit(1);
    }
}
