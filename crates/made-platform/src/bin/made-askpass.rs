//! No diagnostic output: prompts and answers must never enter copied logs.
fn main() {
    #[cfg(unix)]
    if std::env::args_os().nth(1).is_some_and(|a| a == "cwd-report") {
        let result = (|| {
            let socket = std::env::args_os().nth(2).ok_or(())?;
            let path = std::env::args_os().nth(3).ok_or(())?;
            made_platform::shell_integration::emit_cwd_report(std::path::Path::new(&socket), path.as_encoded_bytes()).map_err(|_| ())
        })();
        if result.is_err() { std::process::exit(1); }
        return;
    }
    let result = (|| {
        let socket = std::env::var_os("MADE_ASKPASS_SOCKET").ok_or(())?;
        let prompt = std::env::args().nth(1).ok_or(())?;
        made_platform::ssh_askpass::run_helper(
            std::path::Path::new(&socket),
            &prompt,
            &mut std::io::stdout().lock(),
        )
        .map_err(|_| ())
    })();
    if result.is_err() {
        std::process::exit(1);
    }
}
