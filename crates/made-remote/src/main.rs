//! UI-free companion executable. stdout is exclusively framed protocol, with
//! one exception: the explicit `--install-grok-plugin` mode writes a single
//! bounded JSON report line to stdout.
use made_bridge::{config, config_guard::ConfigSnapshot, remote_rpc::RpcConnection};
use made_platform::{Environment, PathsInputs, Platform, PlatformFamily, PtyRequest};
use made_remote::{
    claude,
    filesystem::{native, remote, Filesystem},
    git::GitRunner,
    grok::{self, GrokFiles},
    integration::{resolve_config, Integration},
    launch,
    protocol::*,
    supervisor::{
        self, read_envelope, write_envelope, ChildLaunch, Endpoint, FollowUpShell, Supervisor,
    },
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
    // URL helper mode: `BROWSER` names a `made-open` link to this executable
    // because it cannot carry a mode argument. Silent: the child prints the
    // URL itself when this exits non-zero.
    let mut argv = std::env::args_os();
    let invoked_as_helper = argv.next().is_some_and(|argv0| {
        Path::new(&argv0).file_name()
            == Some(OsStr::new(made_platform::open_url::REMOTE_HELPER_NAME))
    });
    if invoked_as_helper {
        let args: Vec<OsString> = argv.collect();
        if made_platform::open_url::run_helper(&args).is_err() {
            std::process::exit(1);
        }
        return;
    }
    match run() {
        Ok(()) => {}
        // Fixed, sanitized update-required result. No path, hash or stderr is
        // echoed, and the caller can distinguish it from a generic failure.
        Err(error) if error.to_string() == UPDATE_REQUIRED => {
            eprintln!("{UPDATE_REQUIRED}");
            std::process::exit(74);
        }
        // The original was preserved at the fixed app-owned replacement path
        // after promotion and immediate restoration both failed.
        Err(error) if error.to_string() == RECOVERY_REQUIRED => {
            eprintln!("{RECOVERY_REQUIRED}");
            std::process::exit(75);
        }
        Err(_) => {
            eprintln!(
                "made-remote: operation failed; check prerequisites, consent and selected session"
            );
            std::process::exit(1);
        }
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
    let environment = Environment::current();
    if let Some(path) = environment.get("PATH") {
        if let Some(found) = made_platform::find_executable_on_path(OsStr::new(name), path) {
            return Ok(found);
        }
    }
    // The OpenCode installer commonly uses ~/.opencode/bin, which is not in
    // every login PATH. Check the fixed user-owned locations before failing.
    if let Some(home) = environment.get("HOME") {
        for relative in [".opencode/bin", ".local/bin", ".cargo/bin"] {
            let candidate = std::path::Path::new(&home).join(relative).join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(fail())
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
    // Grok is a metadata-only PATH detection: no spawn, no version check, no
    // plugin install from here (SSH-09/SSH-10). No interpreter is detected; the
    // frozen `Capabilities.python3` field is always encoded false.
    let environment = Environment::current();
    let grok = grok::detect_grok(&environment).is_some();
    Ok(Capabilities {
        target,
        tmux: false,
        opencode,
        shell_cwd: false,
        helper_version: env!("CARGO_PKG_VERSION").into(),
        grok,
        python3: false,
        watch_dirs: true,
        start_shell: true,
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
        // Explicit only: the per-server/version consent flow invokes this
        // mode; watchers and launches never do.
        Some("--install-grok-plugin") => install_mode(),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected --probe, --stdio, --supervisor, --child, --stage or --install-grok-plugin",
        )),
    }
}
fn frontend() -> io::Result<()> {
    let mut stdin = io::stdin().lock();
    // The first frame fixes the grant for this session. A screen-scoped
    // browse grant is served here and never reaches the tab path below.
    let payload = read_payload(&mut stdin)?.ok_or_else(fail)?;
    if let Ok(open) = serde_json::from_slice::<BrowseEnvelope>(&payload) {
        return browse(&open, &mut stdin, &mut io::stdout().lock());
    }
    let first: WireEnvelope = serde_json::from_slice(&payload).map_err(|_| fail())?;
    first.validate().map_err(|_| fail())?;
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
            // The open frame is the stdin payload, never argv. The supervisor's
            // cwd is one private directory under this user's data root, shared
            // by every supervisor and never removed.
            convert(made_platform::spawn_detached_command(
                &executable,
                &[OsString::from("--supervisor")],
                &encode(&first).map_err(|_| fail())?,
                &paths.data_dir.join("remote/detach"),
            ))?;
        }
        Operation::Attach { .. } => {}
        _ => return Err(fail()),
    }
    let deadline = Instant::now() + Duration::from_secs(120);
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
    let output = stream.try_clone()?;
    let reader = thread::spawn(move || {
        let stdout = io::stdout();
        relay(output, stdout.lock())
    });
    // Raw framing is verified at the authenticated supervisor. No retry/replay.
    let result = relay(&mut stdin, &mut stream);
    let _ = stream.shutdown(std::net::Shutdown::Both);
    let _ = reader.join();
    result
}

/// Copy bytes without relying on `io::copy`. Its socket/pipe specialization
/// uses `splice(2)`; on the container kernels used by remote workspaces a
/// spliced frame can sit in the stdout pipe without waking the SSH channel
/// reader, so the client waits forever for a frame that was already consumed.
/// A buffered read/write loop is portable and always issues a real write.
fn relay(mut input: impl io::Read, mut output: impl io::Write) -> io::Result<()> {
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = match input.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(read) => read,
            // Reads carry no timeout (it is cleared before the relay starts),
            // so only an interrupted read is retried; any other error ends
            // the relay instead of spinning.
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        write_all_retry(&mut output, &buffer[..read], RELAY_STALL_LIMIT)?;
        // `Stdout` is line-buffered; every v1 frame is minified JSON with no
        // newline, so an unflushed frame would wait for unrelated output.
        output.flush()?;
    }
}

/// The supervisor socket carries a 2 s write timeout. Under sustained output
/// the peer can stay slow for longer than that while perfectly alive, so a
/// timeout is backpressure, not a dead transport. A closed peer still fails
/// at once (broken pipe/reset); only a peer that makes no progress at all for
/// this long is given up on.
const RELAY_STALL_LIMIT: Duration = Duration::from_secs(120);

fn is_backpressure(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// `write_all` that survives write timeouts without losing its position.
fn write_all_retry(
    output: &mut impl io::Write,
    mut bytes: &[u8],
    stall_limit: Duration,
) -> io::Result<()> {
    let mut last_progress = Instant::now();
    while !bytes.is_empty() {
        match output.write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => {
                bytes = &bytes[written..];
                last_progress = Instant::now();
            }
            Err(error) if is_backpressure(&error) => {
                if last_progress.elapsed() >= stall_limit {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// One bounded frame payload, or `None` on a clean end of input.
fn read_payload(reader: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut prefix = [0; 4];
    let mut filled = 0;
    while filled < prefix.len() {
        match reader.read(&mut prefix[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => return Err(fail()),
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    let length = u32::from_be_bytes(prefix) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(fail());
    }
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload)?;
    Ok(Some(payload))
}

/// The screen-scoped, read-only browse grant (SSH-12). It lives only as long
/// as this stdio session and ends on `BrowseClose` or end of input. No
/// runtime directory, supervisor, socket or file is created, no file content
/// is read, and nothing a tab owns is reachable from here.
fn browse(
    open: &BrowseEnvelope,
    input: &mut impl Read,
    output: &mut impl io::Write,
) -> io::Result<()> {
    open.validate_request().map_err(|_| fail())?;
    if open.operation != BrowseOperation::BrowseOpen {
        return Err(fail());
    }
    write_browse(
        output,
        open,
        BrowseOperation::BrowseOpened {
            home: browse_home(),
        },
    )?;
    while let Some(payload) = read_payload(input)? {
        // A tab frame, a reply, a malformed path or another grant, authority
        // or epoch ends this grant unanswered.
        let request: BrowseEnvelope = serde_json::from_slice(&payload).map_err(|_| fail())?;
        if request.validate_request().is_err() || !request.identity.matches(&open.identity) {
            return Err(fail());
        }
        let replies = match &request.operation {
            BrowseOperation::BrowseClose => {
                return write_browse(output, &request, BrowseOperation::BrowseClosed);
            }
            BrowseOperation::BrowseList { path } => browse_list(path),
            BrowseOperation::BrowseCheck { path } => vec![BrowseOperation::BrowseChecked {
                requested: path.clone(),
                result: check_directory(path),
            }],
            BrowseOperation::BrowseGitRead { path, command } => browse_git(path, *command),
            // A second open under a live grant.
            _ => browse_error(ErrorCode::InvalidRequest),
        };
        for reply in replies {
            write_browse(output, &request, reply)?;
        }
    }
    Ok(())
}

fn write_browse(
    output: &mut impl io::Write,
    request: &BrowseEnvelope,
    operation: BrowseOperation,
) -> io::Result<()> {
    let reply = BrowseEnvelope::new(request.identity.clone(), request.request_id, operation);
    output.write_all(&encode_browse(&reply).map_err(|_| fail())?)?;
    output.flush()
}

/// This helper user's home, from its own environment as every other helper
/// mode resolves it, never from client input: reported only when it is an
/// absolute, readable directory, by its canonical server path.
fn browse_home() -> Option<RemotePath> {
    let home = PathsInputs::from_env(PlatformFamily::current()).home?;
    match check_directory(&remote(&home).ok()?) {
        DirectoryCheck::Directory { path } => Some(path),
        _ => None,
    }
}

fn browse_error(code: ErrorCode) -> Vec<BrowseOperation> {
    vec![BrowseOperation::Error { code }]
}

/// Classify one absolute path on this server. `metadata` follows links but
/// only stats, so a FIFO, socket or device is classified without being opened;
/// a directory is opened only as a directory, to prove it can be listed.
fn check_directory(path: &RemotePath) -> DirectoryCheck {
    let Ok(directory) = native(path) else {
        return DirectoryCheck::Unreadable;
    };
    match fs::metadata(&directory) {
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            DirectoryCheck::Missing
        }
        Err(_) => DirectoryCheck::Unreadable,
        Ok(metadata) if !metadata.is_dir() => DirectoryCheck::NotDirectory,
        Ok(_) => match (fs::read_dir(&directory), fs::canonicalize(&directory)) {
            (Ok(_), Ok(canonical)) => remote(&canonical)
                .map_or(DirectoryCheck::Unreadable, |path| {
                    DirectoryCheck::Directory { path }
                }),
            _ => DirectoryCheck::Unreadable,
        },
    }
}

/// The tab listing code under a single-use guard rooted at this directory:
/// kinds come from the entries themselves (links are reported, not
/// followed), no entry is opened, and at most MAX_BROWSE_ENTRIES are sent,
/// folders first.
fn browse_list(path: &RemotePath) -> Vec<BrowseOperation> {
    if !matches!(check_directory(path), DirectoryCheck::Directory { .. }) {
        return browse_error(ErrorCode::Unavailable);
    }
    let listed = match Filesystem::new(path, 1).and_then(|filesystem| filesystem.list(path)) {
        Ok(listed) => listed,
        Err(code) => return browse_error(code),
    };
    let mut truncated = false;
    let mut entries = Vec::new();
    for operation in listed {
        if let Operation::Listed {
            entries: batch,
            truncated: batch_truncated,
            ..
        } = operation
        {
            truncated |= batch_truncated;
            entries.extend(batch);
        }
    }
    if entries.len() > MAX_BROWSE_ENTRIES {
        entries.truncate(MAX_BROWSE_ENTRIES);
        truncated = true;
    }
    // Eight entries per frame, as for tabs: worst-case paths fit the frame cap.
    // Truncation is reported once, on the final frame.
    let batches: Vec<_> = entries.chunks(8).collect();
    if batches.is_empty() {
        return vec![BrowseOperation::BrowseListed {
            path: path.clone(),
            entries: Vec::new(),
            truncated,
            done: true,
        }];
    }
    batches
        .iter()
        .enumerate()
        .map(|(index, batch)| {
            let done = index + 1 == batches.len();
            BrowseOperation::BrowseListed {
                path: path.clone(),
                entries: batch.to_vec(),
                truncated: truncated && done,
                done,
            }
        })
        .collect()
}

/// One of the fixed read-only Git reads in the server-resolved directory,
/// passed only as the value of `-C`. The bounded runner rejects any other argv.
fn browse_git(path: &RemotePath, command: GitRead) -> Vec<BrowseOperation> {
    let DirectoryCheck::Directory { path: canonical } = check_directory(path) else {
        return browse_error(ErrorCode::Unavailable);
    };
    let bytes = match native(&canonical).and_then(|directory| {
        GitRunner::new()
            .read_bytes(command.into(), &directory)
            .map_err(|_| ErrorCode::Unavailable)
    }) {
        Ok(bytes) => bytes,
        Err(code) => return browse_error(code),
    };
    let pieces: Vec<&[u8]> = if bytes.is_empty() {
        vec![&[]]
    } else {
        bytes.chunks(MAX_CHUNK_BYTES).collect()
    };
    let mut replies = Vec::with_capacity(pieces.len());
    let mut offset = 0;
    for (index, piece) in pieces.iter().enumerate() {
        let Ok(data) = Chunk::new(piece.to_vec()) else {
            return browse_error(ErrorCode::Limit);
        };
        replies.push(BrowseOperation::BrowseGitResult {
            path: path.clone(),
            command,
            offset,
            data,
            done: index + 1 == pieces.len(),
        });
        offset += piece.len() as u64;
    }
    replies
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
    let environment = Environment::current();
    let shell_program = PathBuf::from(&shell.program);
    let shell_args = shell.args.clone();
    let (program, args) = match target {
        LaunchTarget::Shell => (shell_program.clone(), shell.args),
        LaunchTarget::OpenCode => (find("opencode")?, Vec::new()),
        // Direct Grok launch: the detected absolute grok with the single
        // `dashboard` argument. No shell first, no plugin configuration, no
        // bridge, no composed OpenCode config.
        LaunchTarget::Grok => {
            let program = grok::detect_grok(&environment).ok_or_else(fail)?;
            (program, vec![OsString::from("dashboard")])
        }
        // Direct Claude Code launch: the detected absolute claude with no
        // arguments, no shell first and no bridge. Its environment gets only
        // the staged Claude Code plugin entry and spool binding below.
        LaunchTarget::Claude => {
            let program = grok::detect_on_path(&environment, "claude").ok_or_else(fail)?;
            (program, Vec::new())
        }
    };
    let inputs = PathsInputs::from_env(PlatformFamily::current());
    let home = inputs.home.ok_or_else(fail)?;
    let v2 = inputs
        .xdg_config_home
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
        .join("opencode/cli.json");
    let user_config = resolve_config(v2, paths.opencode_config_path.clone());
    let plugin_dir = paths.plugin_dir.clone();
    let plugin_root = paths.plugin_root.clone();
    let inline_config = config::inline_config_from_env();
    // Failure of optional plugin composition leaves terminal/fs available.
    // Grok and Claude Code use no bridge, no plugin environment and no composed
    // config, so those launches skip integration ownership entirely.
    let integration = if matches!(target, LaunchTarget::Grok | LaunchTarget::Claude) {
        None
    } else {
        Integration::start(
            &plugin_dir,
            &plugin_root,
            user_config.clone(),
            inline_config.clone(),
            "remote".into(),
        )
        .ok()
    };
    let has_integration = integration.is_some();
    // Claude Code: the server-staged plugin entry, composed with the remote
    // user's own list, and this tab's spool, for shell and Claude Code
    // launches only. No host value is involved; an inherited spool binding
    // never reaches another tab's processes.
    let staged = claude::staged_plugin_dir(&std::env::current_exe()?);
    let composed = launch::compose(
        *target,
        &platform.environment(&[], &[]).with_terminal_defaults(),
        integration.as_ref().map(Integration::environment),
        staged.as_deref(),
        &environment,
        &claude::spool_path(&runtime),
    );
    let launch = ChildLaunch {
        program,
        args,
        cwd: convert(native(directory))?,
        env: composed.launch,
    };
    let mut supervisor = convert(Supervisor::create(
        &first,
        runtime,
        &paths.data_dir,
        &std::env::current_exe()?,
        launch,
        integration,
    ))?;
    // A launch child that ended gives way to the shell `Terminal SSH` runs, in
    // the same attachment, when the desktop asks for it. That shell gets what
    // a fresh `Terminal SSH` shell gets, including the OpenCode bridge a Grok
    // Build or Claude Code launch does not own until then.
    let integration: Option<supervisor::IntegrationStart> = (!has_integration).then(|| {
        Box::new(move || {
            Integration::start(
                &plugin_dir,
                &plugin_root,
                user_config,
                inline_config,
                "remote".into(),
            )
            .ok()
        }) as supervisor::IntegrationStart
    });
    supervisor = supervisor.with_follow_up_shell(FollowUpShell {
        program: shell_program,
        args: shell_args,
        env: composed.follow_up,
        integration,
    });
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

/// The explicit `--install-grok-plugin` mode. The per-server/version consent
/// flow invokes it; nothing else does. stdout carries exactly one bounded
/// honest JSON report line; the process exits non-zero on failure.
fn install_mode() -> io::Result<()> {
    let environment = Environment::current();
    let staged = std::env::current_exe().ok().and_then(|exe| {
        exe.parent()
            .map(|directory| directory.join("plugins/grok-made"))
    });
    let report = grok::install_grok_plugin(grok::InstallInputs {
        grok: grok::detect_grok(&environment),
        files: GrokFiles::from_environment(&environment),
        staged,
        data_dir: paths().ok().map(|paths| paths.data_dir),
        budget: grok::INSTALL_BUDGET,
    });
    println!("{}", report.line());
    if report.failed() {
        Err(fail())
    } else {
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StageManifest {
    consent: bool,
    /// Explicit per-attempt approval to replace a same-version companion whose
    /// current artifacts differ. Default false keeps older callers unchanged.
    #[serde(default)]
    update: bool,
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
/// The complete shipped claude-code-made package, with the native
/// session-event hook built for this target, staged beside grok-made.
const CLAUDE_ARTIFACTS: &[&str] = &[
    "plugins/claude-code-made/.claude-plugin/plugin.json",
    "plugins/claude-code-made/README.md",
    "plugins/claude-code-made/hooks/hooks.json",
    "plugins/claude-code-made/bin/made-claude-status",
];
/// Every staged artifact: the fixed list above plus the complete shipped
/// grok-made and claude-code-made packages, coordinated with the fixed
/// platform staging list.
fn all_artifacts() -> Vec<&'static str> {
    ARTIFACTS
        .iter()
        .copied()
        .chain(grok::GROK_ARTIFACTS.iter().copied())
        .chain(CLAUDE_ARTIFACTS.iter().copied())
        .collect()
}
/// Fixed marker shared with the platform stage script and its classifier.
const UPDATE_REQUIRED: &str = "MADE-STAGE-UPDATE-REQUIRED";
/// Promotion and its immediate restoration both failed. The original remains
/// at the fixed `.replace` path for the next attempt.
const RECOVERY_REQUIRED: &str = "MADE-STAGE-RECOVERY-REQUIRED";
/// Deterministic, app-owned path for the only original during a replacement.
const REPLACED: &str = ".replace";

fn update_required() -> io::Error {
    io::Error::new(io::ErrorKind::AlreadyExists, UPDATE_REQUIRED)
}

fn recovery_required() -> io::Error {
    io::Error::other(RECOVERY_REQUIRED)
}

fn executable_artifact(name: &str) -> bool {
    matches!(
        name,
        "made-remote"
            | "plugins/grok-made/hooks/bin/made-grok-status"
            | "plugins/claude-code-made/bin/made-claude-status"
    )
}

/// One already-written staging artifact must match size, hash and mode.
fn verify_artifact(path: &Path, file: &Artifact) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| fail())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != file.size {
        return Err(fail());
    }
    if !made_platform::artifact_permissions_match(&metadata, executable_artifact(&file.name)) {
        return Err(fail());
    }
    if ConfigSnapshot::capture(path)?.sha256_hex().as_deref() != Some(file.sha256.as_str()) {
        return Err(fail());
    }
    Ok(())
}

/// A leaf `symlink_metadata` check does not protect a read from a symlinked
/// parent. Every ancestor must be a plain directory before hashing. A missing
/// ancestor means the artifact itself is absent (a mismatch); an unreadable
/// ancestor is unsafe.
fn has_unsafe_ancestor(path: &Path) -> bool {
    path.ancestors()
        .skip(1)
        .any(|ancestor| match fs::symlink_metadata(ancestor) {
            Ok(metadata) => metadata.file_type().is_symlink() || !metadata.is_dir(),
            Err(error) => error.kind() != io::ErrorKind::NotFound,
        })
}

/// Read-only comparison for exact reuse. A missing, non-regular, malformed or
/// different artifact is a mismatch, never a write.
fn artifact_state(path: &Path, file: &Artifact) -> ArtifactState {
    // A leaf metadata check does not protect a read from a symlinked parent.
    // An ancestor link is unsafe, not an artifact mismatch, and nothing is hashed.
    if has_unsafe_ancestor(path) {
        return ArtifactState::Unsafe;
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return ArtifactState::Differs;
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != file.size {
        return ArtifactState::Differs;
    }
    if !made_platform::artifact_permissions_match(&metadata, executable_artifact(&file.name)) {
        return ArtifactState::Differs;
    }
    if ConfigSnapshot::capture(path)
        .ok()
        .and_then(|snapshot| snapshot.sha256_hex())
        .as_deref()
        == Some(file.sha256.as_str())
    {
        ArtifactState::Matches
    } else {
        ArtifactState::Differs
    }
}

enum ArtifactState {
    Matches,
    Differs,
    Unsafe,
}

/// Consume the fixed manifest stream and report whether the installed set is
/// byte-identical. No filesystem write happens here. A symlinked ancestor is a
/// distinct unsafe failure; only a leaf artifact difference is update-required.
fn compare_artifacts(
    files: &[Artifact],
    destination: &Path,
    input: &mut impl Read,
) -> io::Result<bool> {
    for file in files {
        match artifact_state(&destination.join(&file.name), file) {
            ArtifactState::Unsafe => return Err(fail()),
            ArtifactState::Differs => return Ok(false),
            ArtifactState::Matches => {}
        }
        io::copy(&mut input.by_ref().take(file.size), &mut io::sink())?;
    }
    Ok(true)
}

/// Write every artifact into the private temporary sibling and verify it.
fn write_artifacts(
    input: &mut impl Read,
    files: &[Artifact],
    destination: &Path,
) -> io::Result<()> {
    for file in files {
        let path = destination.join(&file.name);
        if let Some(parent) = path.parent() {
            convert(made_platform::ensure_private_directory(parent))?;
        }
        let mut output = convert(made_platform::create_private_file_new(
            &path,
            executable_artifact(&file.name),
        ))?;
        let copied = io::copy(&mut input.by_ref().take(file.size), &mut output)?;
        if copied != file.size {
            return Err(fail());
        }
        output.sync_all()?;
        verify_artifact(&path, file)?;
    }
    Ok(())
}

enum TargetState {
    Missing,
    Directory,
    Unsafe,
}

fn target_state(path: &Path) -> io::Result<TargetState> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(TargetState::Unsafe),
        Ok(metadata) if metadata.is_dir() => Ok(TargetState::Directory),
        Ok(_) => Ok(TargetState::Unsafe),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(TargetState::Missing),
        Err(error) => Err(error),
    }
}

/// Restore an original preserved by an interrupted or failed replacement.
/// Only the fixed app-owned path is inspected; nothing is matched by wildcard.
fn recover_replacement(root: &Path, destination: &Path) -> io::Result<()> {
    let replaced = root.join(REPLACED);
    let metadata = match fs::symlink_metadata(&replaced) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(fail());
    }
    convert(made_platform::ensure_private_directory(&replaced))?;
    match target_state(destination)? {
        TargetState::Missing => {
            fs::rename(&replaced, destination)?;
            Ok(())
        }
        TargetState::Directory => {
            // Destination plus the preserved copy can only follow a confirmed
            // promotion whose cleanup did not finish.
            convert(made_platform::ensure_private_directory(destination))?;
            fs::remove_dir_all(&replaced)
        }
        TargetState::Unsafe => Err(fail()),
    }
}

fn promote_replacement(staging: &Path, destination: &Path, replaced: &Path) -> io::Result<()> {
    promote_with(staging, destination, replaced, |from, to| {
        fs::rename(from, to)
    })
}

/// After the current tree moves to `replaced`, either promote the staged tree
/// or restore the original. `replaced` is removed only after a confirmed
/// promotion; otherwise the original stays at that fixed path for recovery.
fn promote_with(
    staging: &Path,
    destination: &Path,
    replaced: &Path,
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    rename(destination, replaced)?;
    match rename(staging, destination) {
        Ok(()) => fs::remove_dir_all(replaced),
        Err(_) => {
            if rename(replaced, destination).is_ok() {
                return Err(fail());
            }
            Err(recovery_required())
        }
    }
}

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
    let artifacts = all_artifacts();
    if !manifest.consent
        || manifest.target != capabilities()?.target
        || manifest.version.is_empty()
        || manifest.version.len() > 128
        || !manifest
            .version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        || manifest.files.len() != artifacts.len()
    {
        return Err(fail());
    }
    let mut seen = std::collections::HashSet::new();
    for file in &manifest.files {
        if !artifacts.contains(&file.name.as_str())
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
    let replaced = root.join(REPLACED);
    {
        // Serialize recovery with any concurrent stage attempt before the
        // read-only comparison or any write decision.
        let _lock = convert(made_platform::lock_private_file(
            &root.join("stage.lock"),
            Duration::from_secs(1),
        ))?;
        recover_replacement(&root, &destination)?;
    }
    match target_state(&destination)? {
        TargetState::Unsafe => return Err(fail()),
        TargetState::Directory => {
            if !manifest.update {
                // Read-only comparison; the original private-directory and
                // per-artifact checks below never repair or rewrite a file.
                return if compare_artifacts(&manifest.files, &destination, &mut input)? {
                    Ok(())
                } else {
                    Err(update_required())
                };
            }
            // Approved replacement preserves the private no-symlink/permission
            // contract before any byte is moved.
            convert(made_platform::ensure_private_directory(&destination))?;
        }
        TargetState::Missing => {}
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
        write_artifacts(&mut input, &manifest.files, &staging)?;
        // Serialize stage/promotion. No running process is signalled; an
        // already-executed companion keeps its own inode.
        let _lock = convert(made_platform::lock_private_file(
            &root.join("stage.lock"),
            Duration::from_secs(1),
        ))?;
        match target_state(&destination)? {
            TargetState::Directory if manifest.update => {
                promote_replacement(&staging, &destination, &replaced)?;
            }
            TargetState::Missing => fs::rename(&staging, &destination)?,
            _ => return Err(fail()),
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
        // `replaced` is never removed here: only a confirmed promotion or a
        // confirmed restoration may consume the original.
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{all_artifacts, relay};
    use std::io::{self, Write};

    #[derive(Default)]
    struct Recorder {
        bytes: Vec<u8>,
        flushes: usize,
    }

    impl Write for Recorder {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn relay_forwards_minified_frames_and_flushes_each_chunk() {
        let frame = b"\0\0\0\x1d{\"operation\":{\"op\":\"Attached\"}}";
        let mut output = Recorder::default();
        relay(&frame[..], &mut output).unwrap();
        assert_eq!(output.bytes, frame);
        assert_eq!(output.flushes, 1);
    }

    use std::time::Duration;

    /// A writer that times out like a socket with `SO_SNDTIMEO` whose peer is
    /// slow: every other write attempt reports `TimedOut`, and one write is
    /// partial.
    struct SlowWriter {
        bytes: Vec<u8>,
        calls: usize,
        stall_forever: bool,
    }
    impl io::Write for SlowWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            if self.stall_forever || self.calls % 2 == 1 {
                return Err(io::ErrorKind::TimedOut.into());
            }
            let n = buf.len().min(3);
            self.bytes.extend_from_slice(&buf[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn relay_treats_write_timeouts_as_backpressure_and_loses_no_bytes() {
        let frame = b"\0\0\0\x1d{\"operation\":{\"op\":\"Attached\"}}";
        let mut output = SlowWriter {
            bytes: Vec::new(),
            calls: 0,
            stall_forever: false,
        };
        relay(&frame[..], &mut output).expect("a slow peer is not a dead one");
        assert_eq!(output.bytes, frame);
    }

    #[test]
    fn a_peer_that_never_makes_progress_still_fails() {
        let mut output = SlowWriter {
            bytes: Vec::new(),
            calls: 0,
            stall_forever: true,
        };
        let error =
            super::write_all_retry(&mut output, b"x", Duration::from_millis(30)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn relay_read_timeout_is_an_error_not_a_spin() {
        struct TimesOut;
        impl io::Read for TimesOut {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::TimedOut.into())
            }
        }
        let mut output = Recorder::default();
        let error = relay(TimesOut, &mut output).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn a_closed_peer_fails_immediately() {
        struct Closed;
        impl io::Write for Closed {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let error = super::write_all_retry(&mut Closed, b"x", Duration::from_secs(60)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[cfg(unix)]
    fn private_directory(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir(path).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn failed_promotion_restores_the_original_and_consumes_the_replacement() {
        use super::promote_with;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let destination = root.join("target");
        let staging = root.join(".stage-test");
        let replaced = root.join(".replace");
        private_tree(&destination, b"old");
        private_tree(&staging, b"new");
        let mut calls = 0;
        let error = promote_with(&staging, &destination, &replaced, |from, to| {
            calls += 1;
            match calls {
                1 => std::fs::rename(from, to),
                2 => Err(std::io::Error::other("injected promotion failure")),
                _ => std::fs::rename(from, to),
            }
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "remote operation unavailable");
        assert_eq!(std::fs::read(destination.join("artifact")).unwrap(), b"old");
        assert!(!replaced.exists());
        assert!(staging.exists());
    }

    #[test]
    #[cfg(unix)]
    fn failed_promotion_and_rollback_keep_the_original_for_the_next_attempt() {
        use super::{promote_with, recover_replacement, RECOVERY_REQUIRED};
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let destination = root.join("target");
        let staging = root.join(".stage-test");
        let replaced = root.join(".replace");
        private_tree(&destination, b"old");
        private_tree(&staging, b"new");
        let mut calls = 0;
        let error = promote_with(&staging, &destination, &replaced, |from, to| {
            calls += 1;
            if calls == 1 {
                std::fs::rename(from, to)
            } else {
                Err(std::io::Error::other("injected failure"))
            }
        })
        .unwrap_err();
        assert_eq!(error.to_string(), RECOVERY_REQUIRED);
        assert!(!destination.exists());
        assert_eq!(std::fs::read(replaced.join("artifact")).unwrap(), b"old");

        // The next stage attempt restores before comparing or writing.
        recover_replacement(root, &destination).unwrap();
        assert_eq!(std::fs::read(destination.join("artifact")).unwrap(), b"old");
        assert!(!replaced.exists());
    }

    #[cfg(unix)]
    fn private_tree(path: &std::path::Path, content: &[u8]) {
        private_directory(path);
        std::fs::write(path.join("artifact"), content).unwrap();
    }

    #[test]
    fn stage_accepts_exactly_the_fixed_platform_and_app_artifact_list() {
        assert_eq!(
            all_artifacts(),
            vec![
                "made-remote",
                "plugins/opencode-made/package.json",
                "plugins/opencode-made/tui.ts",
                "plugins/opencode-made/index.ts",
                "plugins/opencode-made/src/protocol.ts",
                "plugins/opencode-made/src/integration.ts",
                "plugins/grok-made/plugin.json",
                "plugins/grok-made/README.md",
                "plugins/grok-made/hooks/hooks.json",
                "plugins/grok-made/hooks/bin/made-grok-status",
                "plugins/claude-code-made/.claude-plugin/plugin.json",
                "plugins/claude-code-made/README.md",
                "plugins/claude-code-made/hooks/hooks.json",
                "plugins/claude-code-made/bin/made-claude-status",
            ],
            "the legacy --stage manifest must accept exactly the fixed \
             platform/app staging list"
        );
    }
}
