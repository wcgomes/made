#!/usr/bin/env python3
"""Disposable SSH fixtures. Never reads personal SSH configuration.

tmux is not a prerequisite and is never installed or executed. Rust and system
packages are never installed on the host or the remote.
"""
import base64
import hashlib
import os
import pathlib
import pwd
import re
import shlex
import shutil
import signal
import socket
import subprocess
import tempfile
import time

# Fixtures live under a private base. CI supplies RUNNER_TEMP (or MADE_SSH_TEMP);
# local macOS runs keep the approved short opencode temp directory.
DEFAULT_BASE = pathlib.Path(
    '/private/var/folders/qp/v8kk3jkj5ssb7g2_jj4gnkn80000gn/T/opencode')
BASE = pathlib.Path(os.environ.get('MADE_SSH_TEMP')
                    or os.environ.get('RUNNER_TEMP')
                    or DEFAULT_BASE)
PREREQUISITES = ('ssh', 'sshd', 'ssh-keygen')
GLIBC_CEILING = (2, 39)


class PrerequisiteError(RuntimeError):
    pass


def sshd_executable():
    """Resolve sshd even when the caller's PATH omits the sbin directories."""
    candidates = [os.environ.get('MADE_SSHD'), shutil.which('sshd'),
                  '/usr/sbin/sshd', '/usr/local/sbin/sshd', '/opt/homebrew/sbin/sshd']
    for candidate in candidates:
        if candidate and os.path.isfile(candidate) and os.access(candidate, os.X_OK):
            return candidate
    return None


def tools():
    found = {name: shutil.which(name) for name in
             ('ssh', 'ssh-keygen', 'ssh-keyscan', 'opencode', 'docker', 'podman',
              'git', 'cargo', 'rustc', 'rustup', 'python3')}
    found['sshd'] = sshd_executable()
    return found


def local_user():
    """The account sshd authenticates; never assume an env-provided user name."""
    return pwd.getpwuid(os.getuid()).pw_name


def run(args, **kwargs):
    return subprocess.run(args, check=True, stdin=subprocess.DEVNULL,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kwargs)


def process_table():
    """Return (pid, argv) rows without matching process names broadly."""
    result = run(['ps', '-axo', 'pid=,command=', '-ww'])
    rows = []
    for line in result.stdout.decode(errors='replace').splitlines():
        fields = line.strip().split(None, 1)
        if len(fields) != 2:
            continue
        try:
            rows.append((int(fields[0]), shlex.split(fields[1])))
        except ValueError:
            continue
    return rows


def owned_companion_pids(root):
    """Match only this instance's exact staged executable and companion mode."""
    executable = str(pathlib.Path(root) / 'companion' / 'made-remote')
    return {pid for pid, argv in process_table()
            if len(argv) == 2 and argv[0] == executable
            and argv[1] in ('--stdio', '--supervisor')}


def wait_for_owned_companions(root, timeout):
    deadline = time.monotonic() + timeout
    while True:
        remaining = owned_companion_pids(root)
        if not remaining or time.monotonic() >= deadline:
            return remaining
        time.sleep(min(0.05, max(0, deadline - time.monotonic())))


def cleanup_owned_companions(root, term_timeout=3, kill_timeout=3):
    """Reap only companions staged under root; preserve root if any survive."""
    for pid in sorted(owned_companion_pids(root)):
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    remaining = wait_for_owned_companions(root, term_timeout)
    for pid in sorted(remaining):
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    remaining = wait_for_owned_companions(root, kill_timeout)
    if remaining:
        raise RuntimeError('owned companion processes survived cleanup; temp root preserved; '
                           f'PIDs: {sorted(remaining)}')


class Harness:
    def __enter__(self):
        missing = [x for x in PREREQUISITES if not tools()[x]]
        if missing:
            raise PrerequisiteError('missing: ' + ', '.join(missing))
        if not BASE.is_dir():
            raise PrerequisiteError(f'approved temporary base unavailable: {BASE}')
        self.temp = tempfile.TemporaryDirectory(prefix='made-ssh-', dir=BASE)
        self.root = pathlib.Path(self.temp.name)
        self.processes = []
        try:
            self.setup()
        except BaseException:
            self.__exit__(None, None, None)
            raise
        return self

    def setup(self):
        self.root.chmod(0o700)
        for name in ('home', 'workspace', 'tmp'):
            (self.root / name).mkdir(mode=0o700)
        for name in ('host', 'client'):
            run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(self.root / name)])
        (self.root / 'authorized_keys').write_bytes((self.root / 'client.pub').read_bytes())
        (self.root / 'authorized_keys').chmod(0o600)
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            self.port = sock.getsockname()[1]
        public = (self.root / 'host.pub').read_text().split()
        (self.root / 'known_hosts').write_text(f'[127.0.0.1]:{self.port} {public[0]} {public[1]}\n')
        config = self.root / 'sshd_config'
        config.write_text('\n'.join([
            f'Port {self.port}', 'ListenAddress 127.0.0.1',
            f'HostKey {self.root / "host"}', f'PidFile {self.root / "sshd.pid"}',
            f'AuthorizedKeysFile {self.root / "authorized_keys"}',
            'StrictModes no', 'PasswordAuthentication no', 'KbdInteractiveAuthentication no',
            'PubkeyAuthentication yes', 'PermitRootLogin no', 'UsePAM no',
            'AllowAgentForwarding no', 'AllowTcpForwarding no', 'PermitTunnel no',
            'PermitUserEnvironment no', 'PermitUserRC no',
            f'SetEnv HOME={self.root / "home"} ZDOTDIR={self.root / "home"} '
            f'BASH_ENV={self.root / "home" / "empty"} ENV={self.root / "home" / "empty"}',
            'PrintMotd no', 'LogLevel ERROR',
        ]) + '\n')
        check = subprocess.run([tools()['sshd'], '-t', '-f', str(config)],
                               stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, stdin=subprocess.DEVNULL)
        if check.returncode:
            raise PrerequisiteError('sshd configuration/preflight failed: ' +
                                    check.stderr.decode(errors='replace').strip())
        log = open(self.root / 'sshd.log', 'wb')
        try:
            self.daemon = subprocess.Popen([tools()['sshd'], '-D', '-e', '-f', str(config)],
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        finally:
            log.close()
        self.processes.append(self.daemon)
        # The real SSH handshake is the readiness condition; it also verifies ownership
        # of the selected port and pinned host key if another process won the port race.
        try:
            result = self.ssh_run('printf ready', timeout=10)
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as exc:
            detail = (self.root / 'sshd.log').read_text(errors='replace')
            raise PrerequisiteError(f'isolated sshd/handshake failed: {exc}; {detail.strip()}') from exc
        if result.stdout != b'ready':
            raise PrerequisiteError('unexpected SSH readiness response')
        isolated = self.ssh_run('printf "%s\\n%s" "$HOME" "$ZDOTDIR"')
        expected = str(self.root / 'home').encode()
        if isolated.stdout != expected + b'\n' + expected:
            raise PrerequisiteError('sshd did not enforce isolated HOME/ZDOTDIR')

    def ssh_args(self):
        return ['ssh', '-F', '/dev/null', '-T', '-p', str(self.port), '-i', str(self.root / 'client'),
                '-o', 'IdentitiesOnly=yes', '-o', 'IdentityAgent=none',
                '-o', f'UserKnownHostsFile={self.root / "known_hosts"}',
                '-o', 'GlobalKnownHostsFile=/dev/null', '-o', 'StrictHostKeyChecking=yes',
                '-o', 'BatchMode=yes', '-o', 'ForwardAgent=no', '-o', 'ClearAllForwardings=yes',
                '-o', 'PermitLocalCommand=no', '-o', 'ConnectTimeout=5', '-o', 'ConnectionAttempts=5',
                f'{local_user()}@127.0.0.1']

    def ssh_run(self, command, timeout=10):
        return run(self.ssh_args() + [command], timeout=timeout)

    def ssh_run_input(self, command, input_bytes, timeout=30):
        """Run one fixed remote command with length-delimited stdin bytes."""
        return subprocess.run(
            self.ssh_args() + [command], input=input_bytes, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, timeout=timeout, check=False)

    def ssh_process(self, command):
        p = subprocess.Popen(self.ssh_args() + [command], stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.processes.append(p)
        return p

    def __exit__(self, *_):
        try:
            # Close only Popen handles created by this instance. Closing the SSH
            # frontends first lets their remote stdio peers exit where possible.
            for process in reversed(self.processes):
                if process.poll() is None:
                    if process.stdin:
                        try:
                            process.stdin.close()
                        except OSError:
                            pass
                    process.terminate()
                    try:
                        process.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
                for stream in (process.stdin, process.stdout, process.stderr):
                    if stream:
                        stream.close()
        finally:
            # Detached stdio/supervisor processes have no Popen handle. Re-scan
            # by this root's exact staged argv, and never delete a live root.
            cleanup_owned_companions(self.root)
            self.temp.cleanup()


def fixtures(root):
    workspace = root / 'workspace'
    (workspace / 'nested space').mkdir()
    (workspace / 'nested space' / 'olá.txt').write_text('initial\n')
    (workspace / 'binary').write_bytes(b'\x00binary')
    (workspace / 'escape').symlink_to(root / 'home', target_is_directory=True)
    if tools()['git']:
        run(['git', 'init', '-q', str(workspace)])
        run(['git', '-C', str(workspace), '-c', 'user.name=Fixture', '-c',
             'user.email=fixture@example.invalid', 'commit', '--allow-empty', '-qm', 'fixture'])
        (workspace / 'untracked').write_text('fixture\n')
    return workspace


def fingerprint(key):
    blob = base64.b64decode(key, validate=True)
    return 'SHA256:' + base64.b64encode(hashlib.sha256(blob).digest()).decode().rstrip('=')


def matching_host_keys(scan_stdout, expected):
    matches = []
    for line in scan_stdout.decode('ascii', errors='replace').splitlines():
        parts = line.split()
        if len(parts) == 3 and fingerprint(parts[2]) == expected:
            matches.append(line)
    return matches


def remote_proof_directory(path):
    if not re.fullmatch(r'/tmp/made-proof\.[A-Za-z0-9]+', path or ''):
        raise ValueError('refusing remote path outside the proof directory')
    return path


def linux_arm64_elf(path, glibc_ceiling=GLIBC_CEILING):
    """None when path is a glibc Linux aarch64 ELF this fixture can run."""
    data = pathlib.Path(path).read_bytes()
    if data[:4] in (b'\xcf\xfa\xed\xfe', b'\xfe\xed\xfa\xcf', b'\xce\xfa\xed\xfe', b'\xfe\xed\xfa\xce'):
        return 'Mach-O, not Linux ELF'
    if data[:4] != b'\x7fELF':
        return 'not ELF'
    if len(data) < 64 or data[4] != 2 or data[5] != 1:
        return 'not a little-endian ELF64'
    if int.from_bytes(data[18:20], 'little') != 183:
        return 'ELF machine is not aarch64'
    phoff = int.from_bytes(data[32:40], 'little')
    phentsize = int.from_bytes(data[54:56], 'little')
    phnum = int.from_bytes(data[56:58], 'little')
    if phentsize < 56 or phnum == 0 or phoff + phentsize * phnum > len(data):
        return 'ELF has no readable program headers'
    interp = None
    for index in range(phnum):
        off = phoff + index * phentsize
        if int.from_bytes(data[off:off + 4], 'little') != 1:
            continue
        p_offset = int.from_bytes(data[off + 8:off + 16], 'little')
        p_filesz = int.from_bytes(data[off + 32:off + 40], 'little')
        if p_filesz == 0 or p_filesz > 4096 or p_offset + p_filesz > len(data):
            return 'invalid PT_INTERP'
        interp = data[p_offset:p_offset + p_filesz].split(b'\0', 1)[0].decode('ascii', 'replace')
        break
    if not interp or 'musl' in interp or 'ld-linux-aarch64' not in interp:
        return f'interpreter {interp or "absent"} is not glibc ld-linux-aarch64'
    too_new = sorted({
        (int(major), int(minor))
        for major, minor in re.findall(rb'GLIBC_(\d+)\.(\d+)', data)
        if (int(major), int(minor)) > glibc_ceiling
    })
    if too_new:
        text = ', '.join(f'{major}.{minor}' for major, minor in too_new[:8])
        return f'requires GLIBC newer than {glibc_ceiling[0]}.{glibc_ceiling[1]}: {text}'
    return None


def cross_build_blockage():
    """Read-only toolchain check. Never installs a target, linker, or package."""
    missing = []
    targets = []
    if not tools()['rustc'] or not tools()['rustup']:
        missing.append('rustup/rustc absent')
    else:
        listed = subprocess.run(
            ['rustup', 'target', 'list', '--installed'],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            timeout=30, check=False)
        if listed.returncode:
            missing.append('rustup target list --installed failed')
        else:
            targets = [line.strip() for line in listed.stdout.decode().splitlines() if line.strip()]
            if 'aarch64-unknown-linux-gnu' not in targets:
                missing.append('rustup target aarch64-unknown-linux-gnu is not installed')
    for tool in ('aarch64-linux-gnu-gcc', 'aarch64-linux-gnu-ld', 'zig', 'cargo-zigbuild', 'cross'):
        if not shutil.which(tool):
            missing.append(tool + ' absent')
    return {
        'installed_targets': targets,
        'missing': missing,
        'blocked': bool(missing),
        'refused': 'did not install a Rust target, linker, zig, or package',
    }


def cargo_lock_busy():
    listed = subprocess.run(
        ['pgrep', '-x', 'cargo'], stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False)
    rustc = subprocess.run(
        ['pgrep', '-x', 'rustc'], stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False)
    return bool(listed.stdout.strip() or rustc.stdout.strip())


def confirm_authorized_host(expected):
    """Scan the authorized port into a temporary trust file and delete it."""
    if not expected.startswith('SHA256:') or len(expected) < 16:
        raise PrerequisiteError('refusing fingerprint that is not an out-of-band SHA256 value')
    if not BASE.is_dir():
        raise PrerequisiteError(f'approved temporary base unavailable: {BASE}')
    if not tools()['ssh-keyscan']:
        raise PrerequisiteError('missing: ssh-keyscan')
    personal = pathlib.Path.home() / '.ssh' / 'known_hosts'
    before = personal.stat().st_mtime_ns if personal.exists() else None
    scan = subprocess.run(
        ['ssh-keyscan', '-T', '5', '-p', '2222', '127.0.0.1'],
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        timeout=10, check=True)
    matches = matching_host_keys(scan.stdout, expected)
    with tempfile.TemporaryDirectory(prefix='made-ssh-trust-', dir=BASE) as directory:
        os.chmod(directory, 0o700)
        known = pathlib.Path(directory) / 'known_hosts'
        if matches:
            known.write_text('\n'.join(matches) + '\n')
            known.chmod(0o600)
    after = personal.stat().st_mtime_ns if personal.exists() else None
    if before != after:
        raise PrerequisiteError('personal known_hosts changed during host confirmation')
    if not matches:
        raise PrerequisiteError('host key does not match the explicitly supplied fingerprint')
    return len(matches)


def wipe_secret(path):
    path = pathlib.Path(path)
    if not path.exists():
        return
    size = path.stat().st_size
    with path.open('r+b') as handle:
        handle.write(b'\0' * max(size, 1))
        handle.flush()
        os.fsync(handle.fileno())
    path.unlink()


class ContainerSession:
    """Password SSH to the authorized container. The password file is removed."""

    def __init__(self, password):
        self._password = password
        self.temp = None
        self.root = None
        self.known = None
        self.askpass = None
        self.processes = []

    def __repr__(self):
        return 'ContainerSession(<redacted>)'

    def __enter__(self):
        if not self._password:
            raise PrerequisiteError('empty password refused')
        missing = [name for name in ('ssh', 'ssh-keyscan') if not tools()[name]]
        if missing:
            raise PrerequisiteError('missing: ' + ', '.join(missing))
        if not BASE.is_dir():
            raise PrerequisiteError(f'approved temporary base unavailable: {BASE}')
        self.temp = tempfile.TemporaryDirectory(prefix='made-container-', dir=BASE)
        self.root = pathlib.Path(self.temp.name)
        try:
            self.root.chmod(0o700)
            self.known = self.root / 'known_hosts'
            self.askpass = self.root / 'askpass'
            self.askpass.write_text('#!/bin/sh\nexec /bin/cat -- "$0.secret"\n')
            self.askpass.chmod(0o700)
            secret = pathlib.Path(str(self.askpass) + '.secret')
            secret.write_text(self._password if self._password.endswith('\n') else self._password + '\n')
            secret.chmod(0o600)
        except BaseException:
            self.__exit__(None, None, None)
            raise
        return self

    def write_trust(self, lines):
        self.known.write_text('\n'.join(lines) + '\n')
        self.known.chmod(0o600)

    def child_env(self):
        env = os.environ.copy()
        env['SSH_ASKPASS'] = str(self.askpass)
        env['SSH_ASKPASS_REQUIRE'] = 'force'
        env.pop('SSH_AUTH_SOCK', None)
        env.pop('SSH_AGENT_PID', None)
        env.setdefault('DISPLAY', ':0')
        return env

    def ssh_args(self):
        return ['ssh', '-F', '/dev/null', '-T', '-p', '2222',
                '-o', 'IdentityAgent=none', '-o', 'IdentitiesOnly=yes',
                '-o', 'PubkeyAuthentication=no', '-o', 'PreferredAuthentications=password',
                '-o', 'NumberOfPasswordPrompts=1', '-o', 'ConnectTimeout=5',
                '-o', 'ConnectionAttempts=1', '-o', 'HostKeyAlgorithms=ssh-ed25519',
                '-o', f'UserKnownHostsFile={self.known}',
                '-o', 'GlobalKnownHostsFile=/dev/null', '-o', 'StrictHostKeyChecking=yes',
                '-o', 'UpdateHostKeys=no', '-o', 'HashKnownHosts=no',
                '-o', 'ForwardAgent=no', '-o', 'ClearAllForwardings=yes',
                '-o', 'PermitLocalCommand=no', 'vscode@127.0.0.1']

    def ssh_run(self, command, timeout=30, input_bytes=None):
        return subprocess.run(
            self.ssh_args() + [command], input=input_bytes, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, timeout=timeout, check=True, env=self.child_env(),
            start_new_session=True)

    def ssh_process(self, command):
        process = subprocess.Popen(
            self.ssh_args() + [command], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, env=self.child_env(), start_new_session=True)
        self.processes.append(process)
        return process

    def upload(self, local, remote):
        quoted = shlex.quote(remote)
        self.ssh_run(
            f'umask 077; cat > {quoted} && chmod 700 {quoted}',
            input_bytes=pathlib.Path(local).read_bytes(), timeout=120)

    def make_private_dir(self):
        result = self.ssh_run(
            'umask 077; dir=$(mktemp -d /tmp/made-proof.XXXXXXXX); '
            'mkdir -m 700 "$dir/home" "$dir/data" "$dir/config" "$dir/state" '
            '"$dir/cache" "$dir/tmp" "$dir/workspace"; printf %s "$dir"')
        return remote_proof_directory(result.stdout.decode().strip())

    def cleanup_owned(self, root, shell_pid):
        if not root:
            return 'no remote directory'
        root = remote_proof_directory(root)
        pid = int(shell_pid or 0)
        script = (
            'set -eu\n'
            'root=$1\n'
            'pid=$2\n'
            'binary=$root/made-remote\n'
            'kill_owned() {\n'
            '  cmd=$(tr "\\0" " " < "/proc/$1/cmdline" 2>/dev/null || true)\n'
            '  case "$cmd" in\n'
            '    *"$binary"*)\n'
            '      cwd=$(readlink "/proc/$1/cwd" 2>/dev/null || true)\n'
            '      kill "$1" 2>/dev/null || true\n'
            '      case "$cwd" in\n'
            '        "$root"/*|/tmp/made-detach-*) rm -rf -- "$cwd" ;;\n'
            '      esac\n'
            '      ;;\n'
            '  esac\n'
            '}\n'
            'if [ "$pid" != 0 ] && [ -r "/proc/$pid/status" ]; then\n'
            '  ppid=$(awk \'/^PPid:/ {print $2}\' "/proc/$pid/status")\n'
            '  [ -z "$ppid" ] || kill_owned "$ppid"\n'
            '  [ ! -d "/proc/$pid" ] || kill "$pid" 2>/dev/null || true\n'
            'fi\n'
            'for dir in /proc/[0-9]*; do\n'
            '  kill_owned "${dir#/proc/}"\n'
            'done\n'
            'rm -rf -- "$root"\n'
            'if [ -e "$root" ]; then echo cleanup-unconfirmed; exit 1; fi\n'
            'echo cleanup-ok\n'
        )
        result = self.ssh_run(
            'sh -s -- ' + shlex.quote(root) + ' ' + str(pid),
            input_bytes=script.encode(), timeout=30)
        return result.stdout.decode().strip() or 'cleanup-ok'

    def __exit__(self, *_):
        for process in reversed(self.processes):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            for stream in (process.stdin, process.stdout, process.stderr):
                if stream:
                    stream.close()
        if self.askpass:
            wipe_secret(str(self.askpass) + '.secret')
        self._password = ''
        if self.temp:
            self.temp.cleanup()
            self.temp = None


# ── Remote Grok fixtures (SSH-10, task 2.42) ────────────────────────────────
#
# Everything below seeds files and fake executables inside the disposable
# root. Setup installs nothing, downloads nothing, and never runs the fake
# executables; PATH variants decide which of them detection can see. The Grok
# home is seeded at the spec default ~/.grok and GROK_HOME is never set by
# Made or by the harness.

GROK_CONFIG_SENTINEL = '# fixture grok configuration\n[model]\nname = "fixture"\n'

GROK_FIXTURE_SCRIPT = '''#!/bin/sh
# Disposable fixture grok. It records its argv and the inherited GROK_HOME,
# emulates the two Grok-home effects the product contract observes (an
# installed plugin copy and a session inventory row), and fails on demand.
# Setup never runs it and nothing outside the fixture root is touched.
argv_log='__ARGV_LOG__'
env_log='__ENV_LOG__'
printf '%s\\n' "$*" >> "$argv_log"
grok_home="${GROK_HOME:-$HOME/.grok}"
if [ "$1" = "plugin" ] && [ "$2" = "install" ]; then
  printf '%s\\n' "GROK_HOME=${GROK_HOME-}" >> "$env_log"
  if [ "${MADE_FIXTURE_INSTALL_FAIL:-0}" != "0" ]; then
    exit 1
  fi
  mkdir -p "$grok_home/installed-plugins" || exit 1
  rm -rf "$grok_home/installed-plugins/grok-made"
  cp -R "$3" "$grok_home/installed-plugins/grok-made" || exit 1
  exit 0
fi
if [ "$1" = "dashboard" ]; then
  printf '%s\\n' "GROK_HOME=${GROK_HOME-}" >> "$env_log"
  pid=$$
  if [ "${MADE_FIXTURE_SESSION:-0}" = "1" ]; then
    session="${MADE_FIXTURE_SESSION_ID:-grok-session-$pid}"
    printf '[{"pid":%s,"session_id":"%s","cwd":"%s","opened_at":"2026-01-01T00:00:00Z"}]\\n' \\
      "$pid" "$session" "${MADE_FIXTURE_CWD:-/fixture}" > "$grok_home/active_sessions.json" || exit 1
    printf '%s\\n' "MADE_FIXTURE_SESSION:$session"
  fi
  printf '%s\\n' "MADE_FIXTURE_GROK:$pid"
  exec sleep "${MADE_FIXTURE_LIFETIME:-300}"
fi
exit 0
'''


OPENCODE_FIXTURE_SCRIPT = '''#!/usr/bin/python3
# Fixture stand-in for a remote OpenCode install. `--version` answers fast for
# detection. When the supervisor injected the composed OpenCode configuration
# (OPENCODE_CLI_CONFIG_CONTENT), this fixture also plays the opencode-made
# plugin side of the loopback bridge contract: WebSocket handshake, hello, one
# state push, request_state replies, and heartbeat pongs - the same wire
# surface the real plugin inside a real OpenCode presents. Nothing is written
# outside the conversation; a missing or malformed config degrades to sleep.
import base64
import hashlib
import json
import os
import socket
import struct
import sys
import time

WS_GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11'


def made_options(env_value):
    config = json.loads(env_value)
    for plugin in config.get('plugins', []):
        made = (plugin.get('options') or {}).get('made') or {}
        if made.get('bridgeUrl') and made.get('token'):
            return made
    raise ValueError('no made bridge entry in the composed config')


def bridge_endpoint(url):
    if not url.startswith('ws://'):
        raise ValueError('bridge url is not ws://')
    rest = url[len('ws://'):]
    path = '/made'
    if '/' in rest:
        rest, _, suffix = rest.partition('/')
        path = '/' + suffix
    host, _, port = rest.rpartition(':')
    return host, int(port), path


def ws_connect(url):
    host, port, path = bridge_endpoint(url)
    sock = socket.create_connection((host, port), timeout=5)
    key = base64.b64encode(os.urandom(16)).decode()
    sock.sendall((
        'GET ' + path + ' HTTP/1.1\\r\\n'
        'Host: ' + host + ':' + str(port) + '\\r\\n'
        'Upgrade: websocket\\r\\n'
        'Connection: Upgrade\\r\\n'
        'Sec-WebSocket-Key: ' + key + '\\r\\n'
        'Sec-WebSocket-Version: 13\\r\\n'
        '\\r\\n').encode())
    response = bytearray()
    while b'\\r\\n\\r\\n' not in response:
        chunk = sock.recv(4096)
        if not chunk:
            raise ValueError('bridge handshake closed')
        response.extend(chunk)
        if len(response) > 8192:
            raise ValueError('bridge handshake too long')
    head, _, rest_bytes = bytes(response).partition(b'\\r\\n\\r\\n')
    status = head.split(b'\\r\\n', 1)[0]
    if b' 101 ' not in status + b' ':
        raise ValueError('bridge handshake rejected: ' + status[:64].decode('ascii', 'replace'))
    accept = base64.b64encode(hashlib.sha1(key.encode() + WS_GUID.encode()).digest()).decode()
    if accept.encode() not in head:
        raise ValueError('bridge accept key mismatch')
    return sock, rest_bytes


class Bridge:
    """Minimal RFC6455 text client; client frames are always masked."""

    def __init__(self, sock, seed):
        self.sock = sock
        self.buffer = bytearray(seed)

    def fill(self, count):
        while len(self.buffer) < count:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise EOFError('bridge closed')
            self.buffer.extend(chunk)

    def frame(self, opcode, payload):
        header = bytearray([0x80 | opcode])
        mask = os.urandom(4)
        length = len(payload)
        if length < 126:
            header.append(0x80 | length)
        elif length < 65536:
            header.append(0x80 | 126)
            header.extend(struct.pack('>H', length))
        else:
            header.append(0x80 | 127)
            header.extend(struct.pack('>Q', length))
        header.extend(mask)
        masked = bytes(byte ^ mask[index % 4]
                       for index, byte in enumerate(payload))
        self.sock.sendall(bytes(header) + masked)

    def send_text(self, text):
        self.frame(0x1, text.encode())

    def recv_text(self):
        while True:
            self.fill(2)
            opcode = self.buffer[0] & 0x0F
            length = self.buffer[1] & 0x7F
            offset = 2
            if length == 126:
                self.fill(offset + 2)
                length = struct.unpack('>H', bytes(self.buffer[offset:offset + 2]))[0]
                offset += 2
            elif length == 127:
                self.fill(offset + 8)
                length = struct.unpack('>Q', bytes(self.buffer[offset:offset + 8]))[0]
                offset += 8
            mask_key = b''
            if self.buffer[1] & 0x80:
                self.fill(offset + 4)
                mask_key = bytes(self.buffer[offset:offset + 4])
                offset += 4
            self.fill(offset + length)
            payload = bytes(self.buffer[offset:offset + length])
            del self.buffer[:offset + length]
            if mask_key:
                payload = bytes(byte ^ mask_key[index % 4]
                                for index, byte in enumerate(payload))
            if opcode == 0x9:
                self.frame(0xA, payload)
                continue
            if opcode == 0xA:
                continue
            if opcode == 0x8:
                code = struct.unpack('>H', payload[:2])[0] if len(payload) >= 2 else 0
                reason = payload[2:].decode('utf-8', 'replace')
                raise EOFError(f'bridge closed frame {code}: {reason}')
            if opcode in (0x1, 0x0):
                return payload.decode('utf-8', 'replace')
            raise ValueError('unexpected bridge frame opcode ' + str(opcode))


def serve(made):
    sock, seed = ws_connect(made['bridgeUrl'])
    sock.settimeout(2.0)
    bridge = Bridge(sock, seed)
    hello = {'type': 'hello', 'protocolVersion': 1, 'token': made['token'],
             'paneId': made.get('paneId', 'remote'), 'pluginVersion': '0.1.0',
             'opencodeVersion': '2.0.0', 'pid': os.getpid(), 'cwd': os.getcwd()}
    bridge.send_text(json.dumps(hello, separators=(',', ':')))
    revision = 0

    def state():
        nonlocal revision
        revision += 1
        return {'type': 'state', 'revision': revision,
                'activeSessionID': 'fixture-oc-1',
                'tabsEnabled': True,
                'sessions': [{'sessionID': 'fixture-oc-1',
                              'title': 'Fixture session', 'active': True,
                              'busy': False, 'attention': False, 'unread': False,
                              'promptPulse': 0, 'renaming': False,
                              'directory': '/fixture', 'subpath': None,
                              'projectID': None}]}

    while True:
        text = bridge.recv_text()
        try:
            message = json.loads(text)
        except ValueError:
            continue
        kind = message.get('type')
        if kind == 'welcome':
            bridge.send_text(json.dumps(state(), separators=(',', ':')))
        elif kind == 'request_state':
            bridge.send_text(json.dumps(state(), separators=(',', ':')))
        elif kind == 'ping':
            bridge.send_text(json.dumps({'type': 'pong',
                                         'nonce': message.get('nonce', '')},
                                        separators=(',', ':')))
        elif kind == 'error' and message.get('fatal'):
            return


def main():
    if len(sys.argv) > 1 and sys.argv[1] == '--version':
        print('opencode 2.0.0')
        return 0
    config = os.environ.get('OPENCODE_CLI_CONFIG_CONTENT')
    if config:
        try:
            serve(made_options(config))
            return 0
        except (OSError, EOFError, ValueError, KeyError):
            # A dead bridge or malformed config is a fixture failure; report it
            # on the PTY so harness evidence shows why the child exited.
            import traceback
            traceback.print_exc()
            return 1
    time.sleep(int(os.environ.get('MADE_FIXTURE_LIFETIME', '300')))
    return 0


if __name__ == '__main__':
    sys.exit(main())
'''


def grok_fixture_scripts(root):
    """Fake grok/python3/opencode executables plus their invocation logs.

    PATH variants pick which binaries are visible. The scripts execute only
    when a scenario drives them (launch, typed shell command, consented
    install); setup never runs them and they touch nothing outside the root.
    The fake opencode answers --version quickly because the real version
    probe is bounded and spawns only `opencode`. When the supervisor injected
    the composed OpenCode configuration, the fake opencode also plays the
    opencode-made plugin side of the loopback bridge contract: handshake,
    hello, one state push, request_state replies, and heartbeat pongs — the
    same wire surface the real plugin inside a real OpenCode presents.
    """
    grokbin = root / 'grokbin'
    pybin = root / 'pybin'
    ocbin = root / 'ocbin'
    for directory in (grokbin, pybin, ocbin):
        directory.mkdir(mode=0o700, exist_ok=True)
    argv_log = root / 'grok-argv.log'
    env_log = root / 'grok-env.log'
    script = (GROK_FIXTURE_SCRIPT
              .replace('__ARGV_LOG__', str(argv_log))
              .replace('__ENV_LOG__', str(env_log)))
    grok = grokbin / 'grok'
    grok.write_text(script)
    grok.chmod(0o755)
    python3 = pybin / 'python3'
    python3.write_text('#!/bin/sh\n'
                       '# Metadata-only detection target; the product never executes it.\n'
                       'exit 0\n')
    python3.chmod(0o755)
    opencode = ocbin / 'opencode'
    opencode.write_text(OPENCODE_FIXTURE_SCRIPT)
    opencode.chmod(0o755)
    return {'grok': grok, 'python3': python3, 'opencode': opencode,
            'grokbin': grokbin, 'pybin': pybin, 'ocbin': ocbin,
            'argv_log': argv_log, 'env_log': env_log}


def grok_home(root):
    """Seed the Grok home at the spec default ~/.grok. GROK_HOME stays unset so
    every scenario exercises the default resolution the spec names."""
    home = root / 'home' / '.grok'
    home.mkdir(mode=0o700, parents=True, exist_ok=True)
    (home / 'config.toml').write_text(GROK_CONFIG_SENTINEL)
    return home


def seed_grok_inventory(home, body):
    (home / 'active_sessions.json').write_text(body)


def seed_grok_status(home, lines):
    (home / 'made-grok-status.jsonl').write_text(''.join(line + '\n' for line in lines))


def seed_installed_grok_made(home, version):
    """A pre-installed grok-made copy, shaped like the official installer's."""
    directory = home / 'installed-plugins' / 'grok-made'
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    (directory / 'plugin.json').write_text(
        '{"name":"grok-made","version":"' + version + '"}\n')
    return directory


def seed_staged_grok_made(companion_dir, version):
    """The shipped plugins/grok-made layout beside the companion executable,
    including the executable hook bit real staging applies."""
    staged = companion_dir / 'plugins' / 'grok-made'
    (staged / 'hooks' / 'bin').mkdir(mode=0o700, parents=True, exist_ok=True)
    (staged / 'plugin.json').write_text(
        '{"name":"grok-made","version":"' + version + '"}\n')
    (staged / 'hooks' / 'hooks.json').write_text('{}\n')
    hook = staged / 'hooks' / 'bin' / 'made-grok-status'
    hook.write_text('#!/bin/sh\n# Fixture stand-in for the shipped status hook.\nexit 0\n')
    hook.chmod(0o755)
    return staged


def stage_shipped_opencode_made(companion_dir, package_source):
    """The complete shipped opencode-made plugin package beside the companion
    executable — the staged debug layout the companion's own path resolution
    prefers (`<install_root>/plugins/opencode-made` with package.json and
    tui.ts). Without it the companion composes no OpenCode integration and the
    OpenCode-precedence scenario would test a degraded surface. Files are
    copied read-only into the disposable root; node_modules and tests stay."""
    required = ('package.json', 'tui.ts')
    for name in required:
        if not (pathlib.Path(package_source) / name).is_file():
            raise PrerequisiteError(
                f'shipped opencode-made package incomplete: {package_source} '
                f'is missing {name}')
    staged = companion_dir / 'plugins' / 'opencode-made'
    staged.mkdir(mode=0o700, parents=True, exist_ok=True)
    for name in required + ('index.ts', 'README.md'):
        source = pathlib.Path(package_source) / name
        if source.is_file():
            target = staged / name
            shutil.copyfile(source, target)
            target.chmod(0o644)
    src = pathlib.Path(package_source) / 'src'
    if src.is_dir():
        (staged / 'src').mkdir(mode=0o700, exist_ok=True)
        for entry in sorted(src.iterdir()):
            if entry.is_file():
                target = staged / 'src' / entry.name
                shutil.copyfile(entry, target)
                target.chmod(0o644)
    return staged
