//! UI-free companion executable. stdout is exclusively framed protocol.
use made_bridge::{config, config_guard::ConfigSnapshot, remote_rpc::RpcConnection};
use made_platform::{Environment, PathsInputs, Platform, PlatformFamily, PtyRequest};
use made_remote::{
    filesystem::native,
    integration::{resolve_config, Integration},
    protocol::*,
    supervisor::{self, read_envelope, write_envelope, ChildLaunch, Endpoint, Supervisor},
};
use serde::Deserialize;
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

fn main() {
    if run().is_err() {
        eprintln!(
            "made-remote: operation failed; check prerequisites, consent and selected session"
        );
        std::process::exit(1);
    }
}
fn fail() -> io::Error {
    io::Error::other("remote operation unavailable")
}
fn convert<T>(result: Result<T, impl std::fmt::Debug>) -> io::Result<T> {
    result.map_err(|_| fail())
}
fn paths() -> io::Result<made_platform::AppPaths> {
    convert(made_platform::AppPaths::resolve(&PathsInputs::from_env(
        PlatformFamily::current(),
    )))
}
fn find(name: &str) -> io::Result<PathBuf> {
    let path = Environment::current()
        .get("PATH")
        .map(OsStr::to_owned)
        .ok_or_else(fail)?;
    made_platform::find_executable_on_path(OsStr::new(name), &path).ok_or_else(fail)
}
fn capabilities() -> io::Result<Capabilities> {
    let target = match convert(made_platform::remote_target())?.as_str() {
        "x86_64-unknown-linux-gnu" => RemoteTarget::LinuxX64,
        "aarch64-unknown-linux-gnu" => RemoteTarget::LinuxArm64,
        "x86_64-apple-darwin" => RemoteTarget::MacosX64,
        "aarch64-apple-darwin" => RemoteTarget::MacosArm64,
        _ => return Err(fail()),
    };
    // OpenCode is optional and bounded. tmux is not probed, version-checked,
    // or required; Capabilities.tmux stays on the wire and is always false.
    let opencode = find("opencode").is_ok_and(|path| {
        made_platform::remote_program_version_supported(&path, "opencode").unwrap_or(false)
    });
    Ok(Capabilities {
        target,
        tmux: false,
        opencode,
        shell_cwd: false,
        helper_version: env!("CARGO_PKG_VERSION").into(),
    })
}
fn run() -> io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref().and_then(OsStr::to_str) {
        Some("--probe") => {
            let first = read_envelope(&mut io::stdin().lock())?;
            if first.operation != Operation::Capabilities {
                return Err(fail());
            }
            let operation = match capabilities() {
                Ok(capabilities) => Operation::CapabilitiesResult { capabilities },
                Err(_) => Operation::Error {
                    code: ErrorCode::Unsupported,
                },
            };
            write_envelope(
                &mut io::stdout().lock(),
                &WireEnvelope::new(first.identity, first.request_id, operation),
            )
        }
        Some("--stdio") => frontend(),
        Some("--supervisor") => daemon(),
        Some("--child") => child(&PathBuf::from(args.next().ok_or_else(fail)?)),
        Some("cwd-report") => {
            let socket = PathBuf::from(args.next().ok_or_else(fail)?);
            let path = PathBuf::from(args.next().ok_or_else(fail)?);
            let bytes = convert(made_platform::remote_path_bytes(&path))?;
            made_platform::shell_integration::emit_cwd_report(&socket, &bytes)
        }
        Some("--stage") => stage(),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected --probe, --stdio, --supervisor, --child or --stage",
        )),
    }
}
fn frontend() -> io::Result<()> {
    let mut stdin = io::stdin().lock();
    let first = read_envelope(&mut stdin)?;
    let paths = paths()?;
    let runtime = convert(supervisor::runtime_directory(
        &paths.data_dir,
        &first.identity,
    ))?;
    match first.operation {
        Operation::Open { .. } => {
            // Absence or presence of tmux does not change this step.
            let _ = capabilities()?;
            // Atomic create, never restart/replace a lost existing session.
            convert(made_platform::ensure_private_directory(
                &paths.data_dir.join("remote/runtime"),
            ))?;
            convert(made_platform::create_private_directory_new(&runtime))?;
            let executable = std::env::current_exe()?;
            // Real detach API. `detach` is a private module; this re-export
            // starts a new session, ignores SIGHUP, and closes inherited stdio.
            // The open frame is the stdin payload, never argv.
            convert(made_platform::spawn_detached_command(
                &executable,
                &[OsString::from("--supervisor")],
                &encode(&first).map_err(|_| fail())?,
            ))?;
        }
        Operation::Attach { .. } => {}
        _ => return Err(fail()),
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    let endpoint: Endpoint = loop {
        // A private endpoint can contain only a bounded loopback locator, never
        // command text. Following symlinks is rejected by the platform reader.
        match made_platform::read_private_metadata(&runtime, "endpoint.json", 4096) {
            Ok(Some(bytes)) => break serde_json::from_slice(&bytes).map_err(|_| fail())?,
            Ok(None) | Err(_)
                if Instant::now() < deadline
                    && matches!(first.operation, Operation::Open { .. }) =>
            {
                thread::sleep(Duration::from_millis(10))
            }
            Ok(None) | Err(_) => {
                return Err(io::Error::other(
                    "session unavailable or expired; explicit replacement required",
                ))
            }
        }
    };
    let connection = RpcConnection::connect(
        endpoint.addr,
        &convert(supervisor::capability_bytes(&first.identity))?,
        &first.identity.authority.0,
        first.identity.epoch,
    )?;
    let mut stream = connection.into_stream();
    stream.set_read_timeout(None)?;
    write_envelope(&mut stream, &first)?;
    let mut output = stream.try_clone()?;
    let reader = thread::spawn(move || io::copy(&mut output, &mut io::stdout().lock()));
    // Raw framing is verified at the authenticated supervisor. No retry/replay.
    let result = io::copy(&mut stdin, &mut stream);
    let _ = stream.shutdown(std::net::Shutdown::Both);
    let _ = reader.join();
    result.map(|_| ())
}
fn daemon() -> io::Result<()> {
    let first = read_envelope(&mut io::stdin().lock())?;
    let Operation::Open {
        directory, target, ..
    } = &first.operation
    else {
        return Err(fail());
    };
    let paths = paths()?;
    let runtime = convert(supervisor::runtime_directory(
        &paths.data_dir,
        &first.identity,
    ))?;
    let platform = made_platform::host_platform();
    let shell = convert(platform.resolve_shell(None))?;
    let program = match target {
        LaunchTarget::Shell => PathBuf::from(&shell.program),
        LaunchTarget::OpenCode => find("opencode")?,
    };
    let inputs = PathsInputs::from_env(PlatformFamily::current());
    let home = inputs.home.ok_or_else(fail)?;
    let v2 = inputs
        .xdg_config_home
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
        .join("opencode/cli.json");
    let user_config = resolve_config(v2, paths.opencode_config_path.clone());
    // Failure of optional plugin composition leaves terminal/fs available.
    let integration = Integration::start(
        &paths.plugin_dir,
        &paths.plugin_root,
        user_config,
        config::inline_config_from_env(),
        "remote".into(),
    )
    .ok();
    let mut env = platform.environment(&[], &[]).with_terminal_defaults();
    if let Some(i) = &integration {
        let (key, value) = i.environment();
        env.set(key, value);
    }
    let launch = ChildLaunch {
        program,
        args: if *target == LaunchTarget::Shell {
            shell.args
        } else {
            vec![]
        },
        cwd: convert(native(directory))?,
        env: env.into_vec(),
    };
    let supervisor = convert(Supervisor::create(
        &first,
        runtime,
        &paths.data_dir,
        &std::env::current_exe()?,
        launch,
        integration,
    ))?;
    convert(supervisor.run())
}
fn child(state: &Path) -> io::Result<()> {
    let (directory, name) = state
        .parent()
        .zip(state.file_name().and_then(|name| name.to_str()))
        .ok_or_else(fail)?;
    let bytes = convert(made_platform::read_private_metadata(
        directory,
        name,
        1024 * 1024,
    ))?
    .ok_or_else(fail)?;
    let launch: ChildLaunch = serde_json::from_slice(&bytes).map_err(|_| fail())?;
    let mut request = PtyRequest::new(launch.program.as_os_str(), launch.cwd);
    request.args = launch.args;
    request.env = launch.env;
    // Platform exec replaces this wrapper; no shell-command concatenation.
    convert(made_platform::exec_remote_child(&request))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StageManifest {
    consent: bool,
    version: String,
    target: RemoteTarget,
    files: Vec<Artifact>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    name: String,
    size: u64,
    sha256: String,
}
const ARTIFACTS: &[&str] = &[
    "made-remote",
    "plugins/opencode-made/package.json",
    "plugins/opencode-made/tui.ts",
    "plugins/opencode-made/index.ts",
    "plugins/opencode-made/src/protocol.ts",
    "plugins/opencode-made/src/integration.ts",
];
fn stage() -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut n = [0; 4];
    input.read_exact(&mut n)?;
    let n = u32::from_be_bytes(n) as usize;
    if n == 0 || n > MAX_FRAME_BYTES {
        return Err(fail());
    }
    let mut bytes = vec![0; n];
    input.read_exact(&mut bytes)?;
    let manifest: StageManifest = serde_json::from_slice(&bytes).map_err(|_| fail())?;
    if !manifest.consent
        || manifest.target != capabilities()?.target
        || manifest.version.is_empty()
        || manifest.version.len() > 128
        || !manifest
            .version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        || manifest.files.len() != ARTIFACTS.len()
    {
        return Err(fail());
    }
    let mut seen = std::collections::HashSet::new();
    for file in &manifest.files {
        if !ARTIFACTS.contains(&file.name.as_str())
            || !seen.insert(&file.name)
            || file.size == 0
            || file.size > MAX_UPLOAD_BYTES
            || file.sha256.len() != 64
            || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(fail());
        }
    }
    let paths = paths()?;
    let root = paths.data_dir.join("remote").join(&manifest.version);
    convert(made_platform::ensure_private_directory(&root))?;
    let name = match manifest.target {
        RemoteTarget::LinuxX64 => "x86_64-unknown-linux-gnu",
        RemoteTarget::LinuxArm64 => "aarch64-unknown-linux-gnu",
        RemoteTarget::MacosX64 => "x86_64-apple-darwin",
        RemoteTarget::MacosArm64 => "aarch64-apple-darwin",
    };
    let destination = root.join(name);
    if destination.exists() {
        for file in manifest.files {
            let snapshot = ConfigSnapshot::capture(destination.join(file.name))?;
            if snapshot.sha256_hex().as_deref() != Some(file.sha256.as_str()) {
                return Err(fail());
            }
        }
        return Ok(());
    }
    let random = convert(TabCapability::random())?;
    let suffix =
        made_bridge::Token::from_bytes(convert(supervisor::capability_bytes(&Identity {
            authority: AuthorityId([0; 16]),
            tab: random,
            epoch: 1,
            generation: 1,
        }))?)
        .fingerprint()
        .replace(':', "-");
    let staging = root.join(format!(".stage-{suffix}"));
    convert(made_platform::create_private_directory_new(&staging))?;
    let result = (|| {
        for file in &manifest.files {
            let path = staging.join(&file.name);
            if let Some(parent) = path.parent() {
                convert(made_platform::ensure_private_directory(parent))?;
            }
            let mut output = convert(made_platform::create_private_file_new(
                &path,
                file.name == "made-remote",
            ))?;
            let copied = io::copy(&mut input.by_ref().take(file.size), &mut output)?;
            if copied != file.size {
                return Err(fail());
            }
            output.sync_all()?;
            let snapshot = ConfigSnapshot::capture(&path)?;
            if snapshot.sha256_hex().as_deref() != Some(file.sha256.as_str()) {
                return Err(fail());
            }
        }
        // No existing version is overwritten; serialize staging/promotion.
        let _lock = convert(made_platform::lock_private_file(
            &root.join("stage.lock"),
            Duration::from_secs(1),
        ))?;
        if destination.exists() {
            return Err(fail());
        }
        fs::rename(&staging, &destination)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}
