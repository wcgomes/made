//! Launch-local cwd hooks. No user startup file is written.
use crate::PtyRequest;
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

/// Convert a decoded local URI path to a native absolute path. Never accepts
/// UNC, device, drive-relative or Windows root-relative paths.
pub fn local_cwd_path(decoded: &str) -> Option<PathBuf> {
    if !decoded.starts_with('/')
        || decoded.starts_with("//")
        || decoded.chars().any(|c| c.is_control() || c == '\\')
    {
        return None;
    }
    let bytes = decoded.as_bytes();
    let drive = bytes.len() >= 3 && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
    if drive && bytes.get(3) != Some(&b'/') {
        return None;
    }
    #[cfg(windows)]
    let decoded = {
        if !drive || decoded[3..].chars().any(|c| ":<>\"|?*".contains(c)) {
            return None;
        }
        &decoded[1..]
    };
    let path = PathBuf::from(decoded);
    path.is_absolute().then_some(path)
}

/// Whether a safe emitter was installed for this launch (not proof of a report).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCwdCapability {
    /// A launch-local emitter was installed, not a claim of platform validation.
    pub supported: bool,
    /// Diagnostic explanation without child environment or command contents.
    pub reason: String,
}
impl ShellCwdCapability {
    /// Whether this launch has an installed emitter.
    pub fn is_supported(&self) -> bool {
        self.supported
    }
}

/// Owns the request and private startup directory until the PTY is released.
/// Debug intentionally excludes child arguments/environment.
pub struct ShellLaunchPlan {
    request: PtyRequest,
    capability: ShellCwdCapability,
    directory: Option<PathBuf>,
}
impl std::fmt::Debug for ShellLaunchPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellLaunchPlan")
            .field("request", &self.request)
            .field("capability", &self.capability)
            .field("has_bootstrap", &self.directory.is_some())
            .finish()
    }
}
impl Drop for ShellLaunchPlan {
    fn drop(&mut self) {
        if let Some(path) = &self.directory {
            let _ = fs::remove_dir_all(path);
        }
    }
}
impl ShellLaunchPlan {
    /// Prepare hooks, falling back to the original request on unsafe modes or IO failure.
    pub fn new(request: PtyRequest) -> Self {
        let mut plan = Self {
            request,
            capability: ShellCwdCapability {
                supported: false,
                reason: "shell has no safe launch-local cwd hook".into(),
            },
            directory: None,
        };
        let shell = Path::new(&plan.request.program)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let allowed: &[&str] = match shell.as_str() {
            "bash" => &["-i", "--noprofile", "--norc"],
            "zsh" => &["-i", "-l", "--login"],
            "fish" => &["-i", "--interactive", "-l", "--login"],
            "pwsh" | "powershell" => &["-NoLogo", "-NoProfile", "-NoExit"],
            "cmd" => {
                plan.capability.reason = "cmd has no demonstrated safe cwd hook; latest known cwd or launch folder remains the fallback".into();
                return plan;
            }
            _ => return plan,
        };
        let program = Path::new(&plan.request.program);
        let available = if program.components().count() > 1 {
            program.is_file()
        } else {
            plan.env("PATH").is_some_and(|path| {
                std::env::split_paths(&path).any(|dir| {
                    let candidate = dir.join(program);
                    candidate.is_file()
                        || (cfg!(windows) && candidate.with_extension("exe").is_file())
                })
            })
        };
        if !available {
            plan.capability.reason =
                "shell executable unavailable in child PATH; cwd integration not installed".into();
            return plan;
        }
        if cfg!(windows) && matches!(shell.as_str(), "bash" | "zsh" | "fish") {
            plan.capability.reason = "native Windows POSIX-shell cwd path translation is not validated; normal startup retained".into();
            return plan;
        }
        if plan
            .request
            .args
            .iter()
            .any(|a| !allowed.iter().any(|v| a.eq_ignore_ascii_case(v)))
        {
            plan.capability.reason = "launch arguments require unsupported startup, command, restricted or policy semantics".into();
            return plan;
        }
        if matches!(shell.as_str(), "pwsh" | "powershell")
            && !powershell_allows_bootstrap(&plan.request)
        {
            plan.capability.reason = "PowerShell policy/language mode does not permit a local unsigned bootstrap, or preflight unavailable; normal startup retained".into();
            return plan;
        }
        match plan.prepare(&shell) {
            Ok(()) => {
                plan.capability = ShellCwdCapability {
                    supported: true,
                    reason: "launch-local OSC 7 hook installed; reports remain untrusted".into(),
                }
            }
            Err(_) => {
                plan.capability.reason =
                    "could not prepare private cwd bootstrap; normal startup retained".into();
            }
        }
        plan
    }
    /// Resolved child request with launch-local startup arguments/environment.
    pub fn request(&self) -> &PtyRequest {
        &self.request
    }
    /// Reporting support and explicit fallback reason for this launch.
    pub fn capability(&self) -> &ShellCwdCapability {
        &self.capability
    }
    /// Private launch-owned files. Dropping the plan deletes only this directory.
    pub fn bootstrap_directory(&self) -> Option<&Path> {
        self.directory.as_deref()
    }
    fn env(&self, name: &str) -> Option<OsString> {
        self.request
            .env
            .iter()
            .rev()
            .find(|(k, _)| k == name || (cfg!(windows) && k.eq_ignore_ascii_case(name)))
            .map(|(_, v)| v.clone())
    }
    fn set_env(&mut self, name: &str, value: OsString) {
        self.request
            .env
            .retain(|(k, _)| k != name && !(cfg!(windows) && k.eq_ignore_ascii_case(name)));
        self.request
            .env_remove
            .retain(|key| key != name && !(cfg!(windows) && key.eq_ignore_ascii_case(name)));
        self.request.env.push((name.into(), value));
    }
    fn remove_env(&mut self, name: &str) {
        self.request
            .env
            .retain(|(k, _)| k != name && !(cfg!(windows) && k.eq_ignore_ascii_case(name)));
        self.request
            .env_remove
            .retain(|key| key != name && !(cfg!(windows) && key.eq_ignore_ascii_case(name)));
        self.request.env_remove.push(name.into());
    }
    fn prepare(&mut self, shell: &str) -> std::io::Result<()> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "made-shell-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        #[cfg(unix)]
        let mut builder = fs::DirBuilder::new();
        #[cfg(not(unix))]
        let builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory)?;
        self.directory = Some(directory.clone());
        match shell {
            "bash" => {
                let path = directory.join("bashrc");
                let no_rc = self.request.args.iter().any(|a| a == "--norc");
                fs::write(
                    &path,
                    format!(
                        "{}\n{BASH}",
                        if no_rc {
                            ""
                        } else {
                            "[[ -r ~/.bashrc ]] && source ~/.bashrc"
                        }
                    ),
                )?;
                self.request.args.retain(|a| a != "--norc");
                self.request
                    .args
                    .splice(0..0, [OsString::from("--rcfile"), path.into_os_string()]);
                self.request.args.push("-i".into());
            }
            "zsh" => {
                // zsh reads the system files itself; these wrappers chain only user files.
                fs::write(directory.join(".zshenv"), ZSH_ZSHENV)?;
                for name in [".zprofile", ".zshrc", ".zlogin", ".zlogout"] {
                    fs::write(directory.join(name), zsh_chain_script(name))?;
                }
                // Preserve whether the user's ZDOTDIR was set: an unset value
                // must stay unset so zsh uses $HOME for later startup files and
                // child shells. MADE_ORIGINAL_ZDOTDIR is absent (not empty)
                // when the user did not set ZDOTDIR.
                match self.env("ZDOTDIR") {
                    Some(original) => self.set_env("MADE_ORIGINAL_ZDOTDIR", original),
                    None => self.remove_env("MADE_ORIGINAL_ZDOTDIR"),
                }
                self.remove_env("MADE_USER_ZDOTDIR");
                self.remove_env("MADE_ZDOTDIR_UNSET");
                self.set_env("MADE_BOOTSTRAP_ZDOTDIR", directory.clone().into_os_string());
                self.set_env("ZDOTDIR", directory.into_os_string());
                self.request.args.push("-i".into());
            }
            "fish" => {
                let path = directory.join("init.fish");
                fs::write(&path, FISH)?;
                self.set_env("MADE_FISH_INIT", path.into_os_string());
                self.request.args.extend([
                    "--interactive".into(),
                    "--init-command".into(),
                    "source \"$MADE_FISH_INIT\"".into(),
                ]);
            }
            _ => {
                let path = directory.join("init.ps1");
                fs::write(&path, POWERSHELL)?;
                self.request
                    .args
                    .extend(["-NoExit".into(), "-File".into(), path.into_os_string()]);
            }
        }
        Ok(())
    }
}

// This is a fresh, noninteractive preflight process, never a command injected
// into the live PTY. No execution policy is changed or bypassed.
fn powershell_allows_bootstrap(request: &PtyRequest) -> bool {
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let child = Command::new(&request.program)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "[Console]::Write([string]$ExecutionContext.SessionState.LanguageMode + ':' + [string](Get-ExecutionPolicy))"])
        .env_clear().envs(request.env.iter().cloned())
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
    let Ok(mut child) = child else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return false;
                }
                let mut output = String::new();
                if let Some(stdout) = child.stdout.take() {
                    if stdout.take(256).read_to_string(&mut output).is_err() {
                        return false;
                    }
                }
                return matches!(
                    output.trim(),
                    "FullLanguage:Unrestricted"
                        | "FullLanguage:RemoteSigned"
                        | "FullLanguage:Bypass"
                );
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

// LC_ALL=C makes substring indexing byte-oriented, percent-encoding UTF-8 bytes.
const BASH: &str = r#"
__made_cwd() {
    local LC_ALL=C p=$PWD out='' c n i
    (( ${#p} <= 4096 )) || return 0
    for ((i=0; i<${#p}; i++)); do
        c=${p:i:1}
        case "$c" in [a-zA-Z0-9/._~-]) out+=$c;; *) printf -v n '%%%02X' "'$c"; out+=$n;; esac
    done
    (( ${#out} <= 8000 )) || return 0
    printf '\033]7;file://%s\007' "$out"
}
if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == 'declare -a '* ]]; then
    PROMPT_COMMAND+=(__made_cwd)
else
    PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND$'\n'}__made_cwd"
fi
"#;
// Keeps the user-visible ZDOTDIR state separate from the private bootstrap
// directory. `MADE_ZDOTDIR_UNSET` records that the user left ZDOTDIR unset, so
// later startup files and the resulting shell can fall back to $HOME without
// turning "unset" into an empty assignment.
const ZSH_CAPTURE_ZDOTDIR: &str = r#"if [[ -n ${ZDOTDIR+x} ]]; then
    MADE_USER_ZDOTDIR=$ZDOTDIR
    unset MADE_ZDOTDIR_UNSET
else
    MADE_USER_ZDOTDIR=$HOME
    MADE_ZDOTDIR_UNSET=1
fi
"#;
const ZSH_RESTORE_ZDOTDIR: &str = r#"if [[ -n ${MADE_ZDOTDIR_UNSET+x} ]]; then
    unset ZDOTDIR
else
    ZDOTDIR=$MADE_USER_ZDOTDIR
fi
"#;
const ZSH_ZSHENV: &str = r#"if [[ -n ${MADE_ORIGINAL_ZDOTDIR+x} ]]; then
    ZDOTDIR=$MADE_ORIGINAL_ZDOTDIR
else
    unset ZDOTDIR
fi
__made_zdotdir=${ZDOTDIR:-$HOME}
if [[ -r $__made_zdotdir/.zshenv ]]; then
    source "$__made_zdotdir/.zshenv"
fi
if [[ -n ${ZDOTDIR+x} ]]; then
    MADE_USER_ZDOTDIR=$ZDOTDIR
    unset MADE_ZDOTDIR_UNSET
else
    MADE_USER_ZDOTDIR=$HOME
    MADE_ZDOTDIR_UNSET=1
fi
unset __made_zdotdir MADE_ORIGINAL_ZDOTDIR
ZDOTDIR=$MADE_BOOTSTRAP_ZDOTDIR
"#;

fn zsh_chain_script(name: &str) -> String {
    let mut script = String::new();
    script.push_str("if [[ -n ${MADE_ZDOTDIR_UNSET+x} ]]; then\n    unset ZDOTDIR\nelse\n    ZDOTDIR=$MADE_USER_ZDOTDIR\nfi\n");
    script.push_str("__made_zdotdir=${ZDOTDIR:-$HOME}\n");
    script.push_str(&format!(
        "if [[ -r $__made_zdotdir/{name} ]]; then\n    source \"$__made_zdotdir/{name}\"\nfi\nunset __made_zdotdir\n"
    ));
    script.push_str(ZSH_CAPTURE_ZDOTDIR);
    script.push_str("ZDOTDIR=$MADE_BOOTSTRAP_ZDOTDIR\n");
    match name {
        ".zshrc" => {
            script.push_str(ZSH);
            script.push_str("if [[ ! -o login ]]; then\n");
            script.push_str(ZSH_RESTORE_ZDOTDIR);
            script.push_str(
                "unset MADE_BOOTSTRAP_ZDOTDIR MADE_USER_ZDOTDIR MADE_ZDOTDIR_UNSET\nfi\n",
            );
        }
        ".zlogin" => {
            script.push_str(ZSH_RESTORE_ZDOTDIR);
            script.push_str("unset MADE_BOOTSTRAP_ZDOTDIR MADE_USER_ZDOTDIR MADE_ZDOTDIR_UNSET\n");
        }
        ".zlogout" => script.push_str(ZSH_RESTORE_ZDOTDIR),
        _ => {}
    }
    script
}

const ZSH: &str = r#"
__made_cwd() {
    local LC_ALL=C p=$PWD out='' c n
    local -i i
    (( ${#p} <= 4096 )) || return 0
    for ((i=1; i<=${#p}; i++)); do
        c=$p[i]
        case "$c" in [a-zA-Z0-9/._~-]) out+=$c;; *) printf -v n '%%%02X' "'$c"; out+=$n;; esac
    done
    (( ${#out} <= 8000 )) || return 0
    printf '\033]7;file://%s\007' "$out"
}
precmd_functions+=(__made_cwd)
"#;
const FISH: &str = r#"
function __made_cwd --on-event fish_prompt
    if test (string length -- "$PWD") -le 4096
        set -l encoded (string replace -a '%2F' '/' -- (string escape --style=url -- "$PWD"))
        if test (string length -- "$encoded") -le 8000
            printf '\e]7;file://%s\a' "$encoded"
        end
    end
end
"#;
const POWERSHELL: &str = r#"
$global:__MadeOriginalPrompt = $function:prompt
function global:prompt {
    $result = & $global:__MadeOriginalPrompt
    if ($PWD.Provider.Name -eq 'FileSystem' -and $PWD.Path.Length -le 4096) {
        $uri = [System.Uri]::new($PWD.Path).AbsoluteUri
        if ($uri.Length -le 8000) { [Console]::Write(([char]27 + ']7;' + $uri + [char]7)) }
    }
    $result
}
"#;
