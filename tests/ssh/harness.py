#!/usr/bin/env python3
"""Disposable SSH fixtures. Never reads personal SSH configuration.

tmux is not a prerequisite and is never installed or executed. Rust and system
packages are never installed on the host or the remote.
"""
import base64
import getpass
import hashlib
import os
import pathlib
import re
import shlex
import shutil
import socket
import subprocess
import tempfile

BASE = pathlib.Path('/private/var/folders/qp/v8kk3jkj5ssb7g2_jj4gnkn80000gn/T/opencode')
PREREQUISITES = ('ssh', 'sshd', 'ssh-keygen')
GLIBC_CEILING = (2, 39)


class PrerequisiteError(RuntimeError):
    pass


def tools():
    return {name: shutil.which(name) for name in
            ('ssh', 'sshd', 'ssh-keygen', 'ssh-keyscan', 'opencode', 'docker', 'podman',
             'git', 'cargo', 'rustc', 'rustup', 'python3')}


def run(args, **kwargs):
    return subprocess.run(args, check=True, stdin=subprocess.DEVNULL,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kwargs)


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
        check = subprocess.run(['sshd', '-t', '-f', str(config)], stdout=subprocess.PIPE,
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
                f'{getpass.getuser()}@127.0.0.1']

    def ssh_run(self, command, timeout=10):
        return run(self.ssh_args() + [command], timeout=timeout)

    def ssh_process(self, command):
        p = subprocess.Popen(self.ssh_args() + [command], stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.processes.append(p)
        return p

    def __exit__(self, *_):
        # Only Popen handles created by this instance, never a pidfile, process
        # group, or a user-supplied PID.
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
