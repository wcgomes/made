//! Installed OpenSSH, bounded stdio and scoped process ownership. No domain/UI dependencies.
use crate::ssh_askpass::{AskpassBroker, PromptAnswer, PromptRequest};
use std::{
    ffi::OsString,
    fmt, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub const SSH_CHUNK_BYTES: usize = 64 * 1024;
pub const SSH_QUEUE_CHUNKS: usize = 256;

#[derive(Clone)]
pub struct SshDestination {
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<PathBuf>,
}
impl fmt::Debug for SshDestination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SshDestination(<redacted>)")
    }
}
impl SshDestination {
    pub fn validate(&self) -> io::Result<()> {
        // Not a shell expression, URI, user@host or an option. OpenSSH resolves aliases.
        let safe = |s: &str| {
            !s.is_empty()
                && s.len() <= 1024
                && !s.starts_with('-')
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-:[]%".contains(&b))
        };
        if !safe(&self.host) || self.user.as_ref().is_some_and(|u| !safe(u)) || self.port == Some(0)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid SSH destination",
            ));
        }
        if self
            .identity_file
            .as_ref()
            .is_some_and(|p| !p.is_absolute())
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(())
    }
}

/// Fixed scripts only. All variable bootstrap data travels through stdin.
#[derive(Debug, Clone, Copy)]
pub enum SshCommand {
    Probe,
    Companion,
    Stage,
    GrokPluginInstall,
}
impl SshCommand {
    pub fn script(self) -> &'static str {
        match self {
            Self::Probe => PROBE,
            Self::Companion => COMPANION,
            Self::Stage => STAGE,
            Self::GrokPluginInstall => GROK_PLUGIN_INSTALL,
        }
    }
}
// Shell receives exactly one constant command string. Never concatenate a path,
// hostname, user, prompt or server response here.
// Detection is metadata-only: command -v performs PATH lookups on the
// sanitized PATH and never executes or version-checks a program. OpenCode is
// additionally looked up in fixed user-owned install locations; grok and claude
// are PATH-only (SSH-09/SSH-10/SSH-11); no interpreter is looked up.
const PROBE: &str = r#"sh -c 'set -eu; oldifs=$IFS; IFS=:; safe=""; for p in $PATH; do case "$p" in /*) safe=${safe:+$safe:}$p;; esac; done; IFS=$oldifs; PATH=$safe; export PATH; printf "MADE-PROBE-1\n"; uname -s; uname -m; opencode=$(command -v opencode || :); for p in "$HOME/.opencode/bin/opencode" "$HOME/.local/bin/opencode" "$HOME/.cargo/bin/opencode"; do test -x "$p" && { opencode=$p; break; }; done; printf "%s\n" "$opencode"; grok=$(command -v grok || :); printf "%s\n" "$grok"; claude=$(command -v claude || :); printf "%s\n" "$claude"'"#;
const COMPANION: &str = r#"sh -c 'set -eu; IFS= read -r version; IFS= read -r target; case "$version" in ""|.|..|*[!a-zA-Z0-9._-]*) exit 64;; esac; case "$target" in x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu|x86_64-apple-darwin|aarch64-apple-darwin) ;; *) exit 65;; esac; case $(uname -s) in Darwin) root="$HOME/Library/Application Support/Made";; Linux) case ${XDG_DATA_HOME-} in /*) root="$XDG_DATA_HOME/made";; *) root="$HOME/.local/share/made";; esac;; *) exit 65;; esac; exec "$root/remote/$version/$target/made-remote" --stdio'"#;
// Consent-gated Grok plugin install: identical version/target/root/stdin
// discipline, but the staged companion starts in its `--install-grok-plugin`
// mode. Registering this command never runs it; a caller spawns it only after
// the separate per-server/version Grok install consent, and the companion owns
// the official `grok plugin install` invocation. Everything after the two
// header lines is forwarded to the companion's stdin.
const GROK_PLUGIN_INSTALL: &str = r#"sh -c 'set -eu; IFS= read -r version; IFS= read -r target; case "$version" in ""|.|..|*[!a-zA-Z0-9._-]*) exit 64;; esac; test ${#version} -le 128 || exit 64; case "$target" in x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu|x86_64-apple-darwin|aarch64-apple-darwin) ;; *) exit 65;; esac; case $(uname -s) in Darwin) root="$HOME/Library/Application Support/Made";; Linux) case ${XDG_DATA_HOME-} in /*) root="$XDG_DATA_HOME/made";; *) root="$HOME/.local/share/made";; esac;; *) exit 65;; esac; exec "$root/remote/$version/$target/made-remote" --install-grok-plugin'"#;

// First-install bootstrap does not assume an executable already exists. Exactly
// ten allowlisted files, length-delimited stdin, remote checksum
// verification, private temporary directory and atomic promotion. Never
// tar/extract/eval.
// Header: consent-v1 LF version LF target LF for initial staging/exact reuse,
// or consent-v2-update LF version LF target LF for an already approved
// same-version replacement. Then for each fixed file in order:
// decimal-size LF lowercase-sha256 LF raw-exactly-size-bytes.
// Existing artifacts that differ are never overwritten without the update
// header: the script prints MADE-STAGE-UPDATE-REQUIRED and exits nonzero
// without writing or deleting anything.
const STAGE: &str = r#"sh -c '
set -eu
umask 077
oldifs=$IFS; IFS=:; safe=""; for p in $PATH; do case "$p" in /*) safe=${safe:+$safe:}$p;; esac; done; IFS=$oldifs; PATH=$safe; export PATH
IFS= read -r consent
case "$consent" in consent-v1) update=0;; consent-v2-update) update=1;; *) exit 64;; esac
IFS= read -r version; IFS= read -r target
case "$version" in ""|.|..|*[!a-zA-Z0-9._-]*) exit 64;; esac
test ${#version} -le 128 || exit 64
case "$target" in x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu|x86_64-apple-darwin|aarch64-apple-darwin) ;; *) exit 65;; esac
case "$HOME" in /*) ;; *) exit 65;; esac
case $(uname -s) in
Darwin) root="$HOME/Library/Application Support/Made"; statmode=bsd;;
Linux) case ${XDG_DATA_HOME-} in /*) root="$XDG_DATA_HOME/made";; *) root="$HOME/.local/share/made";; esac; statmode=gnu;;
*) exit 65;; esac
case $(uname -s):$(uname -m) in
 Linux:x86_64) native=x86_64-unknown-linux-gnu;; Linux:aarch64) native=aarch64-unknown-linux-gnu;;
 Darwin:x86_64) native=x86_64-apple-darwin;; Darwin:arm64) native=aarch64-apple-darwin;; *) exit 65;; esac
test "$target" = "$native" || exit 65
if test "$statmode" = gnu; then
 libc=$(getconf GNU_LIBC_VERSION) || exit 65; libc=${libc##* }; major=${libc%%.*}; minor=${libc#*.}; minor=${minor%%.*}
 case "$major:$minor" in *[!0-9:]*) exit 65;; esac
 test "$major" -gt 2 || { test "$major" -eq 2 && test "$minor" -ge 31; } || exit 65
else os=$(sw_vers -productVersion); major=${os%%.*}; test "$major" -ge 12 || exit 65; fi
if command -v sha256sum >/dev/null 2>&1; then checksum=sha256sum
elif command -v shasum >/dev/null 2>&1; then checksum=shasum
else exit 69; fi
check_ancestors() {
 p=$1
 while test "$p" != /; do test ! -L "$p" || exit 73; p=${p%/*}; test -n "$p" || p=/; done
}
private() {
 check_ancestors "$1"
 mkdir -p "$1"
 if test "$statmode" = bsd; then mode=$(stat -f %Lp "$1"); owner=$(stat -f %u "$1")
 else mode=$(stat -c %a "$1"); owner=$(stat -c %u "$1"); fi
 test "$mode" = 700 && test "$owner" = "$(id -u)" || exit 73
}
mode_of() { if test "$statmode" = bsd; then stat -f %Lp "$1"; else stat -c %a "$1"; fi; }
update_required() { printf "MADE-STAGE-UPDATE-REQUIRED\n" >&2; exit 74; }
recovery_required() { printf "MADE-STAGE-RECOVERY-REQUIRED\n" >&2; exit 75; }
private "$root"; private "$root/remote"; private "$root/remote/$version"
base="$root/remote/$version"
replace="$base/.replace"
lock="$base/.stage-lock"
lock_tmp="$base/.stage-lock.$$"
reap="$base/.stage-reap.$$"
stage="$base/.stage-$$"
stage_owned=0
reap_owned=0
lock_owned=0
# A lock owner is alive while its PID exists. kill(2) can fail with EPERM for a
# live process, so ps confirms existence; a zombie is already dead. Missing or
# malformed tokens are never treated as dead: that would let a creation race
# steal a live lock.
owner_alive() {
 pid=$1
 case "$pid" in ""|*[!0-9]*) return 0;; esac
 if kill -0 "$pid" 2>/dev/null; then
  state=$(ps -p "$pid" -o stat= 2>/dev/null || :)
  case "$state" in *Z*) return 1;; *) return 0;; esac
 fi
 ps -p "$pid" >/dev/null 2>&1 && return 0
 return 1
}
release_lock() {
 if test "$lock_owned" = 1; then
  rm -f -- "$lock" 2>/dev/null || :
  lock_owned=0
 fi
}
cleanup() {
 if test "$stage_owned" = 1; then rm -rf -- "$stage" 2>/dev/null || :; stage_owned=0; fi
 if test "$reap_owned" = 1; then rm -rf -- "$reap" 2>/dev/null || :; reap_owned=0; fi
 rm -f -- "$lock_tmp" 2>/dev/null || :
 release_lock
}
# A stale directory is removable only when it is a real, non-symlink directory
# directly under the already validated private base with the exact private mode
# and owner this app creates. Symlinks are never followed or traversed.
private_stage_dir() {
 test ! -L "$1" && test -d "$1" || return 1
 if test "$statmode" = bsd; then dir_mode=$(stat -f %Lp "$1"); dir_owner=$(stat -f %u "$1")
 else dir_mode=$(stat -c %a "$1"); dir_owner=$(stat -c %u "$1"); fi
 test "$dir_mode" = 700 && test "$dir_owner" = "$(id -u)"
}
# Remove only exact app-owned acquisition temps. The name fixes the owner PID;
# a live PID, symlink or unreadable replacement is left untouched. This keeps a
# crash before publication from accumulating files without deleting anything
# whose ownership is unknown.
cleanup_stale_temps() {
 for candidate in "$base"/.stage-lock.*; do
  test ! -L "$candidate" && test -f "$candidate" || continue
  name=${candidate##*/}
  pid=${name#.stage-lock.}
  case "$pid" in ""|*[!0-9]*) continue;; esac
  if test "$pid" = "$$"; then
   rm -f -- "$candidate" 2>/dev/null || :
   continue
  fi
  if owner_alive "$pid"; then continue; fi
  token=
  IFS= read -r token < "$candidate" 2>/dev/null || :
  case "$token" in ""|"$pid") ;; *) continue;; esac
  again=
  IFS= read -r again < "$candidate" 2>/dev/null || :
  test "$again" = "$token" || continue
  rm -f -- "$candidate" 2>/dev/null || :
 done
}
# A crash while reaping leaves `.stage-reap.<reaper>` plus the dead owner
# `.stage-<owner>`. The reaper PID is in the artifact name and the moved lock
# still carries the owner token, so both must be demonstrably dead before
# either path is touched. Malformed names/tokens, symlinks and live PIDs are
# never deleted. Only a real private `.stage-<owner>` directory is removed;
# the reap artifact itself is removed afterwards by its exact name.
cleanup_stale_reaps() {
 for candidate in "$base"/.stage-reap.*; do
  test ! -L "$candidate" || continue
  test -f "$candidate" || test -d "$candidate" || continue
  name=${candidate##*/}
  pid=${name#.stage-reap.}
  case "$pid" in ""|*[!0-9]*) continue;; esac
  if test "$pid" != "$$" && owner_alive "$pid"; then continue; fi
  token=
  if test -d "$candidate"; then
   IFS= read -r token < "$candidate/owner" 2>/dev/null || :
  else
   IFS= read -r token < "$candidate" 2>/dev/null || :
  fi
  case "$token" in ""|*[!0-9]*) continue;; esac
  test "$token" != "$pid" || continue
  if owner_alive "$token"; then continue; fi
  again=
  if test -d "$candidate"; then
   IFS= read -r again < "$candidate/owner" 2>/dev/null || :
  else
   IFS= read -r again < "$candidate" 2>/dev/null || :
  fi
  test "$again" = "$token" || continue
  dead_stage="$base/.stage-$token"
  if private_stage_dir "$dead_stage"; then rm -rf -- "$dead_stage" 2>/dev/null || :; fi
  rm -rf -- "$candidate" 2>/dev/null || :
 done
}
# Serialize the whole attempt, including stale recovery. An owner token is
# written to a private PID-named file first, then hard-linked to the fixed lock
# path. `link` publishes or fails atomically, so the fixed lock is never observed
# without its complete owner token. A live owner is never disturbed. A
# demonstrably dead owner lock is moved aside atomically, its exact private
# `.stage-<pid>` is removed, and this attempt retries. The token is re-read
# after the move; a changed token means another owner appeared and the moved
# entry is never deleted.
acquire_lock() {
 cleanup_stale_reaps
 cleanup_stale_temps
 tries=0
 while test "$tries" -lt 4; do
  tries=$((tries + 1))
  acquired=0
  if test ! -e "$lock_tmp" && test ! -L "$lock_tmp"; then
   if (set -C; printf "%s\n" "$$" > "$lock_tmp") 2>/dev/null; then
    token=
    IFS= read -r token < "$lock_tmp" 2>/dev/null || :
    if test "$token" = "$$" && link -- "$lock_tmp" "$lock" 2>/dev/null; then
     lock_owned=1
     acquired=1
    fi
    rm -f -- "$lock_tmp" 2>/dev/null || :
   fi
  fi
  if test "$acquired" = 1; then return 0; fi
  if test -L "$lock"; then return 1; fi
  owner=
  if test -d "$lock"; then
   IFS= read -r owner < "$lock/owner" 2>/dev/null || :
  elif test -f "$lock"; then
   IFS= read -r owner < "$lock" 2>/dev/null || :
  else
   return 1
  fi
  case "$owner" in ""|*[!0-9]*) return 1;; esac
  if owner_alive "$owner"; then return 1; fi
  current=
  if test -d "$lock"; then
   IFS= read -r current < "$lock/owner" 2>/dev/null || :
  else
   IFS= read -r current < "$lock" 2>/dev/null || :
  fi
  test "$current" = "$owner" || return 1
  reap="$base/.stage-reap.$$"
  test ! -e "$reap" && test ! -L "$reap" || return 1
  mv -- "$lock" "$reap" 2>/dev/null || return 1
  moved=
  if test -d "$reap"; then
   IFS= read -r moved < "$reap/owner" 2>/dev/null || :
  else
   IFS= read -r moved < "$reap" 2>/dev/null || :
  fi
  if test "$moved" != "$owner"; then
   if test ! -e "$lock" && test ! -L "$lock"; then mv -- "$reap" "$lock" 2>/dev/null || :; fi
   return 1
  fi
  reap_owned=1
  dead_stage="$base/.stage-$owner"
  if private_stage_dir "$dead_stage"; then
   rm -rf -- "$dead_stage" 2>/dev/null || :
  fi
  dead_tmp="$base/.stage-lock.$owner"
  if test ! -L "$dead_tmp" && test -f "$dead_tmp"; then
   rm -f -- "$dead_tmp" 2>/dev/null || :
  fi
  if rm -rf -- "$reap" 2>/dev/null; then reap_owned=0; fi
 done
 return 1
}
acquire_lock || exit 73
trap cleanup 0
# Finish an interrupted replacement from an earlier attempt while the lock is
# held. The path is fixed and app-owned, so no wildcard or process-specific
# name is ever scanned.
recover_replace() {
 st=0
 if test -L "$replace" || { test -e "$replace" && test ! -d "$replace"; }; then
  st=73
 elif test ! -e "$base/$target" && test ! -L "$base/$target"; then
  mv -- "$replace" "$base/$target" 2>/dev/null || st=73
 else
  if test -L "$base/$target" || test ! -d "$base/$target"; then
   st=73
  else
   rm -rf -- "$replace" 2>/dev/null || st=73
  fi
 fi
 return "$st"
}
if test -e "$replace" || test -L "$replace"; then
 recover_replace || exit $?
fi
existing=0
test ! -L "$base/$target" || exit 73
if test -e "$base/$target"; then
 test -d "$base/$target" || exit 73
 private "$base/$target"
 existing=1
fi
if test "$existing" = 1 && test "$update" = 0; then
for name in made-remote plugins/opencode-made/package.json plugins/opencode-made/tui.ts plugins/opencode-made/index.ts plugins/opencode-made/src/protocol.ts plugins/opencode-made/src/integration.ts plugins/grok-made/plugin.json plugins/grok-made/README.md plugins/grok-made/hooks/hooks.json plugins/grok-made/hooks/bin/made-grok-status; do
 IFS= read -r size; IFS= read -r hash
 case "$size" in ""|*[!0-9]*) exit 64;; esac
 test ${#size} -le 9 && test "$size" -gt 0 && test "$size" -le 134217728 || exit 64
 case "$hash" in *[!0-9a-f]*) exit 64;; esac
 test ${#hash} -eq 64 || exit 64
 file="$base/$target/$name"
 # Check ancestors only: the leaf itself must reach the explicit
 # update-required classification below, not a generic security exit.
 check_ancestors "${file%/*}"
 if test -L "$file" || test ! -f "$file"; then update_required; fi
 dd bs=1 count="$size" of=/dev/null 2>/dev/null
 test "$(wc -c < "$file" | tr -d " ")" = "$size" || update_required
 if test "$checksum" = sha256sum; then actual=$(sha256sum "$file"); else actual=$(shasum -a 256 "$file"); fi
 case "$actual" in "$hash "*) ;; *) update_required;; esac
 case "$name" in
 made-remote|plugins/grok-made/hooks/bin/made-grok-status) test "$(mode_of "$file")" = 700 || update_required;;
 *) test "$(mode_of "$file")" = 600 || update_required;;
 esac
done
printf "MADE-STAGED-1\n"
exit 0
fi
mkdir "$stage"
stage_owned=1
mkdir "$stage/plugins" "$stage/plugins/opencode-made" "$stage/plugins/opencode-made/src" "$stage/plugins/grok-made" "$stage/plugins/grok-made/hooks" "$stage/plugins/grok-made/hooks/bin"
for name in made-remote plugins/opencode-made/package.json plugins/opencode-made/tui.ts plugins/opencode-made/index.ts plugins/opencode-made/src/protocol.ts plugins/opencode-made/src/integration.ts plugins/grok-made/plugin.json plugins/grok-made/README.md plugins/grok-made/hooks/hooks.json plugins/grok-made/hooks/bin/made-grok-status; do
 IFS= read -r size; IFS= read -r hash
 case "$size" in ""|*[!0-9]*) exit 64;; esac
 test ${#size} -le 9 && test "$size" -gt 0 && test "$size" -le 134217728 || exit 64
 case "$hash" in *[!0-9a-f]*) exit 64;; esac
 test ${#hash} -eq 64 || exit 64
 file="$stage/$name"
 dd bs=1 count="$size" of="$file" 2>/dev/null
 chmod 600 "$file"
 test "$(wc -c < "$file" | tr -d " ")" = "$size" || exit 74
 if test "$checksum" = sha256sum; then actual=$(sha256sum "$file"); else actual=$(shasum -a 256 "$file"); fi
 case "$actual" in "$hash "*) ;; *) exit 74;; esac
 case "$name" in
 made-remote|plugins/grok-made/hooks/bin/made-grok-status) chmod 700 "$file"; test "$(mode_of "$file")" = 700 || exit 74;;
 *) test "$(mode_of "$file")" = 600 || exit 74;;
 esac
done
if test "$existing" = 1; then
 if mv -- "$base/$target" "$replace" 2>/dev/null; then
  if mv -- "$stage" "$base/$target" 2>/dev/null; then
   stage_owned=0
   rm -rf -- "$replace" 2>/dev/null || exit 73
  elif mv -- "$replace" "$base/$target" 2>/dev/null; then
   exit 74
  else
   # The original is the only copy and now lives at the fixed `.replace`
   # path. The next attempt restores it before doing anything else.
   recovery_required
  fi
 else
  exit 74
 fi
else
 test ! -e "$base/$target" && test ! -L "$base/$target" || exit 73
 mv "$stage" "$base/$target"
 stage_owned=0
fi
release_lock
trap - 0
printf "MADE-STAGED-1\n"
'"#;

/// Prepare the fixed companion bootstrap header; subsequent bytes are codec
/// frames. Capabilities/tab tokens never appear in argv or this header.
pub fn companion_header(version: &str, target: &str) -> io::Result<Vec<u8>> {
    if version.is_empty()
        || version.len() > 128
        || matches!(version, "." | "..")
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        || !matches!(
            target,
            "x86_64-unknown-linux-gnu"
                | "aarch64-unknown-linux-gnu"
                | "x86_64-apple-darwin"
                | "aarch64-apple-darwin"
        )
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(format!("{version}\n{target}\n").into_bytes())
}
/// Explicit consent gates creation of a staging stream. Callers must verify
/// packaged local artifacts against their pinned manifest before transmission.
pub fn staging_header(consent: bool, version: &str, target: &str) -> io::Result<Vec<u8>> {
    if !consent {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let mut bytes = b"consent-v1\n".to_vec();
    bytes.extend(companion_header(version, target)?);
    Ok(bytes)
}
/// Explicit per-attempt approval to replace an existing same-version companion
/// whose artifacts differ. Nothing is replaced without this header.
pub fn staging_update_header(consent: bool, version: &str, target: &str) -> io::Result<Vec<u8>> {
    if !consent {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let mut bytes = b"consent-v2-update\n".to_vec();
    bytes.extend(companion_header(version, target)?);
    Ok(bytes)
}

pub struct SshOptions {
    pub executable: PathBuf,
    pub askpass_executable: PathBuf,
    pub private_directory: PathBuf,
    pub known_hosts: PathBuf,
    pub destination: SshDestination,
    pub command: SshCommand,
    /// Whole attempt deadline (including prompts); long-lived streams use None.
    pub timeout: Option<Duration>,
}
impl fmt::Debug for SshOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SshOptions(<redacted>)")
    }
}
impl SshOptions {
    pub fn argv(&self) -> io::Result<Vec<OsString>> {
        self.destination.validate()?;
        // OpenSSH's config parser expands percent tokens in this option and
        // treats whitespace specially. Refuse ambiguous app storage paths.
        let known = self
            .known_hosts
            .to_str()
            .filter(|s| {
                self.known_hosts.is_absolute()
                    && !s
                        .chars()
                        .any(|c| c.is_control() || matches!(c, '%' | '"' | '\\'))
            })
            .ok_or(io::ErrorKind::InvalidInput)?;
        let mut args: Vec<OsString> = ["-T", "-a", "-x"].into_iter().map(Into::into).collect();
        for option in [
            "ForwardAgent=no",
            "ForwardX11=no",
            "ClearAllForwardings=yes",
            "Tunnel=no",
            "PermitLocalCommand=no",
            "LocalCommand=none",
            "RemoteCommand=none",
            "ControlMaster=no",
            "ControlPath=none",
            "ControlPersist=no",
            "StrictHostKeyChecking=ask",
            "UpdateHostKeys=no",
            "VerifyHostKeyDNS=no",
            "KnownHostsCommand=none",
            "AddKeysToAgent=no",
            "ConnectTimeout=10",
            // Do not send SSH keepalive requests: some container port
            // forwards never answer them and the client then drops a healthy
            // long-lived session. OS-level TCP keepalive still detects dead
            // peers.
            "TCPKeepAlive=yes",
        ] {
            args.extend(["-o".into(), option.into()]);
        }
        args.extend([
            "-o".into(),
            format!("UserKnownHostsFile=\"{known}\"").into(),
        ]);
        if let Some(user) = &self.destination.user {
            args.extend(["-l".into(), user.into()]);
        }
        if let Some(port) = self.destination.port {
            args.extend(["-p".into(), port.to_string().into()]);
        }
        if let Some(key) = &self.destination.identity_file {
            args.extend(["-i".into(), key.as_os_str().into()]);
        }
        args.extend([
            "--".into(),
            self.destination.host.clone().into(),
            self.command.script().into(),
        ]);
        Ok(args)
    }
}

/// Stable, sanitized SSH failure. The detail is a static string. Raw stderr,
/// passwords, tokens, and hostnames are never copied into it. A fingerprint is
/// available only through [`SshFailure::unknown_host_fingerprint`].
#[derive(Clone, PartialEq, Eq)]
pub struct SshFailure {
    kind: SshFailureKind,
    detail: &'static str,
    unknown_host_fingerprint: Option<String>,
}

/// Why an SSH attempt failed. Match on this, not on stderr text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SshFailureKind {
    UnknownHost,
    ChangedHostKey,
    AuthFailed,
    Timeout,
    Cancelled,
    MissingCapability,
    /// The installed same-version companion differs and needs an approved
    /// replacement. Nothing was written or deleted.
    UpdateRequired,
    Io,
}

impl SshFailureKind {
    /// Stable machine code. This is the app-facing contract.
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnknownHost => "unknown_host",
            Self::ChangedHostKey => "changed_host_key",
            Self::AuthFailed => "auth_failed",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::MissingCapability => "missing_capability",
            Self::UpdateRequired => "update_required",
            Self::Io => "io",
        }
    }
}

impl SshFailure {
    /// Failure class.
    pub fn kind(&self) -> SshFailureKind {
        self.kind
    }

    /// Sanitized static explanation. Never raw stderr or a secret.
    pub fn detail(&self) -> &'static str {
        self.detail
    }

    /// Fingerprint presented for a never-trusted host, if one was parsed.
    ///
    /// Absent for every other kind, including a changed key. Debug output
    /// redacts this value so copied logs do not collect it.
    pub fn unknown_host_fingerprint(&self) -> Option<&str> {
        self.unknown_host_fingerprint.as_deref()
    }

    fn new(kind: SshFailureKind, detail: &'static str) -> Self {
        Self {
            kind,
            detail,
            unknown_host_fingerprint: None,
        }
    }

    fn io() -> Self {
        Self::new(SshFailureKind::Io, "ssh connection failed")
    }

    fn missing_capability() -> Self {
        Self::new(
            SshFailureKind::MissingCapability,
            "openssh or the private askpass helper is unavailable",
        )
    }

    fn changed_host_key() -> Self {
        Self::new(
            SshFailureKind::ChangedHostKey,
            "the remote host key changed and was refused",
        )
    }

    fn unknown_host(fingerprint: Option<String>) -> Self {
        Self {
            kind: SshFailureKind::UnknownHost,
            detail: "the remote host key is not in the app trust file",
            unknown_host_fingerprint: fingerprint,
        }
    }

    fn auth_failed() -> Self {
        Self::new(SshFailureKind::AuthFailed, "ssh authentication failed")
    }

    fn timeout() -> Self {
        Self::new(SshFailureKind::Timeout, "ssh connection timed out")
    }

    fn cancelled() -> Self {
        Self::new(SshFailureKind::Cancelled, "ssh connection cancelled")
    }

    fn update_required() -> Self {
        Self::new(
            SshFailureKind::UpdateRequired,
            "the installed companion must be updated with explicit approval",
        )
    }
}

impl fmt::Debug for SshFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshFailure")
            .field("kind", &self.kind)
            .field("detail", &self.detail)
            .field(
                "unknown_host_fingerprint",
                &self.unknown_host_fingerprint.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl fmt::Display for SshFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.detail)
    }
}

impl std::error::Error for SshFailure {}

/// Fixed marker emitted by the stage script when artifacts under the same
/// version differ and no approved update was given. It carries no path or hash.
const STAGE_UPDATE_REQUIRED_MARKER: &str = "MADE-STAGE-UPDATE-REQUIRED";
/// Fixed marker emitted when promotion and immediate restoration both failed.
/// The preserved original stays at the app-owned replacement path for the next
/// attempt. It carries no path or hash and is classified as update-required.
const STAGE_RECOVERY_REQUIRED_MARKER: &str = "MADE-STAGE-RECOVERY-REQUIRED";

/// Classify bounded OpenSSH diagnostic text. The returned value does not retain
/// the input. Callers must drop the original bytes.
pub fn classify_ssh_stderr(bytes: &[u8]) -> SshFailure {
    let text = String::from_utf8_lossy(bytes);
    let lower = text.to_ascii_lowercase();
    if is_changed_host_key(&lower) {
        return SshFailure::changed_host_key();
    }
    if is_unknown_host(&lower) {
        return SshFailure::unknown_host(extract_fingerprint(&text));
    }
    if is_auth_failed(&lower) {
        return SshFailure::auth_failed();
    }
    if text.contains(STAGE_UPDATE_REQUIRED_MARKER) || text.contains(STAGE_RECOVERY_REQUIRED_MARKER)
    {
        return SshFailure::update_required();
    }
    if is_timeout_text(&lower) {
        return SshFailure::timeout();
    }
    SshFailure::io()
}

/// Whether this askpass prompt is a changed-key confirmation.
///
/// An unknown-host prompt is not a changed key and may be shown to the user.
/// Remove one app-owned known_hosts entry after an explicit user "forget".
///
/// The next connection is treated as an unknown host and shows the new
/// fingerprint for explicit approval. Only `known_hosts` is touched; the
/// user's personal trust file is never opened. `host` is validated so it
/// cannot be interpreted as an option or a wildcard pattern.
pub fn forget_known_host(
    ssh_keygen: &Path,
    known_hosts: &Path,
    host: &str,
    port: u16,
) -> io::Result<()> {
    if host.is_empty()
        || host.len() > 255
        || host.starts_with('-')
        || host.chars().any(|value| {
            value.is_control()
                || value.is_whitespace()
                || matches!(value, '*' | '?' | '[' | ']' | '#' | '!' | '"' | '\'' | '\\')
        })
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid host"));
    }
    if !known_hosts.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "known_hosts must be absolute",
        ));
    }
    if !known_hosts.exists() {
        return Ok(());
    }
    let target = if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    };
    let mut child = Command::new(ssh_keygen)
        .arg("-R")
        .arg(&target)
        .arg("-f")
        .arg(known_hosts)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait()? {
            Some(status) if status.success() => break,
            Some(_) => return Err(io::Error::other("ssh-keygen failed")),
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "ssh-keygen timed out",
                ));
            }
        }
    }
    // ssh-keygen -R keeps `<known_hosts>.old`; this copy is app-owned.
    let mut backup = known_hosts.as_os_str().to_os_string();
    backup.push(".old");
    let _ = fs::remove_file(PathBuf::from(backup));
    Ok(())
}

pub fn prompt_is_changed_host_key(prompt: &str) -> bool {
    is_changed_host_key(&prompt.to_ascii_lowercase())
}

/// A changed host key is never accepted, including by `yes` or by echoing the
/// fingerprint. Any supplied answer is refused; cancellation sends nothing.
pub fn changed_host_key_answer_refused(prompt: &str, answer: &[u8]) -> bool {
    prompt_is_changed_host_key(prompt) && !answer.is_empty()
}

/// Parse `OpenSSH_8.4p1` or `OpenSSH_for_Windows_9.5p1`. The input is not retained.
pub fn parse_openssh_version(text: &str) -> Option<(u32, u32)> {
    let rest = text.split_once("OpenSSH_")?.1;
    let rest = rest.strip_prefix("for_Windows_").unwrap_or(rest);
    let major_end = rest.find(|c: char| !c.is_ascii_digit())?;
    if major_end == 0 {
        return None;
    }
    let major = rest[..major_end].parse().ok()?;
    let rest = rest[major_end..].strip_prefix('.')?;
    let minor_end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if minor_end == 0 {
        return None;
    }
    let minor = rest[..minor_end].parse().ok()?;
    Some((major, minor))
}

/// OpenSSH 8.4 or newer, including the Windows build.
pub fn openssh_version_supported(text: &str) -> bool {
    matches!(
        parse_openssh_version(text),
        Some((major, minor)) if major > 8 || (major == 8 && minor >= 4)
    )
}

fn is_changed_host_key(lower: &str) -> bool {
    lower.contains("remote host identification has changed")
        || lower.contains("possible dns spoofing")
        || lower.contains("offending key")
        || lower.contains("differs from the key")
        || (lower.contains("host key") && lower.contains("has changed"))
}

fn is_unknown_host(lower: &str) -> bool {
    lower.contains("authenticity of host")
        || lower.contains("can't be established")
        || lower.contains("cannot be established")
        || lower.contains("host key verification failed")
}

fn is_auth_failed(lower: &str) -> bool {
    lower.contains("permission denied")
        || lower.contains("authentication failed")
        || lower.contains("too many authentication failures")
}

fn is_timeout_text(lower: &str) -> bool {
    lower.contains("timed out") || lower.contains("timeout")
}

fn extract_fingerprint(text: &str) -> Option<String> {
    for marker in ["SHA256:", "MD5:"] {
        let Some(start) = text.find(marker) else {
            continue;
        };
        let token: String = text[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '+' | '/' | '=' | '-'))
            .take(128)
            .collect();
        if token.len() > marker.len() + 8 {
            return Some(token);
        }
    }
    None
}

/// Only protocol stdout is exposed. Stderr is classified and discarded: it can
/// contain credentials, proxy diagnostics and hostile server-controlled text.
pub enum SshEvent {
    Data(Vec<u8>),
    Eof,
    Failed(SshFailure),
    Exited(Option<i32>),
    TimedOut,
}
impl fmt::Debug for SshEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data(v) => write!(f, "SshEvent::Data(<{} bytes>)", v.len()),
            Self::Eof => f.write_str("SshEvent::Eof"),
            Self::Failed(failure) => write!(f, "SshEvent::Failed({failure:?})"),
            Self::Exited(c) => write!(f, "SshEvent::Exited({c:?})"),
            Self::TimedOut => f.write_str("SshEvent::TimedOut"),
        }
    }
}

struct StderrSlot {
    done: bool,
    terminal: bool,
    failure: Option<SshFailure>,
}

pub struct SshProcess {
    child: Option<Child>,
    input: Option<SyncSender<Vec<u8>>>,
    output: Receiver<SshEvent>,
    prompts: AskpassBroker,
    stopped: Arc<AtomicBool>,
    deadline: Option<Instant>,
    finished: bool,
    held_eof: bool,
    pending: Option<SshEvent>,
    stderr: Arc<Mutex<StderrSlot>>,
    changed_prompt: bool,
    #[cfg(windows)]
    job: Option<crate::windows::ProcessJob>,
}
impl fmt::Debug for SshProcess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SshProcess(<private>)")
    }
}
impl SshProcess {
    /// Call on a worker, not during UI drawing. Missing capabilities fail closed.
    ///
    /// On Windows this uses the installed OpenSSH client, a private named-pipe
    /// askpass helper, and a kill-on-close job. It does not return a blind
    /// unsupported error and it does not disable host-key checking.
    pub fn spawn(options: SshOptions) -> Result<Self, SshFailure> {
        ensure_openssh_version(&options.executable)?;
        let args = options.argv().map_err(|_| SshFailure::io())?;
        if !options.askpass_executable.is_absolute() || !options.askpass_executable.is_file() {
            return Err(SshFailure::missing_capability());
        }
        ensure_trust_file(&options)?;
        let prompts = AskpassBroker::new(&options.private_directory, Duration::from_secs(300))
            .map_err(|_| SshFailure::missing_capability())?;
        let mut command = Command::new(&options.executable);
        command
            .args(args)
            .env("SSH_ASKPASS", &options.askpass_executable)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("MADE_ASKPASS_SOCKET", prompts.socket_path())
            .env("DISPLAY", "made-private-askpass")
            .env_remove("SSH_ASKPASS_PROMPT")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        }
        let mut child = command.spawn().map_err(|_| SshFailure::io())?;
        #[cfg(windows)]
        let job = crate::windows::ProcessJob::assign(child.id()).ok_or_else(SshFailure::io)?;
        let mut stdin = child.stdin.take().ok_or_else(SshFailure::io)?;
        let mut stdout = child.stdout.take().ok_or_else(SshFailure::io)?;
        let stderr = child.stderr.take().ok_or_else(SshFailure::io)?;
        let (input, rx) = mpsc::sync_channel::<Vec<u8>>(SSH_QUEUE_CHUNKS);
        let (tx, output) = mpsc::sync_channel(SSH_QUEUE_CHUNKS);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let errors = tx.clone();
        let stderr_slot = Arc::new(Mutex::new(StderrSlot {
            done: false,
            terminal: false,
            failure: None,
        }));
        let stderr_state = stderr_slot.clone();
        thread::spawn(move || {
            while let Ok(bytes) = rx.recv() {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                if stdin.write_all(&bytes).and_then(|_| stdin.flush()).is_err() {
                    let _ = errors.try_send(SshEvent::Failed(SshFailure::io()));
                    break;
                }
            }
        });
        thread::spawn(move || {
            let mut buffer = vec![0; SSH_CHUNK_BYTES];
            loop {
                match stdout.read(&mut buffer) {
                    Ok(0) => {
                        let _ = tx.send(SshEvent::Eof);
                        break;
                    }
                    Ok(n) => {
                        if tx.send(SshEvent::Data(buffer[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        let _ = tx.send(SshEvent::Failed(SshFailure::io()));
                        break;
                    }
                }
            }
        });
        thread::spawn(move || drain_stderr(stderr, &stderr_state));
        Ok(Self {
            child: Some(child),
            input: Some(input),
            output,
            prompts,
            stopped,
            deadline: options.timeout.map(|deadline| Instant::now() + deadline),
            finished: false,
            held_eof: false,
            pending: None,
            stderr: stderr_slot,
            changed_prompt: false,
            #[cfg(windows)]
            job: Some(job),
        })
    }
    pub fn poll_prompt(&mut self) -> Option<PromptRequest> {
        let request = self.prompts.poll()?;
        if prompt_is_changed_host_key(&request.text) {
            self.changed_prompt = true;
            self.pending = Some(SshEvent::Failed(SshFailure::changed_host_key()));
            self.stop_child();
            return None;
        }
        Some(request)
    }
    pub fn answer_prompt(&mut self, id: u64, answer: Option<PromptAnswer>) -> io::Result<()> {
        if answer.is_none() {
            let _ = self.prompts.respond(id, None);
            self.pending = Some(SshEvent::Failed(SshFailure::cancelled()));
            self.stop_child();
            return Ok(());
        }
        // A changed key is never accepted, including by "yes" or by repeating
        // the fingerprint. Nothing is written to the askpass helper.
        if self.changed_prompt {
            self.pending = Some(SshEvent::Failed(SshFailure::changed_host_key()));
            self.stop_child();
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "changed host key answer refused",
            ));
        }
        self.prompts.respond(id, answer)
    }
    /// One bounded atomic enqueue. No partial writes on WouldBlock.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > SSH_CHUNK_BYTES {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        self.input
            .as_ref()
            .ok_or(io::ErrorKind::BrokenPipe)?
            .try_send(bytes.to_vec())
            .map_err(|e| match e {
                TrySendError::Full(_) => io::ErrorKind::WouldBlock.into(),
                TrySendError::Disconnected(_) => io::ErrorKind::BrokenPipe.into(),
            })
    }
    pub fn poll(&mut self) -> Option<SshEvent> {
        if let Some(event) = self.pending.take() {
            self.finished = true;
            return Some(event);
        }
        if self.finished {
            return None;
        }
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.stop_child();
            self.finished = true;
            return Some(SshEvent::TimedOut);
        }
        if self.stderr_is_terminal() {
            let failure = self
                .take_failure()
                .unwrap_or_else(SshFailure::changed_host_key);
            self.stop_child();
            self.finished = true;
            return Some(SshEvent::Failed(failure));
        }
        if let Ok(event) = self.output.try_recv() {
            match event {
                SshEvent::Eof => self.held_eof = true,
                SshEvent::Failed(_) => return Some(event),
                other => return Some(other),
            }
        }
        match self.child.as_mut()?.try_wait() {
            Ok(Some(status)) => {
                self.wait_for_stderr();
                self.input.take();
                self.prompts.cancel();
                self.finished = true;
                if status.success() && !self.stderr_is_terminal() {
                    return Some(if self.held_eof {
                        SshEvent::Eof
                    } else {
                        SshEvent::Exited(status.code())
                    });
                }
                Some(SshEvent::Failed(
                    self.take_failure().unwrap_or_else(SshFailure::io),
                ))
            }
            Err(_) => {
                self.stop_child();
                self.finished = true;
                Some(SshEvent::Failed(SshFailure::io()))
            }
            Ok(None) => None,
        }
    }
    /// True once `poll` has returned this process's terminal event. Later polls
    /// report `None`, so callers that keep a non-terminal `Failed(Io)` can stop
    /// instead of spinning until their own deadline.
    pub fn is_finished(&self) -> bool {
        self.finished
    }
    /// Drop only the host attachment; never sends remote Close or signals the
    /// detached supervisor.
    pub fn cancel(&mut self) {
        if self.pending.is_none() {
            self.pending = Some(SshEvent::Failed(SshFailure::cancelled()));
        }
        self.stop_child();
    }

    fn stop_child(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.input.take();
        self.prompts.cancel();
        let (_, empty) = mpsc::sync_channel(1);
        self.output = empty;
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            job.terminate();
        }
        if let Some(mut child) = self.child.take() {
            signal_owned_process_group(child.id());
            let _ = child.kill();
            thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }

    fn stderr_is_terminal(&self) -> bool {
        self.stderr
            .lock()
            .map(|slot| slot.terminal)
            .unwrap_or(false)
    }

    fn take_failure(&self) -> Option<SshFailure> {
        self.stderr.lock().ok()?.failure.take()
    }

    fn wait_for_stderr(&self) {
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if self.stderr.lock().map(|slot| slot.done).unwrap_or(true) {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}

fn ensure_openssh_version(executable: &std::path::Path) -> Result<(), SshFailure> {
    if !executable.is_absolute() || !executable.is_file() {
        return Err(SshFailure::missing_capability());
    }
    let mut command = Command::new(executable);
    command
        .arg("-V")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|_| SshFailure::missing_capability())?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(output) = stdout.take() {
            let _ = output.take(256).read_to_end(&mut bytes);
        }
        if let Some(output) = stderr.take() {
            let mut extra = Vec::new();
            let _ = output.take(256).read_to_end(&mut extra);
            bytes.extend(extra);
        }
        bytes
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    let finished = loop {
        match child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => break false,
        }
    };
    if !finished {
        signal_owned_process_group(child.id());
        let _ = child.kill();
        let _ = child.wait();
        let _ = reader.join();
        return Err(SshFailure::missing_capability());
    }
    let bytes = reader.join().unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    if openssh_version_supported(&text) {
        Ok(())
    } else {
        Err(SshFailure::missing_capability())
    }
}

fn ensure_trust_file(options: &SshOptions) -> Result<(), SshFailure> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
        let parent = options.known_hosts.parent().ok_or_else(SshFailure::io)?;
        let metadata = fs::symlink_metadata(parent).map_err(|_| SshFailure::io())?;
        if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(SshFailure::io());
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&options.known_hosts)
            .map_err(|_| SshFailure::io())?;
        let trust = file.metadata().map_err(|_| SshFailure::io())?;
        if !trust.is_file()
            || trust.permissions().mode() & 0o077 != 0
            || trust.uid() != metadata.uid()
            || trust.nlink() != 1
        {
            return Err(SshFailure::io());
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        let parent = options.known_hosts.parent().ok_or_else(SshFailure::io)?;
        let metadata = fs::symlink_metadata(parent).map_err(|_| SshFailure::io())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(SshFailure::io());
        }
        match fs::symlink_metadata(&options.known_hosts) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => Err(SshFailure::io()),
            Ok(_) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&options.known_hosts)
                    .map_err(|_| SshFailure::io())?;
                Ok(())
            }
            Err(_) => Err(SshFailure::io()),
        }
    }
}

fn drain_stderr(mut stderr: impl Read, slot: &Mutex<StderrSlot>) {
    let mut raw = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if raw.len() < 8192 {
                    let take = n.min(8192 - raw.len());
                    raw.extend_from_slice(&chunk[..take]);
                }
                if classify_ssh_stderr(&raw).kind() == SshFailureKind::ChangedHostKey {
                    if let Ok(mut guard) = slot.lock() {
                        guard.terminal = true;
                        guard.failure = Some(SshFailure::changed_host_key());
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    let failure = classify_ssh_stderr(&raw);
    drop(raw);
    if let Ok(mut guard) = slot.lock() {
        guard.done = true;
        let keep_changed =
            guard.failure.as_ref().map(SshFailure::kind) == Some(SshFailureKind::ChangedHostKey);
        if !keep_changed && failure.kind() != SshFailureKind::Io {
            guard.failure = Some(failure);
        } else if guard.failure.is_none() {
            guard.failure = Some(SshFailure::io());
        }
        guard.terminal =
            guard.failure.as_ref().map(SshFailure::kind) == Some(SshFailureKind::ChangedHostKey);
    }
}

fn signal_owned_process_group(pid: u32) {
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return;
        };
        if pid <= 1 {
            return;
        }
        // SAFETY: pid is the direct child started with process_group(0). The
        // negative value names that group. ESRCH is expected if it already
        // exited. This does not signal the detached supervisor session.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
            libc::kill(pid, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}
impl Drop for SshProcess {
    fn drop(&mut self) {
        self.cancel();
    }
}
