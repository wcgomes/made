#!/usr/bin/env python3
"""Real loopback SSH probe and protocol checkpoint, not native GUI certification.

    --frontend is the explicit remote command implementing the bootstrap; no executable is guessed.
It must consume v1 frames on stdin and emit only v1 frames on stdout.

The claude-parity mode drives the same entry points with a fixture claude
that fires the real staged native Claude Code hook (see its section below).

The grok-parity and grok-first-fail modes drive the real companion entry
points (--probe, --stdio frontend, --supervisor, --install-grok-plugin) over
the isolated loopback SSH harness with the task-2.42 Grok fixtures. Grok
activity itself is produced by fixture executables and seeded Grok-home files,
so these scenarios certify the companion/protocol surface only: not real grok,
not the desktop GUI (model-level tests live in crates/made-app), and not the
live SSH matrix.
"""
import argparse
import base64
import getpass
import hashlib
import json
import os
import pathlib
import platform
import re
import select
import selectors
import shlex
import shutil
import struct
import subprocess
import sys
import time

from harness import (
    ContainerSession, GROK_CONFIG_SENTINEL, Harness, PrerequisiteError,
    cargo_lock_busy, confirm_authorized_host, cross_build_blockage,
    fixtures, grok_fixture_scripts, grok_home, linux_arm64_elf,
    matching_host_keys, seed_grok_inventory, seed_grok_status,
    seed_installed_grok_made, seed_staged_grok_made, stage_shipped_opencode_made,
    tools,
)

LIMIT = 256 * 1024


def encode(frame):
    payload = json.dumps(frame, separators=(',', ':')).encode()
    if not 0 < len(payload) <= LIMIT:
        raise ValueError('invalid frame length')
    return struct.pack('>I', len(payload)) + payload


def read_exact(fd, count, deadline):
    data = bytearray()
    with selectors.DefaultSelector() as selector:
        selector.register(fd, selectors.EVENT_READ)
        while len(data) < count:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not selector.select(remaining):
                raise TimeoutError('protocol response deadline exceeded')
            chunk = os.read(fd, count - len(data))
            if not chunk:
                raise EOFError('frontend closed protocol stream')
            data.extend(chunk)
    return bytes(data)


def receive(fd, timeout=10):
    deadline = time.monotonic() + timeout
    size = struct.unpack('>I', read_exact(fd, 4, deadline))[0]
    if not 0 < size <= LIMIT:
        raise ValueError('invalid frame length')
    return json.loads(read_exact(fd, size, deadline))


def envelope(identity, request, op, data=None):
    operation = {'op': op}
    if data is not None:
        operation['data'] = data
    return {'major': 1, 'identity': identity, 'request_id': request, 'operation': operation}


def protocol(process, identity):
    state = {'request_id': 0}

    def send(op, data=None):
        state['request_id'] += 1
        process.stdin.write(encode(envelope(identity, state['request_id'], op, data)))
        process.stdin.flush()
        return state['request_id']

    def wait(op, timeout=10):
        deadline = time.monotonic() + timeout
        while True:
            reply = receive(process.stdout.fileno(), max(0, deadline - time.monotonic()))
            if reply.get('major') != 1 or reply.get('identity') != identity:
                raise AssertionError('foreign/stale response identity')
            operation = reply['operation']
            if operation['op'] == 'Error':
                raise AssertionError('companion returned ' + str(operation.get('data', {}).get('code')))
            if operation['op'] == op:
                return operation.get('data') or {}

    return send, wait


def observe_marker(send, wait, payload, marker):
    send('TerminalInput', {'data': base64.b64encode(payload).decode()})
    output = bytearray()
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        data = wait('TerminalOutput')
        output.extend(base64.b64decode(data['data'], validate=True))
        if len(output) > 1024 * 1024:
            raise AssertionError('terminal observation bound exceeded')
        match = re.search(rb'(?:^|[\r\n])' + marker.encode() + rb':([0-9]+)[\r\n]', output)
        if match:
            return int(match.group(1))
    raise TimeoutError('shell PID marker unavailable')


def collect_until_idle(process, identity, idle=0.4, cap=5):
    output = bytearray()
    deadline = time.monotonic() + cap
    fd = process.stdout.fileno()
    while time.monotonic() < deadline:
        remaining = deadline - time.monotonic()
        if not select.select([fd], [], [], min(idle, remaining))[0]:
            if output:
                break
            continue
        try:
            frame = receive(fd, min(idle, max(0.01, deadline - time.monotonic())))
        except TimeoutError:
            if output:
                break
            continue
        if frame.get('major') != 1 or frame.get('identity') != identity:
            raise AssertionError('foreign/stale response identity')
        operation = frame['operation']
        if operation['op'] == 'Error':
            raise AssertionError('companion returned ' + str(operation.get('data', {}).get('code')))
        if operation['op'] == 'TerminalOutput':
            output.extend(base64.b64decode(operation['data']['data'], validate=True))
            if len(output) > 1024 * 1024:
                raise AssertionError('terminal observation bound exceeded')
    return bytes(output)


def capabilities(h, command):
    # Isolate OpenCode and helper state from the actual user's HOME/config.
    home = str(h.root / 'home')
    env = {'HOME': home, 'XDG_CONFIG_HOME': home + '/.config',
           'XDG_DATA_HOME': home + '/.local/share', 'XDG_STATE_HOME': home + '/.local/state',
           'XDG_CACHE_HOME': home + '/.cache', 'TMPDIR': str(h.root / 'tmp')}
    prefix = 'env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin ' + ' '.join(
        f'{key}={shlex.quote(value)}' for key, value in env.items())
    process = h.ssh_process(prefix + ' ' + command)
    identity = {'authority': list(os.urandom(16)), 'tab': list(os.urandom(32)),
                'epoch': 1, 'generation': 1}
    process.stdin.write(encode(envelope(identity, 1, 'Capabilities')))
    process.stdin.flush()
    reply = receive(process.stdout.fileno())
    if (reply.get('major') != 1 or reply.get('identity') != identity or
            reply.get('request_id') != 1 or reply.get('operation', {}).get('op') != 'CapabilitiesResult'):
        raise AssertionError('frontend did not return the matching v1 CapabilitiesResult')
    print('PASS real SSH companion capabilities; survival and native UI NOT tested')


def survival(h, command, workspace):
    """Shell transport checkpoint. Does not certify the desktop input queue/TUI."""
    home = str(h.root / 'home')
    prefix = ('env -i PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin '
              f'HOME={shlex.quote(home)} XDG_DATA_HOME={shlex.quote(home + "/.local/share")} '
              f'XDG_CONFIG_HOME={shlex.quote(home + "/.config")} '
              f'XDG_CACHE_HOME={shlex.quote(home + "/.cache")} '
              f'TMPDIR={shlex.quote(str(h.root / "tmp"))} ')
    identity = {'authority': list(os.urandom(16)), 'tab': list(os.urandom(32)),
                'epoch': 1, 'generation': 1}
    process = h.ssh_process(prefix + command)
    send, wait = protocol(process, identity)
    opened = False
    try:
        send('Open', {'directory': list(os.fsencode(str(workspace))),
                      'target': 'Shell', 'rows': 24, 'cols': 80})
        wait('Attached')
        opened = True
        before_marker = 'BEFORE_' + os.urandom(8).hex()
        before = observe_marker(
            send, wait, f"printf '\\n{before_marker}:%s\\n' \"$$\"\n".encode(), before_marker)
        process.kill()  # exact owned SSH frontend; never terminate the remote PID
        process.wait(timeout=5)
        # Fault duration is the subject of the experiment, not a readiness sleep.
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            select.select([], [], [], max(0, deadline - time.monotonic()))
        identity['epoch'] += 1
        process = h.ssh_process(prefix + command)
        send, wait = protocol(process, identity)
        send('Attach', {'rows': 30, 'cols': 100})
        wait('Attached')
        after_marker = 'AFTER_' + os.urandom(8).hex()
        after = observe_marker(
            send, wait, f"printf '\\n{after_marker}:%s\\n' \"$$\"\n".encode(), after_marker)
        if before != after:
            raise AssertionError('shell was replaced instead of reattached')
        send('Close')
        closed = wait('Closed')
        opened = False
        if not closed.get('confirmed'):
            raise AssertionError('remote cleanup unconfirmed')
        print(f'PASS real shell PID {before} survived 60s owned-SSH loss; '
              'desktop no-replay, OpenCode session and TUI fidelity NOT certified')
    finally:
        if opened and process.poll() is None:
            try:
                send('Close')
                wait('Closed')
            except (OSError, ValueError, EOFError, TimeoutError, AssertionError):
                print('WARNING remote cleanup unconfirmed; supervisor lease remains applicable',
                      file=sys.stderr)


def companion_candidates():
    root = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
    target = os.path.join(root, 'target')
    return [
        os.path.join(target, 'aarch64-unknown-linux-gnu', 'release', 'made-remote'),
        os.path.join(target, 'aarch64-unknown-linux-gnu', 'debug', 'made-remote'),
    ]


def resolve_linux_arm64_companion(explicit):
    paths = [explicit] if explicit else companion_candidates()
    reasons = []
    for path in paths:
        if not path or not os.path.isfile(path):
            reasons.append(f'{path or "(none)"}: absent')
            continue
        reason = linux_arm64_elf(path)
        if reason is None:
            return path, None
        reasons.append(f'{path}: {reason}')
    host = os.path.join(os.path.dirname(companion_candidates()[0]), '..', '..', 'debug', 'made-remote')
    host = os.path.abspath(host)
    if os.path.isfile(host):
        reasons.append(f'{host}: {linux_arm64_elf(host)}')
    status = cross_build_blockage()
    detail = '; '.join(reasons)
    missing = '; '.join(status['missing']) or 'none'
    targets = ', '.join(status['installed_targets']) or 'none'
    lock = 'busy' if cargo_lock_busy() else 'free'
    return None, (
        f'no compatible Linux aarch64 companion ({detail}). '
        f'installed targets: {targets}. missing: {missing}. cargo lock: {lock}. '
        f'{status["refused"]}; rustup target add was not run'
    )


def frontend_command(root, binary):
    env = {
        'HOME': root + '/home',
        'XDG_DATA_HOME': root + '/data',
        'XDG_CONFIG_HOME': root + '/config',
        'XDG_STATE_HOME': root + '/state',
        'XDG_CACHE_HOME': root + '/cache',
        'TMPDIR': root + '/tmp',
    }
    prefix = 'env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin ' + ' '.join(
        f'{key}={shlex.quote(value)}' for key, value in env.items())
    return prefix + ' ' + shlex.quote(binary) + ' --stdio'


def prove_native_supervisor(session, companion):
    """Create a shell, kill SSH, prove the same PID, and reattach without replay."""
    root = None
    shell_pid = None
    process = None
    opened = False
    try:
        root = session.make_private_dir()
        print(f'command: ssh mkdir private {root}')
        remote_bin = root + '/made-remote'
        session.upload(companion, remote_bin)
        print(f'command: ssh upload companion to {remote_bin}')
        command = frontend_command(root, remote_bin)
        identity = {'authority': list(os.urandom(16)), 'tab': list(os.urandom(32)),
                    'epoch': 1, 'generation': 1}
        process = session.ssh_process(command)
        send, wait = protocol(process, identity)
        send('Open', {'directory': list((root + '/workspace').encode()),
                      'target': 'Shell', 'rows': 24, 'cols': 80})
        wait('Attached')
        opened = True
        ready = 'READY_' + os.urandom(8).hex()
        shell_pid = observe_marker(
            send, wait,
            f"stty -echo; printf '\\n{ready}:%s\\n' \"$$\"\n".encode(), ready)
        print(f'pid_created: {shell_pid}')
        session.ssh_run(f'kill -0 {shell_pid}')
        nonce = os.urandom(8).hex()
        sent = 'SENT_' + os.urandom(8).hex()
        payload = (
            f"printf '\\033]999;NOREPLAY_{nonce}\\007\\n{sent}:%s\\n' \"$$\"\n".encode())
        echoed = observe_marker(send, wait, payload, sent)
        if echoed != shell_pid:
            raise AssertionError('shell PID changed before SSH death')
        local_ssh = process.pid
        process.kill()
        process.wait(timeout=5)
        opened = False
        print(f'command: kill local ssh pid {local_ssh}')
        session.ssh_run(f'kill -0 {shell_pid}')
        print(f'pid_after_ssh_death: {shell_pid}')
        identity['epoch'] += 1
        process = session.ssh_process(command)
        send, wait = protocol(process, identity)
        send('Attach', {'rows': 30, 'cols': 100})
        wait('Attached')
        opened = True
        snapshot = collect_until_idle(process, identity)
        if not snapshot:
            raise AssertionError('reattach produced no terminal resnapshot')
        if b'NOREPLAY_' in snapshot:
            raise AssertionError('reattach replayed historical terminal bytes')
        after = 'AFTER_' + os.urandom(8).hex()
        reattached = observe_marker(
            send, wait, f"printf '\\n{after}:%s\\n' \"$$\"\n".encode(), after)
        print(f'pid_after_reattach: {reattached}')
        if reattached != shell_pid:
            raise AssertionError('shell was replaced instead of reattached')
        send('Close')
        closed = wait('Closed')
        opened = False
        if not closed.get('confirmed'):
            raise AssertionError('remote cleanup unconfirmed')
        print(f'PASS same PID {shell_pid} survived SSH death and reattached without replay; '
              '60s lease, modes, mouse, alternate screen, and OpenCode NOT certified')
        return 0
    finally:
        if opened and process is not None and process.poll() is None:
            try:
                send('Close')
                wait('Closed')
            except (OSError, ValueError, EOFError, TimeoutError, AssertionError, NameError):
                print('WARNING protocol close unconfirmed', file=sys.stderr)
        if process is not None and process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        if session is not None:
            try:
                status = session.cleanup_owned(root, shell_pid)
            except (OSError, subprocess.SubprocessError, ValueError) as exc:
                status = f'cleanup-unconfirmed: {exc}'
            print(f'cleanup: {status}')


def tty_available():
    try:
        fd = os.open('/dev/tty', os.O_RDWR)
    except OSError:
        return False
    os.close(fd)
    return True


def container_survival(fingerprint, companion):
    confirm_authorized_host(fingerprint)
    print('command: ssh-keyscan -T 5 -p 2222 127.0.0.1')
    print('PASS authorized ED25519 host key matches; personal known_hosts unchanged; '
          'survival not tested')
    path, blocked = resolve_linux_arm64_companion(companion)
    if blocked:
        print('BLOCKED companion proof unrun: ' + blocked)
        print('pids: none')
        print('cleanup: temporary trust store removed; password not written; '
              'no remote process or file created')
        return 2
    if not tty_available():
        print('BLOCKED companion present but no terminal for a temporary password; '
              'refusing a persisted credential')
        print('pids: none')
        print('cleanup: no password file written; no remote file created')
        return 2
    password = getpass.getpass('Fixture password (not stored): ')
    try:
        if not password:
            print('BLOCKED empty password refused')
            return 2
        scan = subprocess.run(
            ['ssh-keyscan', '-T', '5', '-p', '2222', '127.0.0.1'],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            timeout=10, check=True)
        matches = matching_host_keys(scan.stdout, fingerprint)
        if not matches:
            raise PrerequisiteError('host key does not match the explicitly supplied fingerprint')
        with ContainerSession(password) as session:
            session.write_trust(matches)
            return prove_native_supervisor(session, path)
    finally:
        password = ''


# ═══ Grok Build parity over the isolated loopback harness (tasks 2.42/2.43) ══
#
# Every scenario below drives real companion entry points through the isolated
# loopback SSH harness. First-fail evidence comes from fixture-level mutations
# only (PATH composition, seed contents, install/no-install, kill/no-kill,
# epoch choice); product code is never mutated.

GROK_VERSION = '0.1.1'
GROK_BOUNDARY = ('real loopback SSH plus real companion entry points; desktop tab/label/'
                 'diagnostic presentation is model-level (crates/made-app tests); native '
                 'GUI and the live-SSH matrix are not certified here')


def fresh_identity():
    return {'authority': list(os.urandom(16)), 'tab': list(os.urandom(32)),
            'epoch': 1, 'generation': 1}


def decode_cwd(value):
    """RemotePath travels as a byte array on the wire."""
    if value is None:
        return None
    if isinstance(value, list):
        return bytes(value).decode('utf-8', 'replace')
    return value


def session_ids(snapshot):
    return [session['session_id'] for session in snapshot.get('sessions', [])]


def inventory_row(pid, session, cwd, opened_at='2026-01-01T00:00:00Z'):
    return json.dumps({'pid': pid, 'session_id': session, 'cwd': cwd,
                       'opened_at': opened_at})


def hook_line(session, cwd, event, status, pid=None):
    """One made-grok-status.jsonl line. The shipped grok-made hook records the
    parent pid of the hook process — the Grok session process that ran it —
    so pid-bearing lines are the current plugin shape; pid=None is the legacy
    shape a pre-existing log can still carry."""
    line = {'sessionId': session}
    if pid is not None:
        line['pid'] = int(pid)
    line.update({'cwd': cwd, 'event': event, 'status': status})
    return json.dumps(line)


def manifest_version(text):
    if not text:
        return None
    match = re.search(r'"version"\s*:\s*"([^"]+)"', text.decode('utf-8', 'replace'))
    return match.group(1) if match else None


def marker_text_ish(text, name):
    match = re.search(name + r':(\S+)', text)
    return match.group(1) if match else None


def remote_write(h, path, text):
    h.ssh_run(f"cat > {shlex.quote(str(path))} <<'MADE_FIXTURE_EOF'\n{text}\nMADE_FIXTURE_EOF")


def remote_append(h, path, line):
    h.ssh_run(f"cat >> {shlex.quote(str(path))} <<'MADE_FIXTURE_EOF'\n{line}\nMADE_FIXTURE_EOF")


def remote_read(h, path):
    try:
        return h.ssh_run(f'cat {shlex.quote(str(path))}').stdout
    except subprocess.CalledProcessError:
        return None


def remote_exists(h, path):
    try:
        h.ssh_run(f'test -e {shlex.quote(str(path))}')
        return True
    except subprocess.CalledProcessError:
        return False


def pid_alive(h, pid):
    try:
        h.ssh_run(f'kill -0 {int(pid)}')
        return True
    except subprocess.CalledProcessError:
        return False


def kill_pid(h, pid):
    h.ssh_run(f'kill {int(pid)}')


class GrokFixture:
    """Task 2.42 fixtures around one isolated harness root: PATH variants, a
    seeded Grok home, and staged/installed plugin copies. All state stays
    inside the disposable root; setup installs and downloads nothing."""

    def __init__(self, root, companion):
        self.root = pathlib.Path(root)
        self.scripts = grok_fixture_scripts(self.root)
        self.companion_dir = self.root / 'companion'
        self.companion_dir.mkdir(mode=0o700, exist_ok=True)
        self.binary = self.companion_dir / 'made-remote'
        self.host_companion = pathlib.Path(companion)
        shutil.copyfile(companion, self.binary)
        self.binary.chmod(0o755)
        self.home = grok_home(self.root)
        self.supervisor_status = None
        self.supervisor_reason = None
        self.opencode_staged = None

    def stage_shipped_opencode(self):
        """Stage the shipped opencode-made package beside the companion (the
        staged debug layout the companion resolves). One copy per fixture; the
        package never changes so re-staging after reset_stage is a no-op."""
        if self.opencode_staged is None:
            source = pathlib.Path(__file__).resolve().parents[2] / 'plugins' / 'opencode-made'
            stage_shipped_opencode_made(self.companion_dir, source)
            self.opencode_staged = self.companion_dir / 'plugins' / 'opencode-made'
        return self.opencode_staged

    def native_hook(self):
        """The real native session-status hook for the host target: the one
        staged beside the host companion (the CI artifact), else a local cargo
        build. A missing hook is unrun, never a pass."""
        return native_hook_path(self.host_companion)

    def reset_home(self, inventory=None, status_lines=(), installed=None):
        shutil.rmtree(self.home, ignore_errors=True)
        self.home = grok_home(self.root)
        if inventory is not None:
            seed_grok_inventory(self.home, inventory)
        if status_lines:
            seed_grok_status(self.home, status_lines)
        if installed:
            seed_installed_grok_made(self.home, installed)

    def reset_stage(self, version=None):
        shutil.rmtree(self.companion_dir / 'plugins', ignore_errors=True)
        if version:
            seed_staged_grok_made(self.companion_dir, version)
        # The shipped opencode-made package is part of every staged companion;
        # reset_stage rebuilds the plugins directory, so re-stage it.
        self.opencode_staged = None
        self.stage_shipped_opencode()

    def reset_logs(self):
        self.scripts['argv_log'].write_text('')
        self.scripts['env_log'].write_text('')

    @property
    def sessions_file(self):
        return self.home / 'active_sessions.json'

    @property
    def status_file(self):
        return self.home / 'made-grok-status.jsonl'

    @property
    def config_file(self):
        return self.home / 'config.toml'

    @property
    def installed_manifest(self):
        return self.home / 'installed-plugins' / 'grok-made' / 'plugin.json'

    @property
    def staged_dir(self):
        return self.companion_dir / 'plugins' / 'grok-made'

    def prefix(self, grok=False, python3=False, opencode=True, extra=None):
        """env -i prefix with absolute PATH entries only (the detection rule).
        No interpreter is a product prerequisite, so python3 is absent by
        default: /usr/bin carries the host python3 and Ubuntu's usrmerge makes
        /bin a symlink to /usr/bin, so Linux uses a private coreutils-only
        shadow directory instead. `python3=True` puts a fixture python3 on
        PATH, only for scenarios proving its presence changes nothing. The fake
        opencode is omitted only when a scenario needs opencode to be absent."""
        home = str(self.root / 'home')
        entries = []
        if grok:
            entries.append(str(self.scripts['grokbin']))
        if python3:
            entries.append(str(self.scripts['pybin']))
        if opencode:
            entries.append(str(self.scripts['ocbin']))
        if python3:
            entries += ['/bin', '/usr/sbin', '/sbin', '/usr/bin']
        elif platform.system() == 'Linux':
            entries += [self.coreutils_path(), '/usr/sbin', '/sbin']
        else:
            entries += ['/bin', '/usr/sbin', '/sbin']
        environment = {
            'PATH': ':'.join(entries),
            'HOME': home,
            'XDG_CONFIG_HOME': home + '/.config',
            'XDG_DATA_HOME': home + '/.local/share',
            'XDG_STATE_HOME': home + '/.local/state',
            'XDG_CACHE_HOME': home + '/.cache',
            'TMPDIR': str(self.root / 'tmp'),
        }
        environment.update(extra or {})
        return 'env -i ' + ' '.join(f'{key}={shlex.quote(value)}'
                                    for key, value in environment.items())

    def coreutils_path(self):
        """A PATH entry with the host's coreutils and no python3 executable.

        A merged /bin must be shadowed rather than merely omitted, so no
        python3 resolves on the remote PATH. Symlinks stay inside this
        disposable root and never modify the host."""
        shadow = self.root / 'corebin'
        if shadow.is_dir():
            return str(shadow)
        shadow.mkdir(mode=0o700)
        for directory in ('/bin', '/usr/bin'):
            try:
                entries = sorted(pathlib.Path(directory).iterdir())
            except OSError:
                continue
            for entry in entries:
                if entry.name == 'python3' or entry.name.startswith('python3.'):
                    continue
                target = shadow / entry.name
                if target.exists() or target.is_symlink():
                    continue
                if entry.is_symlink() or entry.is_file():
                    target.symlink_to(entry)
        return str(shadow)

    def frontend_command(self, prefix, mode='--stdio'):
        return prefix + ' ' + shlex.quote(str(self.binary)) + ' ' + mode

    def argv(self):
        return [line for line in self.scripts['argv_log'].read_text().splitlines() if line]

    def env_lines(self):
        return [line for line in self.scripts['env_log'].read_text().splitlines() if line]


class GrokTab:
    """One live companion frontend over the loopback harness: real Open/Attach/
    Close/GrokState/SessionState/TerminalInput entry points, with GrokEvent and
    GrokSnapshot pushes collected instead of discarded. A failed connect is
    kept in `error` so callers can assert on rejections and still clean up."""

    def __init__(self, h, fx, workspace, target='Grok', grok=True, python3=False,
                 opencode=True, extra=None, identity=None, epoch=None, mode='Open',
                 attach_timeout=15):
        self.h = h
        self.fx = fx
        self.workspace = workspace
        self.identity = dict(identity) if identity else fresh_identity()
        if epoch is not None:
            self.identity['epoch'] = epoch
        self.events = []
        self.stream_ops = []
        self.terminal = bytearray()
        self.last_snapshot = None
        self.opened = False
        self.error = None
        self.process = None
        self.send = None
        self.wait = None
        self.stderr_text = ''
        try:
            self._connect(target, grok, python3, opencode, extra, mode, attach_timeout)
        except (AssertionError, TimeoutError, EOFError, OSError) as exc:
            self.error = exc
            self.opened = False
            if self.process is not None and self.process.poll() is None:
                self.process.kill()
                self.process.wait(timeout=5)
            self.stderr_text = self._captured_stderr()

    def _captured_stderr(self):
        """Frontend stderr from the failed connect, available after the process
        has exited and before the harness closes its PIPE."""
        if self.process is None or self.process.stderr is None:
            return ''
        try:
            if self.process.poll() is None:
                return ''
            return self.process.stderr.read().decode('utf-8', 'replace').strip()
        except (OSError, ValueError):
            return ''

    def _connect(self, target, grok, python3, opencode, extra, mode, attach_timeout):
        prefix = self.fx.prefix(grok=grok, python3=python3, opencode=opencode, extra=extra)
        self.process = self.h.ssh_process(self.fx.frontend_command(prefix))
        self.send, self.wait = protocol(self.process, self.identity)
        if mode == 'Open':
            self.send('Open', {'directory': list(os.fsencode(str(self.workspace))),
                               'target': target, 'rows': 24, 'cols': 80})
        else:
            self.send('Attach', {'rows': 24, 'cols': 80})
        self.wait('Attached', timeout=attach_timeout)
        self.opened = True

    def receive_frame(self, timeout):
        return receive(self.process.stdout.fileno(), timeout)

    def absorb(self, operation):
        name = operation.get('op')
        self.stream_ops.append(name)
        if name == 'GrokEvent':
            # Wire shape: GrokEvent data carries the event under `event`
            # (`{"op": "GrokEvent", "data": {"event": {...}}}`).
            self.events.append((operation.get('data') or {}).get('event') or {})
            return True
        if name == 'TerminalOutput':
            data = operation.get('data') or {}
            self.terminal.extend(base64.b64decode(data.get('data', ''), validate=True))
            return data.get('done') is True
        if name == 'Error':
            raise AssertionError('companion returned '
                                 + str((operation.get('data') or {}).get('code')))
        return False

    def collect_terminal(self, idle=0.6, cap=6):
        """Read the post-attach resnapshot until its `done: true` terminator."""
        fd = self.process.stdout.fileno()
        out = bytearray()
        deadline = time.monotonic() + cap
        while time.monotonic() < deadline:
            remaining = deadline - time.monotonic()
            if not select.select([fd], [], [], min(idle, remaining))[0]:
                if out:
                    break
                continue
            try:
                frame = self.receive_frame(min(idle, max(0.01, deadline - time.monotonic())))
            except TimeoutError:
                if out:
                    break
                continue
            operation = frame['operation']
            self.absorb(operation)
            if operation['op'] == 'TerminalOutput':
                out.extend(base64.b64decode(operation['data']['data'], validate=True))
                if len(out) > 1024 * 1024:
                    raise AssertionError('terminal observation bound exceeded')
                if operation['data'].get('done'):
                    break
        self.terminal.extend(out)
        return bytes(out)

    def type(self, payload):
        self.send('TerminalInput', {'data': base64.b64encode(payload.encode()).decode()})

    def wait_terminal(self, pattern, timeout=10):
        deadline = time.monotonic() + timeout
        compiled = re.compile(pattern)
        while time.monotonic() < deadline:
            match = compiled.search(self.terminal.decode('utf-8', 'replace'))
            if match:
                return match
            try:
                frame = self.receive_frame(min(0.25, max(0.01, deadline - time.monotonic())))
            except TimeoutError:
                continue
            self.absorb(frame['operation'])
        raise TimeoutError(f'terminal marker unavailable: {pattern}')

    def wait_pid(self, timeout=10):
        """The fixture grok's own pid marker. The child's first output can sit
        in the resnapshot or arrive as live output after the resnapshot's
        `done` terminator (the supervisor pauses PTY reads while one is
        pending), so the marker itself is the condition — never the resnapshot
        boundary."""
        return int(self.wait_terminal(r'MADE_FIXTURE_GROK:([0-9]+)', timeout=timeout).group(1))

    def state(self, timeout=10):
        """One GrokState round trip; the reply carries the current revision."""
        request = self.send('GrokState')
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError('GrokState deadline exceeded; events so far: '
                                   + repr(self.events[-8:]))
            try:
                frame = self.receive_frame(min(0.25, remaining))
            except TimeoutError:
                continue
            operation = frame['operation']
            if frame.get('request_id') == request and operation.get('op') == 'GrokSnapshot':
                # Wire shape: GrokSnapshot data carries the state under
                # `snapshot` (`{"op": "GrokSnapshot", "data": {"snapshot": {...}}}`),
                # so revision/running/sessions live one level below the frame
                # data, exactly like SessionSnapshot.
                snapshot = (operation.get('data') or {}).get('snapshot') or {}
                self.last_snapshot = snapshot
                return snapshot, list(self.events)
            self.absorb(operation)

    def arm(self):
        """A fresh attachment builds state synchronously on the first GrokState;
        this forces the hook-tier decision before any appended line."""
        return self.state()

    def wait_state(self, predicate, timeout=12):
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError('Grok snapshot condition not met; last: '
                                   + repr(self.last_snapshot))
            snapshot, _ = self.state(timeout=min(2.0, max(0.2, remaining)))
            if predicate(snapshot):
                return snapshot, list(self.events)

    def wait_event(self, predicate, timeout=8):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for event in self.events:
                if predicate(event):
                    return event
            try:
                frame = self.receive_frame(min(0.25, max(0.01, deadline - time.monotonic())))
            except TimeoutError:
                continue
            self.absorb(frame['operation'])
        raise TimeoutError('Grok event condition not met; events: '
                           + repr(self.events[-8:]))

    def wait_stream(self, op, timeout=10):
        """An unsolicited stream frame of the given op (e.g. SessionSnapshot
        pushed when the integration state reaches this tab). Returns once one
        has been seen; the reply to a matching request can then be asserted
        without racing the push."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if op in self.stream_ops:
                return
            try:
                frame = self.receive_frame(min(0.25, max(0.01, deadline - time.monotonic())))
            except TimeoutError:
                continue
            self.absorb(frame['operation'])
        raise TimeoutError(f'{op} never arrived on the event stream; ops so far: '
                           + repr(self.stream_ops[-16:]))

    def expect_no_events(self, window):
        seen = len(self.events)
        deadline = time.monotonic() + window
        while time.monotonic() < deadline:
            try:
                frame = self.receive_frame(min(0.25, max(0.01, deadline - time.monotonic())))
            except TimeoutError:
                continue
            self.absorb(frame['operation'])
        if len(self.events) > seen:
            raise AssertionError(f'unexpected Grok events: {self.events[seen:]}')

    def expect_error(self, op, timeout=8):
        request = self.send(op)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                frame = self.receive_frame(min(0.25, max(0.01, deadline - time.monotonic())))
            except TimeoutError:
                continue
            operation = frame['operation']
            if frame.get('request_id') == request and operation.get('op') == 'Error':
                return (operation.get('data') or {}).get('code')
            if operation.get('op') != 'Error':
                self.absorb(operation)
        raise TimeoutError(f'{op} did not fail')

    def close(self):
        if not self.opened:
            return None
        self.send('Close')
        closed = self.wait('Closed', timeout=10)
        self.opened = False
        return bool(closed.get('confirmed'))

    def kill_transport(self):
        self.process.kill()
        self.process.wait(timeout=5)
        self.opened = False


def close_quietly(tab):
    """Best-effort cleanup: explicit Close, then one reattach+Close recovery.
    The detached lease is 24h, so an unrecovered session is a visible warning,
    never a silent leak."""
    if tab is None:
        return
    try:
        if tab.opened and tab.process.poll() is None:
            try:
                if tab.close():
                    return
            except (AssertionError, TimeoutError, EOFError, OSError, ValueError):
                pass
            try:
                recovery = GrokTab(tab.h, tab.fx, tab.workspace, identity=tab.identity,
                                   epoch=tab.identity['epoch'] + 1, mode='Attach')
                recovery.close()
            except (AssertionError, TimeoutError, EOFError, OSError, ValueError) as exc:
                print(f'WARNING unrecovered remote session (bounded but long lease): {exc}',
                      file=sys.stderr)
    finally:
        if tab.process is not None and tab.process.poll() is None:
            tab.process.kill()
            tab.process.wait(timeout=5)


def run_install(h, fx, python3=False, fail=False):
    """Run the real --install-grok-plugin entry point and return (report, exit)."""
    extra = {'MADE_FIXTURE_INSTALL_FAIL': '1'} if fail else None
    command = fx.frontend_command(fx.prefix(grok=True, python3=python3, extra=extra),
                                  '--install-grok-plugin')
    try:
        result = h.ssh_run(command, timeout=30)
        code = 0
    except subprocess.CalledProcessError as exc:
        result = exc
        code = exc.returncode
    line = (result.stdout or b'').decode().strip()
    if not line:
        raise AssertionError('the install mode produced no report line')
    return json.loads(line), code


class SupervisorPreflightError(RuntimeError):
    """A session cannot be opened at all: the product's supervisor never
    starts on this target. Scenarios that need a session are unrun, not fails."""


def runtime_dir_name(identity):
    """The runtime directory name: Token::from_bytes hexes the tab bytes and
    the fingerprint is sha256 of that hex text, truncated to 16."""
    hexed = bytes(identity['tab']).hex()
    return 'sha256-' + hashlib.sha256(hexed.encode()).hexdigest()[:16]


def supervisor_unavailable(h, fx, workspace):
    """One real Open attempt per harness run, cached. Returns the diagnosed
    reason when the companion supervisor cannot start, else None."""
    if fx.supervisor_status is None:
        tab = GrokTab(h, fx, workspace, target='Grok')
        try:
            if tab.error is None:
                tab.close()
                fx.supervisor_status = 'ok'
                fx.supervisor_reason = None
            else:
                fx.supervisor_status = 'broken'
                fx.supervisor_reason = diagnose_supervisor_failure(
                    h, tab.identity, tab.error, tab.stderr_text)
        finally:
            close_quietly(tab)
    return fx.supervisor_reason


def diagnose_supervisor_failure(h, identity, error, stderr=''):
    """Report the captured evidence for a failed supervisor-backed Open.

    The same Open failure can come from a supervisor crash, an unavailable
    service, a resource limit, or a transport fault. The report therefore leads
    with the observed supervisor/frontend evidence and adds known-cause
    guidance only when that evidence matches it."""
    name = runtime_dir_name(identity)
    data_root = host_data_root(h)
    shown = ('$HOME/Library/Application Support/Made' if platform.system() == 'Darwin'
             else '$HOME/.local/share/made')
    runtime = f'{shown}/remote/runtime/{name}'
    try:
        listing = h.ssh_run(
            'ls -a ' + shlex.quote(str(data_root / 'remote' / 'runtime' / name))
            + ' 2>/dev/null || true').stdout
    except (subprocess.SubprocessError, OSError):
        listing = b''
    seen = [line for line in listing.decode(errors='replace').splitlines()
            if line not in ('', '.', '..')]
    endpoint = ('endpoint published; the failure came after startup'
                if 'endpoint.json' in seen else
                'endpoint.json absent, so the supervisor did not publish an endpoint')
    frontend = stderr.strip() or '<no stderr captured>'
    text = (
        'supervisor preflight failed: the companion frontend did not reach '
        'Attached, so session-dependent Grok scenarios are unrun. '
        f'Captured frontend stderr: {frontend}. '
        f'Frontend exception: {type(error).__name__}: {error}. '
        f'Runtime directory: {runtime}; entries: {seen or "none"}; {endpoint}. '
        'CwdReportSink binds inside that runtime directory while the endpoint '
        'fits the 103-byte portable AF_UNIX limit and otherwise creates an '
        'exclusive 0700 directory under the short OS temp root (TMPDIR, then '
        '/tmp), so a long HOME or data path alone does not prevent startup.')
    observed = f'{frontend} {type(error).__name__}: {error}'.lower()
    if ('file name too long' in observed or 'name too long' in observed
            or 'enametoolong' in observed or 'cwd sink' in observed):
        text += (
            ' Known cause matched: the captured error is an AF_UNIX path-length '
            'or cwd-sink rejection. Check the sink path named in that error and '
            'that the short temp-root fallback can be created owner-only and '
            'writable; the runtime directory itself does not have to fit the '
            'limit.')
    return text


def grok_detection_probe(h, fx, workspace, mutation=False):
    """PATH variants decide grok detection; detection never spawns and never
    looks up python3, even when a python3 sits on the remote PATH. Mutation:
    grok is removed from every variant."""
    cases = [(True, False), (True, True), (False, False), (False, True)]
    for grok, python3_on_path in cases:
        prefix = fx.prefix(grok=(False if mutation else grok), python3=python3_on_path)
        process = h.ssh_process(fx.frontend_command(prefix, '--probe'))
        try:
            identity = fresh_identity()
            process.stdin.write(encode(envelope(identity, 1, 'Capabilities')))
            process.stdin.flush()
            reply = receive(process.stdout.fileno())
            operation = reply['operation']
            if operation['op'] == 'Error':
                raise AssertionError('probe failed with '
                                     + str((operation.get('data') or {}).get('code')))
            capabilities = operation['data']['capabilities']
            if capabilities['grok'] is not grok:
                raise AssertionError(
                    f'PATH grok={grok} detected grok={capabilities["grok"]}')
            if capabilities['python3'] is not False:
                raise AssertionError(
                    f'python3 on PATH={python3_on_path} changed the frozen python3 field: '
                    f'{capabilities["python3"]}')
            if capabilities['tmux'] is not False:
                raise AssertionError('Capabilities.tmux must stay false')
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)
    if fx.argv():
        raise AssertionError(f'detection spawned an executable: {fx.argv()}')
    return ('real --probe entry point over loopback SSH: grok follows the PATH variants '
            'exactly, python3 present or absent never changes a capability (the frozen '
            'field stays false), tmux stays false, and no executable is spawned; '
            + GROK_BOUNDARY)


def grok_direct_launch(h, fx, workspace, mutation=False):
    """Direct Grok SSH launch: the detected grok with the single argument
    dashboard, sessionless running indication, no plugin environment, and no
    OpenCode integration on the tab. Mutation: grok absent from PATH."""
    require_supervisor(fx, h, workspace)
    tab = GrokTab(h, fx, workspace, target='Grok', grok=not mutation)
    try:
        tab.collect_terminal()
        pid = tab.wait_pid()
        if marker_text_ish(tab.terminal.decode('utf-8', 'replace'),
                           'MADE_FIXTURE_SESSION') is not None:
            raise AssertionError('a sessionless launch invented an inventory row')
        if fx.argv() != ['dashboard']:
            raise AssertionError(f'direct launch argv is not exactly dashboard: {fx.argv()}')
        if fx.env_lines() != ['GROK_HOME=']:
            raise AssertionError(f'the launch child saw a redirected GROK_HOME: {fx.env_lines()}')
        snapshot, _ = tab.state()
        if not snapshot['running'] or session_ids(snapshot):
            raise AssertionError(f'sessionless running indication wrong: {snapshot}')
        if tab.expect_error('SessionState') != 'Unavailable':
            raise AssertionError('a Grok tab must not carry OpenCode integration state')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        if pid_alive(h, pid):
            raise AssertionError('the directly launched grok survived Close')
        return (f'Open/Grok launched the detected grok with argv [dashboard] (pid {pid}), '
                f'sessionless running indication, empty GROK_HOME in the child env, clean close; '
                f'Grok Build branding is desktop model-level. {GROK_BOUNDARY}')
    finally:
        close_quietly(tab)


def grok_sessionless_branding(h, fx, workspace, mutation=False):
    """running=true with zero sessions, foreign inventory rows ignored and no
    hook events without the consented install. Mutation: the fixture grok
    registers its own session row."""
    extra = {'MADE_FIXTURE_SESSION': '1', 'MADE_FIXTURE_SESSION_ID': 'owned-early'} \
        if mutation else None
    fx.reset_home(inventory='[' + inventory_row(os.getpid(), 'foreign-row', '/foreign') + ']',
                  status_lines=[hook_line('ghost', '/ghost', 'UserPromptSubmit', 'working')])
    fx.reset_stage(GROK_VERSION)
    require_supervisor(fx, h, workspace)
    tab = GrokTab(h, fx, workspace, target='Grok', extra=extra)
    try:
        tab.collect_terminal()
        snapshot, _ = tab.state()
        if not snapshot['running']:
            raise AssertionError('a living grok child must report running before any session')
        if mutation:
            # The fixture registers its own row as its first action; the first
            # snapshot can predate it, and the inventory refreshes on the
            # one-second cadence, so wait for the owned row to become visible.
            snapshot, _ = tab.wait_state(lambda s: 'owned-early' in session_ids(s),
                                         timeout=10)
        if session_ids(snapshot):
            if mutation:
                raise AssertionError('the sessionless tab listed the row the fixture '
                                     f'registered: {session_ids(snapshot)}')
            raise AssertionError(f'foreign rows leaked into the snapshot: {session_ids(snapshot)}')
        if tab.events:
            raise AssertionError(f'uninstalled hooks produced events: {tab.events}')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        return ('GrokState reports running with zero sessions; foreign inventory rows and '
                'pre-existing hook lines change nothing; ' + GROK_BOUNDARY)
    finally:
        close_quietly(tab)


def grok_manual_launch(h, fx, workspace, mutation=False):
    """Typing grok in the remote shell stays untouched; the companion follows
    the session the typed process registers; the tier stays inventory-only.
    Mutation: grok absent from PATH."""
    fx.reset_home(status_lines=[hook_line('history', '/old', 'UserPromptSubmit', 'working')])
    fx.reset_stage(GROK_VERSION)
    extra = {'MADE_FIXTURE_SESSION': '1', 'MADE_FIXTURE_SESSION_ID': 'manual-1',
             'MADE_FIXTURE_CWD': '/manual'}
    require_supervisor(fx, h, workspace)
    tab = GrokTab(h, fx, workspace, target='Shell', grok=not mutation, extra=extra)
    try:
        snapshot, _ = tab.state()
        if snapshot['running']:
            raise AssertionError('a shell tab must never claim a running grok child')
        tab.type("grok dashboard\n")
        tab.wait_terminal(r'MADE_FIXTURE_SESSION:\S+')
        manual_pid = tab.wait_pid()
        if fx.argv() != ['dashboard']:
            raise AssertionError(f'typed grok argv was altered or extra: {fx.argv()}')
        snapshot, _ = tab.wait_state(lambda s: 'manual-1' in session_ids(s), timeout=12)
        if snapshot['running']:
            raise AssertionError('a manually launched grok must not brand the tab running')
        session = next(s for s in snapshot['sessions'] if s['session_id'] == 'manual-1')
        if decode_cwd(session['cwd']) != '/manual' or session['activity'] != 'Idle':
            raise AssertionError(f'inventory-tier session is wrong: {session}')
        remote_append(h, fx.status_file,
                      hook_line('manual-1', '/manual', 'UserPromptSubmit', 'working',
                                pid=manual_pid))
        tab.expect_no_events(2.0)
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        if manual_pid is None or pid_alive(h, manual_pid):
            raise AssertionError('Close left the manually launched grok alive')
        return ('typed `grok dashboard` in the real remote shell: argv untouched, the session '
                f'follows with an idle dot, running stays false, no hook events without '
                f'consent, close reaps pid {manual_pid}; ' + GROK_BOUNDARY)
    finally:
        close_quietly(tab)


def grok_inventory_follow(h, fx, workspace, mutation=False):
    """Inventory tier without consent: owned rows appear once a second, foreign
    rows never count, duplicate numeric ids collapse deterministically, a
    malformed file invents nothing, and process death removes the session.
    Mutation: the owned row lists a foreign pid."""
    fx.reset_stage(GROK_VERSION)
    foreign = os.getpid()
    require_supervisor(fx, h, workspace)
    tab = GrokTab(h, fx, workspace, target='Shell')
    try:
        # The owned process must report its own pid. The pid is captured inside
        # the backgrounded `sh -c` (single-quoted, so interactive history
        # expansion on any remote shell cannot mangle it) and that process
        # execs sleep in place, keeping the reported pid meaningful.
        tab.type("sh -c 'printf \"\\nOWNED_PID:%s\\n\" \"$$\"; exec sleep 300' &\n")
        owned = int(tab.wait_terminal(r'OWNED_PID:([0-9]+)').group(1))
        owned_row_pid = foreign if mutation else owned
        rows = '[' + ',\n'.join([
            inventory_row(owned_row_pid, 'owned-1', '/one', '2026-01-01T00:00:02Z'),
            inventory_row(foreign, 'foreign-1', '/foreign', '2026-01-01T00:00:01Z'),
            inventory_row(owned, '42', '/first', '2026-01-01T00:00:03Z'),
            inventory_row(foreign, '42', '/second', '2026-01-01T00:00:04Z'),
        ]) + ']'
        remote_write(h, fx.sessions_file, rows)
        snapshot, _ = tab.wait_state(lambda s: 'owned-1' in session_ids(s), timeout=10)
        ids = session_ids(snapshot)
        if ids.count('42') != 1:
            raise AssertionError(f'the numeric-id collision did not collapse once: {ids}')
        collision = next(s for s in snapshot['sessions'] if s['session_id'] == '42')
        if decode_cwd(collision['cwd']) != '/first':
            raise AssertionError(f'the duplicate id kept the wrong row: {collision}')
        if 'foreign-1' in ids:
            raise AssertionError('a foreign pid row leaked into the snapshot')
        if any(s['activity'] != 'Idle' for s in snapshot['sessions']):
            raise AssertionError('the inventory tier must show idle dots only')
        remote_append(h, fx.status_file,
                      hook_line('owned-1', '/one', 'UserPromptSubmit', 'working', pid=owned))
        tab.expect_no_events(2.0)
        remote_write(h, fx.sessions_file, '[not json at all')
        snapshot, _ = tab.wait_state(lambda s: not session_ids(s), timeout=10)
        tab.type("printf '\\nALIVE_%s_OK\\n' \"$$\"\n")
        tab.wait_terminal(r'ALIVE_[0-9]+_OK')
        remote_write(h, fx.sessions_file,
                     '[' + inventory_row(owned_row_pid, 'owned-1', '/one', 't') + ']')
        if not mutation:
            tab.wait_state(lambda s: 'owned-1' in session_ids(s), timeout=10)
        kill_pid(h, owned)
        # The killed background job stays a zombie, which still counts as live
        # ancestry, until the interactive shell reaps it at its next prompt.
        tab.type("true\n")
        snapshot, _ = tab.wait_state(lambda s: 'owned-1' not in session_ids(s), timeout=10)
        if 'foreign-1' in session_ids(snapshot):
            raise AssertionError('the foreign row appeared after a refresh')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        return (f'owned session (pid {owned}) follows on the one-second cadence, foreign and '
                'malformed rows change nothing, duplicate numeric ids collapse to the first '
                'row, no events without consent, the terminal stays usable, process death '
                'removes the session; ' + GROK_BOUNDARY)
    finally:
        close_quietly(tab)


def grok_open_close_lifecycle(h, fx, workspace, mutation=False):
    """A session appears when first owned, disappears when its process is gone,
    running flips honestly, a reattached client gets a fresh revision, and
    Close is confirmed. Mutation: the child is never killed."""
    fx.reset_stage(GROK_VERSION)
    extra = {'MADE_FIXTURE_SESSION': '1', 'MADE_FIXTURE_SESSION_ID': 'life-1',
             'MADE_FIXTURE_CWD': '/life'}
    require_supervisor(fx, h, workspace)
    tab = GrokTab(h, fx, workspace, target='Grok', extra=extra)
    second = None
    try:
        tab.collect_terminal()
        pid = tab.wait_pid()
        snapshot, _ = tab.wait_state(lambda s: 'life-1' in session_ids(s), timeout=10)
        if not snapshot['running']:
            raise AssertionError('the grok child must report running while alive')
        if not mutation:
            kill_pid(h, pid)
        snapshot, _ = tab.wait_state(lambda s: not session_ids(s) and not s['running'],
                                     timeout=10)
        second = GrokTab(h, fx, workspace, identity=tab.identity, epoch=2, mode='Attach')
        if second.error is not None:
            raise AssertionError(f'reattach after child exit failed: {second.error}')
        tab.opened = False
        snapshot, _ = second.state()
        if snapshot['revision'] != 1:
            raise AssertionError(f'a fresh attachment must restart the revision: '
                                 f'{snapshot["revision"]}')
        if snapshot['running'] or session_ids(snapshot):
            raise AssertionError('a dead child resurrected sessions or running state')
        if not second.close():
            raise AssertionError('Close unconfirmed after the child exited')
        second.opened = False
        return (f'session appeared (pid {pid}), disappeared with the process, running flipped, '
                f'reattach rebuilt revision 1, Close confirmed; ' + GROK_BOUNDARY)
    finally:
        close_quietly(tab)
        close_quietly(second)


def grok_consent_refused(h, fx, workspace, mutation=False):
    """Without the consented install the inventory tier keeps working and no
    installer ever runs. Mutation: perform the write the refusal prevents."""
    fx.reset_home(status_lines=[hook_line('old', '/old', 'SessionStart', 'idle')])
    fx.reset_stage(GROK_VERSION)
    if mutation:
        run_install(h, fx)
    config_before = remote_read(h, fx.config_file)
    extra = {'MADE_FIXTURE_SESSION': '1', 'MADE_FIXTURE_SESSION_ID': 'ref-1',
             'MADE_FIXTURE_CWD': '/ref'}
    require_supervisor(fx, h, workspace)
    tab = GrokTab(h, fx, workspace, target='Grok', extra=extra)
    try:
        tab.collect_terminal()
        pid = tab.wait_pid()
        snapshot, _ = tab.wait_state(lambda s: 'ref-1' in session_ids(s), timeout=10)
        if not snapshot['running']:
            raise AssertionError('the inventory tier must keep the running indication')
        remote_append(h, fx.status_file,
                      hook_line('ref-1', '/ref', 'UserPromptSubmit', 'working', pid=pid))
        tab.expect_no_events(2.0)
        if fx.argv() != ['dashboard']:
            raise AssertionError(f'an unexpected child ran without consent: {fx.argv()}')
        if remote_exists(h, fx.installed_manifest):
            raise AssertionError('a consent-refused run installed grok-made')
        if remote_read(h, fx.config_file) != config_before:
            raise AssertionError('Grok configuration changed without consent')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        return ('no install mode ran (the only child argv is the direct launch itself), '
                'no installed copy, config.toml byte-identical, and the inventory tier '
                'kept following and the running indication; desktop refusal cards are '
                'model-level. ' + GROK_BOUNDARY)
    finally:
        close_quietly(tab)


def grok_install_without_python3(h, fx, workspace, mutation=False):
    """A server with no python3 on its PATH gets the same consented install:
    the official installer runs once with the staged native hook, config is
    untouched, and the result is no different from a server with python3.
    Mutation: the official installer fails."""
    fx.reset_stage(GROK_VERSION)
    report, code = run_install(h, fx, fail=bool(mutation))
    if report.get('outcome') != 'installed' or code != 0:
        raise AssertionError(f'an install without python3 must succeed: {report} exit={code}')
    if report.get('new_sessions_only') is not True:
        raise AssertionError('the new-sessions-only limit must be disclosed')
    if 'reason' in report:
        raise AssertionError(f'a successful install carries no failure reason: {report}')
    if fx.argv() != ['plugin install ' + str(fx.staged_dir) + ' --trust']:
        raise AssertionError(f'official installer argv mismatch: {fx.argv()}')
    if fx.env_lines() != ['GROK_HOME=']:
        raise AssertionError('the installer child must not see a redirected GROK_HOME')
    if manifest_version(remote_read(h, fx.installed_manifest)) != GROK_VERSION:
        raise AssertionError('the consented install did not leave the staged copy installed')
    if remote_read(h, fx.config_file) != GROK_CONFIG_SENTINEL.encode():
        raise AssertionError('the install edited Grok configuration')
    # Presence of python3 changes nothing: an equal version still skips.
    rerun, _ = run_install(h, fx, python3=True)
    if rerun.get('outcome') != 'skipped' or len(fx.argv()) != 1:
        raise AssertionError(f'python3 presence changed the install outcome: {rerun}')
    return (f'report {report} with exit 0 and no python3 on the remote PATH: the official '
            'installer ran once on the staged package, config.toml is byte-identical, the '
            'child GROK_HOME stayed empty, and python3 presence changes nothing; '
            + GROK_BOUNDARY)


def grok_native_hook_without_python3(h, fx, workspace, mutation=False):
    """The installed native hook runs on a PATH without python3 and its real
    line drives the tab: a Prompt event with the typed cwd, owned through the
    hook's parent pid. Mutation: the consented install never runs, so hooks
    are never ready and the real line is ignored."""
    hook_source = fx.native_hook()
    fx.reset_stage(GROK_VERSION)
    staged_hook = fx.staged_dir / 'hooks' / 'bin' / 'made-grok-status'
    shutil.copyfile(hook_source, staged_hook)
    staged_hook.chmod(0o755)
    if not mutation:
        report, code = run_install(h, fx)
        if report.get('outcome') != 'installed' or code != 0:
            raise AssertionError(f'the install without python3 must succeed: {report} exit={code}')
    installed_hook = (fx.home / 'installed-plugins' / 'grok-made' / 'hooks' / 'bin'
                      / 'made-grok-status')
    if not mutation and not remote_exists(h, installed_hook):
        raise AssertionError('the installed copy carries no native hook')
    require_supervisor(fx, h, workspace)
    tab = GrokTab(h, fx, workspace, target='Shell')
    try:
        tab.collect_terminal()
        tab.type("printf '\\nSHELL_PID:%s\\n' \"$$\"\n")
        shell_pid = int(tab.wait_terminal(r'SHELL_PID:([0-9]+)').group(1))
        tab.arm()
        event = ('{"hookEventName":"UserPromptSubmit","sessionId":"native-1",'
                 '"cwd":"/native"}')
        # An interactive shell rebuilds PATH from its startup files, so the hook
        # runs under env -i with an empty PATH: nothing but the executable.
        tab.type(f"printf '%s' '{event}' | env -i PATH=/nonexistent "
                 f"HOME=\"$HOME\" {shlex.quote(str(installed_hook))}; "
                 "printf '\\nHOOK_EXIT:%s\\n' \"$?\"\n")
        tab.wait_terminal(r'HOOK_EXIT:0')
        line = None
        for candidate in (remote_read(h, fx.status_file) or b'').decode().splitlines():
            line = json.loads(candidate)
        if line is None:
            raise AssertionError('the installed native hook appended no line')
        expected = {'sessionId': 'native-1', 'cwd': '/native', 'event': 'UserPromptSubmit',
                    'notification': '', 'status': 'working', 'pid': shell_pid}
        if {key: line.get(key) for key in expected} != expected or set(line) != {
                'at', *expected}:
            raise AssertionError(f'the native hook line is wrong: {line}')
        got = tab.wait_event(lambda e: e['kind'] == 'Prompt', timeout=8)
        if got['activity'] != 'Working' or decode_cwd(got['cwd']) != '/native':
            raise AssertionError(f'the native hook line produced the wrong event: {got}')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        return (f'the staged native hook, installed by the consented install on a PATH with no '
                f'python or python3, appended its real line (pid {shell_pid} = the owning '
                'process) and the companion ingested it as a Working prompt with the typed cwd; '
                + GROK_BOUNDARY)
    finally:
        close_quietly(tab)


def grok_install_failed(h, fx, workspace, mutation=False):
    """A failing official installer reports install-failed, writes nothing, and
    the inventory tier keeps working. Mutation: the fixture installer succeeds."""
    fx.reset_stage(GROK_VERSION)
    report, code = run_install(h, fx, fail=not mutation)
    if report.get('outcome') != 'failed' or report.get('reason') != 'install-failed':
        raise AssertionError(f'a failing installer must report install-failed: {report}')
    if code == 0:
        raise AssertionError('a failed install must exit non-zero')
    if remote_exists(h, fx.installed_manifest):
        raise AssertionError('a failed install left an installed copy')
    if remote_read(h, fx.config_file) != GROK_CONFIG_SENTINEL.encode():
        raise AssertionError('the failed install edited Grok configuration')
    tier_note = supervisor_unavailable(h, fx, workspace)
    if tier_note is None:
        extra = {'MADE_FIXTURE_SESSION': '1', 'MADE_FIXTURE_SESSION_ID': 'fail-1',
                 'MADE_FIXTURE_CWD': '/fail'}
        tab = GrokTab(h, fx, workspace, target='Grok', extra=extra)
        try:
            tab.collect_terminal()
            snapshot, _ = tab.wait_state(lambda s: 'fail-1' in session_ids(s), timeout=10)
            if not snapshot['running']:
                raise AssertionError('the inventory tier must keep working after the failure')
            if not tab.close():
                raise AssertionError('remote cleanup unconfirmed')
            tier_note = 'Grok stayed usable on the inventory tier'
        finally:
            close_quietly(tab)
    else:
        tier_note = ('UNVERIFIED post-failure inventory tier — '
                     + tier_note.splitlines()[0])
    return (f'report {report} with non-zero exit, no installed copy, config untouched; '
            f'{tier_note}; ' + GROK_BOUNDARY)


def grok_consented_install_hooks(h, fx, workspace, mutation=False):
    """The consented install runs the official installer once per version,
    never edits config, then arms hooks: history at arm time is skipped, fresh
    appends apply, SessionEnd removes the dot, and roots are never moved
    server-side. Mutation: the consented install never runs."""
    fx.reset_stage(GROK_VERSION)
    if not mutation:
        report, code = run_install(h, fx)
        if report.get('outcome') != 'installed' or code != 0:
            raise AssertionError(f'the first install must run: {report} exit={code}')
        if report.get('new_sessions_only') is not True:
            raise AssertionError('the new-sessions-only limit must be disclosed')
        expected_argv = ['plugin install ' + str(fx.staged_dir) + ' --trust']
        if fx.argv() != expected_argv:
            raise AssertionError(f'official installer argv mismatch: {fx.argv()}')
        if fx.env_lines() != ['GROK_HOME=']:
            raise AssertionError('the installer child must not see a redirected GROK_HOME')
        version = manifest_version(remote_read(h, fx.installed_manifest))
        if version != GROK_VERSION:
            raise AssertionError(f'the installed copy version is {version}')
        if remote_read(h, fx.config_file) != GROK_CONFIG_SENTINEL.encode():
            raise AssertionError('the consented install edited Grok configuration')
        report, _ = run_install(h, fx)
        if report.get('outcome') != 'skipped' or len(fx.argv()) != 1:
            raise AssertionError(f'an equal version must skip: {report} argv={fx.argv()}')
        remote_write(h, fx.installed_manifest,
                     '{"name":"grok-made","version":"9.9.9"}\n')
        report, _ = run_install(h, fx)
        if report.get('outcome') != 'skipped':
            raise AssertionError(f'a newer installed copy must skip: {report}')
        if manifest_version(remote_read(h, fx.installed_manifest)) != '9.9.9':
            raise AssertionError('the newer installed copy was overwritten')
        remote_write(h, fx.installed_manifest,
                     '{"name":"grok-made","version":"0.0.1"}\n')
        report, _ = run_install(h, fx)
        if report.get('outcome') != 'installed' or len(fx.argv()) != 2:
            raise AssertionError(f'an older version must run exactly once more: '
                                 f'{report} argv={fx.argv()}')
    require_supervisor(fx, h, workspace)
    remote_append(h, fx.status_file, hook_line('history', '/old', 'UserPromptSubmit', 'working'))
    extra = {'MADE_FIXTURE_SESSION': '1', 'MADE_FIXTURE_SESSION_ID': 'hook-1',
             'MADE_FIXTURE_CWD': '/hook'}
    tab = GrokTab(h, fx, workspace, target='Grok', extra=extra)
    try:
        tab.collect_terminal()
        pid = tab.wait_pid()
        tab.arm()
        if tab.events:
            raise AssertionError(f'history predating the attachment replayed: {tab.events}')
        remote_append(h, fx.status_file,
                      hook_line('hook-1', '/hook', 'SessionStart', 'idle', pid=pid))
        event = tab.wait_event(lambda e: e['kind'] == 'SessionStart', timeout=8)
        if event['activity'] != 'Idle':
            raise AssertionError(f'session start must show idle: {event}')
        remote_append(h, fx.status_file,
                      hook_line('hook-1', '/hook-next', 'UserPromptSubmit', 'working', pid=pid))
        event = tab.wait_event(lambda e: e['kind'] == 'Prompt', timeout=8)
        if event['activity'] != 'Working' or decode_cwd(event['cwd']) != '/hook-next':
            raise AssertionError(f'the prompt event is wrong: {event}')
        remote_append(h, fx.status_file,
                      hook_line('hook-1', '/hook-next', 'Notification', 'attention', pid=pid))
        event = tab.wait_event(lambda e: e['activity'] == 'Attention', timeout=8)
        remote_append(h, fx.status_file,
                      hook_line('hook-1', '/hook-next', 'SessionEnd', 'idle', pid=pid))
        tab.wait_event(lambda e: e['kind'] == 'SessionEnd', timeout=8)
        tab.wait_state(lambda s: 'hook-1' not in session_ids(s), timeout=10)
        for forbidden in ('RootResult', 'ShellCwd', 'RootSelect'):
            if forbidden in tab.stream_ops:
                raise AssertionError(f'Grok ingestion moved the root server-side: {forbidden}')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        if pid_alive(h, pid):
            raise AssertionError('Close left the launched grok alive')
        return (f'install ran exactly once per version (equal/newer skip, older reruns once), '
                f'config.toml byte-identical, child GROK_HOME empty, then hooks armed: history '
                f'skipped, fresh appends produced SessionStart/Prompt/attention/SessionEnd '
                f'without moving any root (pid {pid}); ' + GROK_BOUNDARY)
    finally:
        close_quietly(tab)


def grok_opencode_precedence(h, fx, workspace, mutation=False):
    """Protocol-level OpenCode precedence: an OpenCode-target tab owns the
    integration state while Grok state stays subordinate (running=false), and
    server-global hook lines for foreign sessions never reach this tab.
    Mutation: no opencode on the remote PATH."""
    fx.reset_stage(GROK_VERSION)
    seed_installed_grok_made(fx.home, GROK_VERSION)
    remote_write(h, fx.sessions_file,
                 '[' + inventory_row(os.getpid(), 'foreign-1', '/foreign', 't') + ']')
    require_supervisor(fx, h, workspace)
    tab = GrokTab(h, fx, workspace, target='OpenCode', grok=not mutation,
                  opencode=not mutation)
    try:
        if tab.error is not None:
            raise AssertionError(f'the OpenCode tab never attached: {tab.error}')
        tab.collect_terminal()
        # The integration state reaches the tab as an unsolicited SessionSnapshot
        # push once the composed plugin peer connects to the bridge. Waiting on
        # that push first makes the SessionState reply below deterministic.
        tab.wait_stream('SessionSnapshot')
        reply = tab.send('SessionState')
        deadline = time.monotonic() + 8
        snapshot_seen = False
        while time.monotonic() < deadline and not snapshot_seen:
            try:
                frame = tab.receive_frame(min(0.25, max(0.01, deadline - time.monotonic())))
            except TimeoutError:
                continue
            operation = frame['operation']
            if frame.get('request_id') == reply and operation['op'] == 'SessionSnapshot':
                snapshot_seen = True
            elif operation['op'] != 'Error':
                tab.absorb(operation)
        if not snapshot_seen:
            raise AssertionError('an OpenCode tab must own the integration session state')
        snapshot, _ = tab.state()
        if snapshot['running']:
            raise AssertionError('a non-grok child must not report a running grok')
        remote_append(h, fx.status_file,
                      hook_line('foreign-1', '/foreign', 'UserPromptSubmit', 'working',
                                pid=os.getpid()))
        tab.expect_no_events(2.0)
        for forbidden in ('RootResult', 'ShellCwd'):
            if forbidden in tab.stream_ops:
                raise AssertionError(f'Grok ingestion pushed {forbidden} on a foreign session')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        return ('SessionState works on the OpenCode tab while GrokState stays subordinate '
                '(running=false), foreign-session hook lines never leak into this tab, and no '
                'root is moved server-side; label/icon precedence is desktop model-level. '
                + GROK_BOUNDARY)
    finally:
        close_quietly(tab)


def grok_repeated_switches(h, fx, workspace, mutation=False):
    """Repeated attachments replace the previous one, the replaced frontend is
    dropped, each attachment starts a fresh revision, and A→B→C prompt events
    arrive once each on the current attachment without moving any root.
    Mutation: the second attachment reuses the stale epoch."""
    fx.reset_stage(GROK_VERSION)
    seed_installed_grok_made(fx.home, GROK_VERSION)
    extra = {'MADE_FIXTURE_SESSION': '1', 'MADE_FIXTURE_SESSION_ID': 'switch-1',
             'MADE_FIXTURE_CWD': '/a'}
    require_supervisor(fx, h, workspace)
    tab = GrokTab(h, fx, workspace, target='Grok', extra=extra)
    second = None
    try:
        tab.collect_terminal()
        pid = tab.wait_pid()
        tab.arm()
        remote_write(h, fx.sessions_file,
                     '[' + inventory_row(pid, 'switch-1', '/a', 't') + ']')
        tab.wait_state(lambda s: 'switch-1' in session_ids(s), timeout=10)
        second_epoch = tab.identity['epoch'] if mutation else tab.identity['epoch'] + 1
        second = GrokTab(h, fx, workspace, identity=tab.identity, epoch=second_epoch,
                         mode='Attach')
        if second.error is not None:
            raise AssertionError(f'the replacement attachment failed: {second.error}')
        # The design drops the replaced attachment's RPC transport (a new attach
        # "drops only the previous frontend"). The old frontend notices on its
        # writes: the first write after the drop can still land in the
        # half-closed socket buffer, so the harness types twice — the second
        # write hits the reset transport and the frontend exits. The GrokState
        # round trip between the two inputs is a bounded condition wait that
        # guarantees the old frontend already consumed (and wrote) the first
        # line; there are no clock waits here.
        try:
            tab.type("printf 'REPLACED_FRONTEND_INPUT_A\\n'\n")
        except (OSError, ValueError):
            pass  # The replaced frontend's transport already died.
        second.state()
        try:
            tab.type("printf 'REPLACED_FRONTEND_INPUT_B\\n'\n")
        except (OSError, ValueError):
            pass
        try:
            tab.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            raise AssertionError('the replaced attachment was not dropped by the supervisor')
        tab.opened = False
        snapshot, _ = second.state()
        if snapshot['revision'] != 1:
            raise AssertionError(f'a fresh attachment must restart the revision: '
                                 f'{snapshot["revision"]}')
        if session_ids(snapshot) != ['switch-1']:
            raise AssertionError(f'the fresh attachment lost the session: {snapshot}')
        seen = []
        for cwd in ('/a', '/b', '/c'):
            remote_append(h, fx.status_file,
                          hook_line('switch-1', cwd, 'UserPromptSubmit', 'working', pid=pid))
            event = second.wait_event(
                lambda e: e['kind'] == 'Prompt' and decode_cwd(e['cwd']) == cwd, timeout=8)
            seen.append(decode_cwd(event['cwd']))
        if seen != ['/a', '/b', '/c']:
            raise AssertionError(f'prompt switches did not arrive in order: {seen}')
        prompts = [e for e in second.events if e['kind'] == 'Prompt']
        if len(prompts) != 3:
            raise AssertionError(f'a repeated event double-applied: {prompts}')
        for forbidden in ('RootResult', 'ShellCwd', 'RootSelect'):
            if forbidden in second.stream_ops:
                raise AssertionError(f'Grok ingestion moved the root server-side: {forbidden}')
        if not second.close():
            raise AssertionError('remote cleanup unconfirmed')
        second.opened = False
        if pid_alive(h, pid):
            raise AssertionError('Close left the launched grok alive')
        return (f'second attachment (epoch {second_epoch}) replaced the first and dropped its '
                f'frontend, revision restarted at 1, A→B→C prompt events arrived once each '
                f'with their cwds and no root change (pid {pid}); client-side root arbitration '
                f'is model-level. {GROK_BOUNDARY}')
    finally:
        close_quietly(tab)
        close_quietly(second)


def grok_two_tab_isolation(h, fx, workspace, mutation=False):
    """Two Grok tabs on one server each follow only their own ancestry from the
    shared inventory and the server-global hook log, and one tab's transport
    loss neither touches the other nor blocks its reattach. Mutation: tab A's
    hook line claims tab B's pid — under the pid-bearing plugin contract the
    ancestry check, not the session id, must keep it off tab A."""
    fx.reset_stage(GROK_VERSION)
    seed_installed_grok_made(fx.home, GROK_VERSION)
    extra_a = {'MADE_FIXTURE_SESSION': '1', 'MADE_FIXTURE_CWD': '/tab-a'}
    extra_b = {'MADE_FIXTURE_SESSION': '1', 'MADE_FIXTURE_CWD': '/tab-b'}
    extra_a['MADE_FIXTURE_SESSION_ID'] = 'tab-a-1'
    extra_b['MADE_FIXTURE_SESSION_ID'] = 'tab-b-1'
    require_supervisor(fx, h, workspace)
    tab_a = GrokTab(h, fx, workspace, target='Grok', extra=extra_a)
    tab_b = None
    try:
        tab_a.collect_terminal()
        pid_a = tab_a.wait_pid()
        tab_b = GrokTab(h, fx, workspace, target='Grok', extra=extra_b)
        tab_b.collect_terminal()
        pid_b = tab_b.wait_pid()
        id_a, id_b = extra_a['MADE_FIXTURE_SESSION_ID'], extra_b['MADE_FIXTURE_SESSION_ID']
        rows = '[' + ',\n'.join([
            inventory_row(pid_a, id_a, '/tab-a', '2026-01-01T00:00:01Z'),
            inventory_row(pid_b, id_b, '/tab-b', '2026-01-01T00:00:02Z'),
        ]) + ']'
        remote_write(h, fx.sessions_file, rows)
        snap_a, _ = tab_a.wait_state(lambda s: id_a in session_ids(s), timeout=10)
        if id_a != id_b and id_b in session_ids(snap_a):
            raise AssertionError(f'tab A leaked tab B session: {session_ids(snap_a)}')
        if not snap_a['running']:
            raise AssertionError('tab A lost its running indication')
        snap_b, _ = tab_b.wait_state(lambda s: id_b in session_ids(s), timeout=10)
        if id_a != id_b and id_a in session_ids(snap_b):
            raise AssertionError(f'tab B leaked tab A session: {session_ids(snap_b)}')
        remote_append(h, fx.status_file,
                      hook_line(id_b, '/tab-b', 'UserPromptSubmit', 'working', pid=pid_b))
        tab_b.wait_event(lambda e: e['kind'] == 'Prompt', timeout=8)
        tab_a.expect_no_events(2.0)
        # A hook line for tab A's session, but carrying tab B's pid under the
        # mutation: only the live-pid ancestry check may keep it off tab A.
        remote_append(h, fx.status_file,
                      hook_line(id_a, '/tab-a', 'SessionStart', 'idle',
                                pid=pid_b if mutation else pid_a))
        tab_a.wait_event(lambda e: e['kind'] == 'SessionStart', timeout=8)
        tab_b.expect_no_events(2.0)
        tab_a.kill_transport()
        remote_append(h, fx.status_file,
                      hook_line(id_b, '/tab-b', 'Stop', 'idle', pid=pid_b))
        tab_b.wait_event(lambda e: e['kind'] == 'Stop', timeout=8)
        reattached = GrokTab(h, fx, workspace, identity=tab_a.identity,
                             epoch=tab_a.identity['epoch'] + 1, mode='Attach')
        if reattached.error is not None:
            raise AssertionError(f'the disconnected tab failed to reattach: {reattached.error}')
        snapshot, _ = reattached.state()
        if snapshot['revision'] != 1:
            raise AssertionError('the reattached tab must rebuild a fresh snapshot')
        if not reattached.close():
            raise AssertionError('the reattached tab close was unconfirmed')
        reattached.opened = False
        tab_a.opened = False
        if not tab_b.close():
            raise AssertionError('the surviving tab close was unconfirmed')
        tab_b.opened = False
        for pid in (pid_a, pid_b):
            if pid_alive(h, pid):
                raise AssertionError('a tab child survived its Close')
        return (f'two tabs on one server followed only their own ancestry both ways, the '
                f'disconnected tab (pid {pid_a}) reattached fresh without touching the '
                f'surviving tab (pid {pid_b}), and both closed cleanly; ' + GROK_BOUNDARY)
    finally:
        close_quietly(tab_a)
        close_quietly(tab_b)


def grok_disconnect_reconnect(h, fx, workspace, mutation=False):
    """A transport drop mid-session: the reattached client gets a fresh
    snapshot, pre-disconnect events never replay, outage/post lines apply at
    most once, and stale or mismatched identities are rejected without
    disclosing state. Mutation: the reconnect reuses the stale epoch."""
    fx.reset_stage(GROK_VERSION)
    seed_installed_grok_made(fx.home, GROK_VERSION)
    extra = {'MADE_FIXTURE_SESSION': '1', 'MADE_FIXTURE_SESSION_ID': 're-1',
             'MADE_FIXTURE_CWD': '/re'}
    require_supervisor(fx, h, workspace)
    tab = GrokTab(h, fx, workspace, target='Grok', extra=extra)
    fresh = None
    try:
        tab.collect_terminal()
        pid = tab.wait_pid()
        tab.arm()
        remote_write(h, fx.sessions_file,
                     '[' + inventory_row(pid, 're-1', '/re', 't') + ']')
        tab.wait_state(lambda s: 're-1' in session_ids(s), timeout=10)
        remote_append(h, fx.status_file,
                      hook_line('re-1', '/before', 'UserPromptSubmit', 'working', pid=pid))
        tab.wait_event(lambda e: e['kind'] == 'Prompt', timeout=8)
        epoch = tab.identity['epoch']
        tab.kill_transport()
        remote_append(h, fx.status_file,
                      hook_line('re-1', '/during', 'UserPromptSubmit', 'working', pid=pid))
        stale = GrokTab(h, fx, workspace, identity=tab.identity, epoch=epoch, mode='Attach')
        if stale.error is None:
            raise AssertionError('a stale-epoch attachment was accepted')
        reconnect_epoch = epoch if mutation else epoch + 1
        fresh = GrokTab(h, fx, workspace, identity=tab.identity, epoch=reconnect_epoch,
                        mode='Attach')
        if fresh.error is not None:
            raise AssertionError(f'the reconnect failed: {fresh.error}')
        snapshot, _ = fresh.state()
        if snapshot['revision'] != 1:
            raise AssertionError(f'the reconnect must apply a fresh snapshot: '
                                 f'revision {snapshot["revision"]}')
        if session_ids(snapshot) != ['re-1']:
            raise AssertionError(f'the fresh snapshot lost the session: {snapshot}')
        remote_append(h, fx.status_file,
                      hook_line('re-1', '/after', 'UserPromptSubmit', 'working', pid=pid))
        fresh.wait_event(lambda e: decode_cwd(e['cwd']) == '/after', timeout=10)
        if any(decode_cwd(e['cwd']) == '/before' for e in fresh.events):
            raise AssertionError('a pre-disconnect event replayed after reattachment')
        during = [e for e in fresh.events if decode_cwd(e['cwd']) == '/during']
        if len(during) > 1:
            raise AssertionError(f'an outage event double-applied: {during}')
        impostor_identity = dict(tab.identity)
        impostor_identity['tab'] = list(os.urandom(32))
        impostor = GrokTab(h, fx, workspace, identity=impostor_identity,
                           epoch=reconnect_epoch + 1, mode='Attach')
        if impostor.error is None:
            raise AssertionError('a mismatched tab identity attached')
        if not fresh.close():
            raise AssertionError('the reconnected tab close was unconfirmed')
        fresh.opened = False
        if pid_alive(h, pid):
            raise AssertionError('Close left the launched grok alive')
        return (f'transport drop then reconnect: fresh revision 1 (epoch {reconnect_epoch}), '
                f'stale epoch rejected ({type(stale.error).__name__}), pre-disconnect events '
                f'never replayed, outage/post lines applied at most once, impostor tab '
                f'rejected ({type(impostor.error).__name__}), close confirmed (pid {pid}); '
                f'client-side stale-state discard is model-level. {GROK_BOUNDARY}')
    finally:
        close_quietly(tab)
        close_quietly(fresh)


def grok_supervisor_preflight(h, fx, workspace, mutation=False):
    """One real session Open. The decisive assertion: Attached arrives. When
    the companion supervisor cannot start on this target, the failure is
    diagnosed as a product defect and every session-dependent scenario is
    unrun rather than asserted against a broken surface."""
    reason = supervisor_unavailable(h, fx, workspace)
    if reason is not None:
        raise AssertionError(reason)
    return ('a real companion Open reached Attached over loopback SSH and closed cleanly; '
            'session-dependent Grok scenarios may run. ' + GROK_BOUNDARY)


# ═══ Host-companion update-in-place parity (task 5.7) ══════════════════════
#
# The fixed platform scripts are the shipped source of truth: they are read from
# the current repository and executed over the isolated loopback SSH root. The
# companion binary and plugin files are the real staged payload; the old process
# is the real companion binary launched from the first staged copy.

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]


def current_source_script(name):
    source = (REPO_ROOT / 'crates' / 'made-platform' / 'src' / 'ssh.rs').read_text()
    match = re.search(r'const ' + name + r': &str = r#"(.*?)"#;', source, re.S)
    if not match:
        raise AssertionError(f'the fixed {name} script is absent from the current platform source')
    return match.group(1)


def current_stage_artifact_names():
    source = (REPO_ROOT / 'crates' / 'made-app' / 'src' / 'remote_session.rs').read_text()
    match = re.search(r'pub const STAGE_FILES: &\[&str\] = &\[(.*?)\];', source, re.S)
    if not match:
        raise AssertionError('STAGE_FILES is absent from the current app source')
    return re.findall(r'"([^"]+)"', match.group(1))


def native_remote_target():
    return {
        ('Darwin', 'arm64'): 'aarch64-apple-darwin',
        ('Darwin', 'x86_64'): 'x86_64-apple-darwin',
        ('Linux', 'aarch64'): 'aarch64-unknown-linux-gnu',
        ('Linux', 'x86_64'): 'x86_64-unknown-linux-gnu',
    }.get((platform.system(), platform.machine())) or _unsupported_native_target()


def _unsupported_native_target():
    raise AssertionError(
        f'no fixed staging target for {platform.system()}/{platform.machine()}')


def host_data_root_for(root):
    home = pathlib.Path(str(root)) / 'home'
    if platform.system() == 'Darwin':
        return home / 'Library' / 'Application Support' / 'Made'
    # Matches the fixed Linux branch: XDG_DATA_HOME when absolute, else
    # ~/.local/share. sshd leaves XDG unset, but the companion-launching
    # prefixes used with this harness root pin XDG_DATA_HOME to
    # <home>/.local/share, so both branches name the same directory.
    return home / '.local' / 'share' / 'made'


def host_data_root(h):
    return host_data_root_for(h.root)


NATIVE_HOOK_ARTIFACT = 'plugins/grok-made/hooks/bin/made-grok-status'


def native_hook_path(companion):
    """The native hook the host target ships, found beside the companion's own
    staged package (the CI artifact) or in a local cargo build. The repository
    `plugins/grok-made` is source only and never holds it."""
    companion = pathlib.Path(companion)
    candidates = [companion.parent / NATIVE_HOOK_ARTIFACT]
    candidates += [REPO_ROOT / 'target' / profile / 'made-grok-status'
                   for profile in ('release', 'debug')]
    for candidate in candidates:
        if candidate.is_file() and os.access(candidate, os.R_OK) and candidate.stat().st_size > 0:
            return candidate
    raise PrerequisiteError(
        'no native made-grok-status hook beside the companion or in target/; build one '
        'with `cargo build -p made-grok-hook`. A missing hook is unrun, not a pass')


def staged_artifact_bytes(companion):
    """The current real packaged artifact set, one byte map keyed by STAGE_FILES."""
    files = {}
    for name in current_stage_artifact_names():
        if name == 'made-remote':
            source = pathlib.Path(companion)
        elif name == NATIVE_HOOK_ARTIFACT:
            source = native_hook_path(companion)
        elif name == CLAUDE_HOOK_ARTIFACT:
            source = native_claude_hook_path(companion)
        else:
            source = REPO_ROOT / name
        files[name] = source.read_bytes()
    return files


def stage_payload(names, files, version, target, approved, declared=None):
    """The fixed stage wire body: header, then size/hash/raw bytes per artifact."""
    body = b'consent-v2-update\n' if approved else b'consent-v1\n'
    body += version.encode() + b'\n' + target.encode() + b'\n'
    for name in names:
        data = files[name]
        digest = (declared or {}).get(name) or hashlib.sha256(data).hexdigest()
        body += f'{len(data)}\n{digest}\n'.encode() + data
    return body


def tree_fingerprint(root):
    """Bytes, modes and inodes of one staged tree; reads never change it."""
    fingerprint = {}
    if not root.is_dir():
        return fingerprint
    for path in sorted(root.rglob('*')):
        metadata = path.lstat()
        digest = (hashlib.sha256(path.read_bytes()).hexdigest()
                  if path.is_file() and not path.is_symlink() else None)
        fingerprint[str(path.relative_to(root))] = (
            metadata.st_size, metadata.st_mode & 0o777,
            metadata.st_ino, metadata.st_mtime_ns, digest)
    return fingerprint


def directory_entries(root):
    return sorted(entry.name for entry in root.iterdir()) if root.is_dir() else []


def assert_no_staging_temporaries(version_root):
    leftovers = [name for name in sorted(entry.name for entry in version_root.iterdir())
                 if name.startswith('.stage') or name.startswith('.replace')]
    if leftovers:
        raise AssertionError(f'staging temporary/replaced directories accumulated: {leftovers}')


def grok_update_in_place(h, fx, workspace, mutation=False):
    """Same app version A→B update: no write before approval, verified swap,
    running-process continuity, resolved future paths, cleaned temporaries and
    exact reuse afterwards. Mutation: send consent-v2-update for the unapproved
    B attempt, the write the refusal control exists to prevent."""
    del workspace
    version = '1.0'
    target = native_remote_target()
    names = current_stage_artifact_names()
    stage_script = current_source_script('STAGE')
    companion_script = current_source_script('COMPANION')
    install_script = current_source_script('GROK_PLUGIN_INSTALL')
    data_root = host_data_root(h)
    # Own this scenario's isolated data root even when a previous scenario in the
    # same disposable harness staged the same version.
    shutil.rmtree(data_root, ignore_errors=True)
    version_root = data_root / 'remote' / version
    destination = version_root / target

    files_a = staged_artifact_bytes(fx.host_companion)
    updated_companion = b'#!/bin/sh\nprintf \'updated:%s\\n\' "$1"\n'
    files_b = dict(files_a)
    files_b['made-remote'] = updated_companion
    files_b['plugins/grok-made/plugin.json'] = (
        b'{"name":"grok-made","version":"1.0","fixture":"updated"}\n')
    companion_only = dict(files_a)
    companion_only['made-remote'] = updated_companion
    plugin_only = dict(files_a)
    plugin_only['plugins/grok-made/plugin.json'] = files_b['plugins/grok-made/plugin.json']

    def run_stage(body):
        return h.ssh_run_input(stage_script, body, timeout=60)

    def approved_stage(files):
        return run_stage(stage_payload(names, files, version, target, True))

    process = None
    try:
        initial = run_stage(stage_payload(names, files_a, version, target, False))
        if initial.returncode != 0 or initial.stdout != b'MADE-STAGED-1\n':
            raise AssertionError(
                'the real fixed stage script rejected the real artifact set A: '
                f'exit {initial.returncode}; '
                f'{initial.stderr.decode(errors="replace").strip()}')
        before = tree_fingerprint(destination)

        reuse = run_stage(stage_payload(names, files_a, version, target, False))
        if reuse.returncode != 0 or tree_fingerprint(destination) != before:
            raise AssertionError('exact reuse rewrote or deleted the staged A artifacts')

        for label, changed in (('companion', companion_only), ('plugin', plugin_only)):
            mismatch = run_stage(stage_payload(names, changed, version, target, False))
            if tree_fingerprint(destination) != before:
                raise AssertionError(
                    f'a same-version {label}-byte mismatch without approval wrote or deleted '
                    'artifacts')
            if mismatch.returncode != 74 or b'MADE-STAGE-UPDATE-REQUIRED' not in mismatch.stderr:
                raise AssertionError(
                    f'a changed {label} artifact must report the fixed update-required '
                    f'marker with exit 74; got exit {mismatch.returncode}: '
                    f'{mismatch.stderr.decode(errors="replace").strip()}')
            disclosed = {hashlib.sha256(changed[name]).hexdigest().encode()
                         for name in names}
            if (str(h.root).encode() in mismatch.stderr
                    or any(digest in mismatch.stderr for digest in disclosed)):
                raise AssertionError('update-required disclosed a path or artifact hash')

        # No approval: the B set must not write, replace or delete anything.
        refused = run_stage(stage_payload(names, files_b, version, target, mutation))
        if tree_fingerprint(destination) != before:
            raise AssertionError(
                'a same-version B mismatch without explicit approval wrote or deleted artifacts')
        if refused.returncode != 74 or b'MADE-STAGE-UPDATE-REQUIRED' not in refused.stderr:
            raise AssertionError(
                'the no-approval control must keep the original A bytes and report '
                f'update-required; got exit {refused.returncode}: '
                f'{refused.stderr.decode(errors="replace").strip()}')
        if str(h.root).encode() in refused.stderr:
            raise AssertionError('update-required disclosed the isolated home path')
        assert_no_staging_temporaries(version_root)

        # Approved updates that fail before the swap leave A byte-identical and
        # clean their private temporary sibling.
        corrupt = run_stage(stage_payload(
            names, files_b, version, target, True, declared={'made-remote': '0' * 64}))
        if corrupt.returncode == 0:
            raise AssertionError('an approved update with a corrupt declared hash succeeded')
        if tree_fingerprint(destination) != before:
            raise AssertionError('a pre-swap hash failure changed the original artifacts')
        assert_no_staging_temporaries(version_root)
        truncated = run_stage(stage_payload(names, files_b, version, target, True)[:-1])
        if truncated.returncode == 0:
            raise AssertionError('a truncated approved update succeeded')
        if tree_fingerprint(destination) != before:
            raise AssertionError('a pre-swap size failure changed the original artifacts')
        assert_no_staging_temporaries(version_root)

        # A real process launched from A must survive the directory swap.
        process = subprocess.Popen(
            [str(destination / 'made-remote'), '--stdio'],
            stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if process.poll() is not None:
            raise AssertionError('the real companion process launched from A exited immediately')
        old_pid = process.pid
        os.kill(old_pid, 0)

        update = approved_stage(files_b)
        if update.returncode != 0 or update.stdout != b'MADE-STAGED-1\n':
            raise AssertionError(
                f'the approved update failed: exit {update.returncode}; '
                f'{update.stderr.decode(errors="replace").strip()}')
        for name in names:
            path = destination / name
            if not path.is_file() or path.read_bytes() != files_b[name]:
                raise AssertionError(f'the approved update left the wrong bytes for {name}')
            expected_mode = (0o700 if name in ('made-remote', NATIVE_HOOK_ARTIFACT,
                                               CLAUDE_HOOK_ARTIFACT) else 0o600)
            if path.stat().st_mode & 0o777 != expected_mode:
                raise AssertionError(
                    f'{name} has mode {path.stat().st_mode & 0o777:o}, not '
                    f'{expected_mode:o}')
        if directory_entries(version_root) != [target]:
            raise AssertionError(
                f'build/content directories accumulated beside the target: '
                f'{directory_entries(version_root)}')
        if directory_entries(data_root / 'remote') != [version]:
            raise AssertionError(
                f'more than one version directory accumulated: '
                f'{directory_entries(data_root / "remote")}')
        assert_no_staging_temporaries(version_root)
        if process.poll() is not None:
            raise AssertionError('the approved update signalled or killed the running A process')
        os.kill(old_pid, 0)

        # Future companion and Grok-install launches resolve the same updated
        # current version/target path and pass the expected mode.
        header = f'{version}\n{target}\n'.encode()
        companion_launch = h.ssh_run_input(companion_script, header, timeout=30)
        if companion_launch.returncode != 0 or companion_launch.stdout != b'updated:--stdio\n':
            raise AssertionError(
                'the companion launch did not resolve the updated path: '
                f'exit {companion_launch.returncode} '
                f'{companion_launch.stdout!r} '
                f'{companion_launch.stderr.decode(errors="replace").strip()}')
        install_launch = h.ssh_run_input(install_script, header, timeout=30)
        if (install_launch.returncode != 0
                or install_launch.stdout != b'updated:--install-grok-plugin\n'):
            raise AssertionError(
                'the Grok-install launch did not resolve the updated path: '
                f'exit {install_launch.returncode} '
                f'{install_launch.stdout!r} '
                f'{install_launch.stderr.decode(errors="replace").strip()}')

        # The fourth same-version attempt over B is exact reuse, not a rewrite.
        updated = tree_fingerprint(destination)
        after_update = run_stage(stage_payload(names, files_b, version, target, False))
        if after_update.returncode != 0 or tree_fingerprint(destination) != updated:
            raise AssertionError('post-update exact reuse rewrote or deleted updated artifacts')
        assert_no_staging_temporaries(version_root)

        if pid_alive_local(old_pid) is False:
            raise AssertionError('the running A process disappeared before teardown')
        return (
            'real fixed STAGE/COMPANION/GROK_PLUGIN_INSTALL scripts over loopback SSH: '
            f'{len(names)}-file real artifact set A staged; changed companion and plugin '
            'bytes under the same version reported update-required with zero writes or '
            'deletes until explicit consent-v2-update; the verified swap cleaned its '
            f'temporary and replaced directories; real companion pid {old_pid} from A '
            'stayed alive; later companion and Grok-install launches resolved the updated '
            'current version/target path; the post-update attempt reused exactly. Desktop '
            'native GUI approval is outside this harness: script/companion/protocol '
            'evidence only. ' + GROK_BOUNDARY)
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        if process is not None:
            for stream in (process.stdin, process.stdout, process.stderr):
                if stream:
                    stream.close()


def pid_alive_local(pid):
    try:
        os.kill(int(pid), 0)
    except OSError:
        return False
    return True


GROK_SCENARIOS = (
    ('supervisor-preflight', grok_supervisor_preflight, False),
    ('update-in-place', grok_update_in_place, False),
    ('detection-probe', grok_detection_probe, False),
    ('direct-launch', grok_direct_launch, True),
    ('sessionless-branding', grok_sessionless_branding, True),
    ('manual-launch', grok_manual_launch, True),
    ('inventory-follow', grok_inventory_follow, True),
    ('open-close-lifecycle', grok_open_close_lifecycle, True),
    ('consent-refused', grok_consent_refused, True),
    ('install-without-python3', grok_install_without_python3, False),
    ('install-failed', grok_install_failed, False),
    ('consented-install-hooks', grok_consented_install_hooks, True),
    ('native-hook-without-python3', grok_native_hook_without_python3, True),
    ('opencode-precedence', grok_opencode_precedence, True),
    ('repeated-switches-attach-filtering', grok_repeated_switches, True),
    ('two-tab-isolation', grok_two_tab_isolation, True),
    ('disconnect-reconnect', grok_disconnect_reconnect, True),
)


def resolve_host_companion(explicit):
    """A missing companion binary is unrun, never a pass."""
    root = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
    candidates = [explicit] if explicit else [
        os.path.join(root, 'target', 'debug', 'made-remote'),
        os.path.join(root, 'target', 'release', 'made-remote'),
    ]
    for candidate in candidates:
        if candidate and os.path.isfile(candidate) and os.access(candidate, os.X_OK):
            return os.path.abspath(candidate)
    raise PrerequisiteError(
        'no executable companion binary; build one locally with '
        '`cargo build -p made-remote` or pass --host-companion. A missing '
        'companion is unrun, not a pass')


def require_supervisor(fx, h, workspace):
    """Gate for session-dependent scenarios. A diagnosed supervisor-start
    failure is an unrun, not a behavioral failure."""
    reason = supervisor_unavailable(h, fx, workspace)
    if reason is not None:
        raise SupervisorPreflightError(reason)


def run_grok_scenarios(harness, fixture, workspace, mode, only):
    results = []
    for name, scenario, needs_session in GROK_SCENARIOS:
        if only and name not in only:
            continue
        fixture.reset_home()
        fixture.reset_stage()
        fixture.reset_logs()
        if needs_session:
            reason = supervisor_unavailable(harness, fixture, workspace)
            if reason is not None:
                print(f'BLOCKED grok/{name}: scenario unrun — {reason.splitlines()[0]}')
                results.append((name, 'blocked', reason))
                continue
        try:
            if mode == 'grok-first-fail':
                evidence = scenario(harness, fixture, workspace, mutation=True)
                if name == 'supervisor-preflight':
                    # The gate has no fixture mutation; in first-fail mode its
                    # pass is a pass, never an unexpected one.
                    print(f'PASS grok/{name} — {evidence}')
                    results.append((name, 'pass', evidence))
                else:
                    print(f'UNEXPECTED-PASS grok/{name}: {evidence}')
                    results.append((name, 'unexpected-pass', evidence))
            else:
                evidence = scenario(harness, fixture, workspace)
                print(f'PASS grok/{name} — {evidence}')
                results.append((name, 'pass', evidence))
        except SupervisorPreflightError as exc:
            print(f'BLOCKED grok/{name}: {exc}')
            results.append((name, 'blocked', str(exc)))
        except (AssertionError, TimeoutError, EOFError, ValueError,
                subprocess.SubprocessError, OSError) as exc:
            detail = f'{type(exc).__name__}: {exc}'
            if mode == 'grok-first-fail' and name != 'supervisor-preflight':
                print(f'FIRST-FAIL grok/{name}: {detail}')
                results.append((name, 'first-fail', detail))
            else:
                print(f'FAIL grok/{name}: {detail}', file=sys.stderr)
                results.append((name, 'fail', detail))
    counts = {}
    for _, status, _ in results:
        counts[status] = counts.get(status, 0) + 1
    tally = ', '.join(f'{key}={value}' for key, value in sorted(counts.items())) or 'none ran'
    print(f'SUMMARY grok/{mode}: {tally}')
    if mode == 'grok-first-fail':
        decided = [status for _, status, _ in results
                   if status in ('first-fail', 'unexpected-pass')]
        clean = (decided and all(status == 'first-fail' for status in decided)
                 and not counts.get('fail') and not counts.get('unexpected-pass'))
        return 0 if clean else 2
    return 0 if counts.get('pass') and not counts.get('fail') else 2


# ═══ Claude Code parity (SSH-11, tasks 5.1/5.2) ═════════════════════════════
#
# The claude-parity mode drives the real companion entry points (--stdio
# frontend and its supervisor) over the isolated loopback SSH harness with a
# fixture `claude`. The fixture never is real Claude Code: it fires the real
# staged native hook (`made-claude-status`) with crafted payloads, emits status
# protocol bytes, and answers `agents --json` from a scenario-controlled file.
# It certifies the companion/protocol surface only; desktop presentation,
# precedence and diagnostics are model-level (crates/made-app tests), and real
# claude over SSH is the manual task 5.4.

CLAUDE_ARTIFACTS = (
    'plugins/claude-code-made/.claude-plugin/plugin.json',
    'plugins/claude-code-made/README.md',
    'plugins/claude-code-made/hooks/hooks.json',
    'plugins/claude-code-made/bin/made-claude-status',
)
CLAUDE_HOOK_ARTIFACT = 'plugins/claude-code-made/bin/made-claude-status'
CLAUDE_MARKER = 'MADE-PRIVACY-MARKER-7f3c'
CLAUDE_BOUNDARY = ('real loopback SSH, the real companion supervisor and the real staged '
                   'native hook, driven by a fixture claude (not real Claude Code); desktop '
                   'presentation, precedence and diagnostics are model-level (crates/made-app '
                   'tests); native GUI and the live-SSH matrix are not certified here')

CLAUDE_FIXTURE_SCRIPT = r'''#!/bin/sh
# Fixture claude for the claude-parity loopback scenarios. It is never real
# Claude Code: it fires the hook Made staged, prints status protocol bytes and
# answers `agents --json` from a scenario-controlled file. Its own tools may
# sit outside the fixture PATH (macOS keeps sed and wc in /usr/bin).
PATH="$PATH:/usr/bin:/bin"
if [ "$#" = 2 ] && [ "$1" = agents ] && [ "$2" = --json ]; then
  printf 'argv:%s\n' "$*" >> '__ARGV_LOG__'
  cat '__AGENTS__' 2>/dev/null || printf '[]\n'
  exit 0
fi
printf 'argv:%s\n' "$*" >> '__ARGV_LOG__'
printf 'pid=%s\tdirs=%s\tspool=%s\n' "$$" "${CLAUDE_CODE_PLUGIN_DIRS-<unset>}" \
  "${MADE_CLAUDE_SPOOL-<unset>}" >> '__ENV_LOG__'
plugin=
if [ "$1" != --bare ]; then
  old_ifs=$IFS
  IFS=:
  for entry in ${CLAUDE_CODE_PLUGIN_DIRS-}; do
    case "$entry" in */plugins/claude-code-made) plugin=$entry ;; esac
  done
  IFS=$old_ifs
fi
ctl='__CTL_DIR__'/$$.ctl
: > "$ctl"
printf 'MADE_FIXTURE_CLAUDE:%s\n' "$$"
seen=0
while :; do
  total=$(wc -l < "$ctl" | tr -d ' ')
  while [ "$seen" -lt "$total" ]; do
    seen=$((seen + 1))
    line=$(sed -n "${seen}p" "$ctl")
    verb=${line%% *}
    arg=${line#* }
    case "$verb" in
      hook) if [ -n "$plugin" ]; then "$plugin/bin/made-claude-status" < "$arg"; fi ;;
      emit) cat "$arg" ;;
      cd) cd "$arg" || exit 3 ;;
      exit) rm -f "$ctl"; exit 0 ;;
    esac
  done
  sleep 0.1
done
'''


def claude_hook_payload(event, session, cwd, title='', notification=None):
    """One Claude Code hook payload: the metadata fields the hook copies plus
    content fields carrying the privacy marker, which must never be copied."""
    payload = {
        'hook_event_name': event,
        'session_id': session,
        'cwd': cwd,
        'transcript_path': f'/tmp/{CLAUDE_MARKER}/transcript.jsonl',
        'prompt': f'prompt {CLAUDE_MARKER}',
        'message': f'message {CLAUDE_MARKER}',
        'tool_name': f'tool-{CLAUDE_MARKER}',
        'tool_input': {'command': f'echo {CLAUDE_MARKER}'},
    }
    if title:
        payload['session_title'] = title
    if notification:
        payload['notification_type'] = notification
    return json.dumps(payload)


def native_claude_hook_path(companion):
    """The native Claude Code hook the host target ships: beside the
    companion's own staged package (the CI artifact), else a local cargo
    build. A missing hook is unrun, never a pass."""
    companion = pathlib.Path(companion)
    candidates = [companion.parent / CLAUDE_HOOK_ARTIFACT]
    candidates += [REPO_ROOT / 'target' / profile / 'made-claude-status'
                   for profile in ('release', 'debug')]
    for candidate in candidates:
        if candidate.is_file() and os.access(candidate, os.X_OK) and candidate.stat().st_size > 0:
            return candidate
    raise PrerequisiteError(
        'no native made-claude-status hook beside the companion or in target/; build one '
        'with `cargo build -p made-claude-hook`. A missing hook is unrun, not a pass')


class ClaudeFixture(GrokFixture):
    """The Grok fixture root plus a fixture claude, its control directory and
    logs, a seeded remote `~/.claude` and the staged claude-code-made package
    beside the companion. Everything stays inside the disposable root."""

    def __init__(self, root, companion, hook=None):
        super().__init__(root, companion)
        self.hook_binary = pathlib.Path(hook) if hook else None
        self.claudebin = self.root / 'claudebin'
        self.claudebin.mkdir(mode=0o700, exist_ok=True)
        self.ctl_dir = self.root / 'claude-ctl'
        self.ctl_dir.mkdir(mode=0o700, exist_ok=True)
        self.payload_dir = self.root / 'claude-payloads'
        self.payload_dir.mkdir(mode=0o700, exist_ok=True)
        self.argv_log = self.root / 'claude-argv.log'
        self.env_log = self.root / 'claude-env.log'
        self.agents_file = self.root / 'claude-agents.json'
        script = (CLAUDE_FIXTURE_SCRIPT
                  .replace('__ARGV_LOG__', str(self.argv_log))
                  .replace('__ENV_LOG__', str(self.env_log))
                  .replace('__AGENTS__', str(self.agents_file))
                  .replace('__CTL_DIR__', str(self.ctl_dir)))
        claude = self.claudebin / 'claude'
        claude.write_text(script)
        claude.chmod(0o755)
        self.claude_scripts = {'claude': claude}
        self.payloads = 0
        self.reset_claude_logs()

    @property
    def claude_home(self):
        return self.root / 'home' / '.claude'

    def seed_claude_home(self):
        """The remote user's own Claude Code configuration and state."""
        home = self.claude_home
        (home / 'sessions').mkdir(parents=True, exist_ok=True)
        (home / 'plugins').mkdir(parents=True, exist_ok=True)
        (home / 'settings.json').write_text('{"hooks": {}, "statusLine": null}\n')
        (home / 'sessions' / 'existing.json').write_text('{"id": "existing"}\n')
        (home / 'plugins' / 'installed_plugins.json').write_text('{"plugins": {}}\n')

    def stage_claude_package(self):
        """The four shipped claude-code-made artifacts beside the companion, with
        the native hook for this host target and the SSH-06 hook mode."""
        hook = self.hook_binary or native_claude_hook_path(self.host_companion)
        staged = self.companion_dir / 'plugins' / 'claude-code-made'
        for name in CLAUDE_ARTIFACTS:
            target = self.companion_dir / name
            target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            source = hook if name == CLAUDE_HOOK_ARTIFACT else REPO_ROOT / name
            shutil.copyfile(source, target)
            target.chmod(0o700 if name == CLAUDE_HOOK_ARTIFACT else 0o644)
        return staged

    def reset_claude_logs(self):
        for path in (self.argv_log, self.env_log):
            path.write_text('')
        self.set_agents([])
        for entry in self.ctl_dir.iterdir():
            entry.unlink()

    def set_agents(self, entries):
        self.agents_file.write_text(json.dumps(entries))

    def claude_argv(self):
        return [line[len('argv:'):] for line in self.argv_log.read_text().splitlines()
                if line.startswith('argv:')]

    def claude_env(self, pid):
        for line in self.env_log.read_text().splitlines():
            fields = dict(part.split('=', 1) for part in line.split('\t') if '=' in part)
            if fields.get('pid') == str(pid):
                return fields
        raise AssertionError(f'the fixture claude {pid} logged no environment')

    def claude_prefix(self, claude=True, plugin_dirs=None, extra=None):
        values = dict(extra or {})
        if plugin_dirs is not None:
            values['CLAUDE_CODE_PLUGIN_DIRS'] = plugin_dirs
        prefix = self.prefix(grok=False, extra=values)
        if claude:
            prefix = prefix.replace("PATH='", "PATH='" + str(self.claudebin) + ':', 1) \
                if "PATH='" in prefix else prefix.replace(
                    'PATH=', 'PATH=' + str(self.claudebin) + ':', 1)
        return prefix

    def control(self, pid, verb, arg=''):
        path = self.ctl_dir / f'{pid}.ctl'
        deadline = time.monotonic() + 5
        while not path.exists():
            if time.monotonic() > deadline:
                raise AssertionError(f'the fixture claude {pid} has no control file')
            time.sleep(0.05)
        with path.open('a') as handle:
            handle.write(f'{verb} {arg}\n')

    def hook(self, pid, event, session, cwd, title='', notification=None):
        self.payloads += 1
        payload = self.payload_dir / f'{self.payloads}.json'
        payload.write_text(claude_hook_payload(event, session, cwd, title, notification))
        self.control(pid, 'hook', str(payload))

    def emit(self, pid, data):
        self.payloads += 1
        path = self.payload_dir / f'{self.payloads}.bin'
        path.write_bytes(data)
        self.control(pid, 'emit', str(path))


class ClaudeTab(GrokTab):
    """One live companion frontend with the Claude Code operations: ClaudeState
    round trips, ClaudeTrigger acknowledgements, and ClaudeEvent/ClaudeListing
    pushes collected. Every received frame is kept for the privacy search."""

    def __init__(self, h, fx, workspace, target='Shell', claude=True, plugin_dirs=None,
                 extra=None, **kwargs):
        self.claude_on_path = claude
        self.plugin_dirs = plugin_dirs
        self.claude_extra = extra
        self.claude_events = []
        self.listings = []
        self.raw = []
        self.last_claude = None
        super().__init__(h, fx, workspace, target=target, **kwargs)

    def _connect(self, target, grok, python3, opencode, extra, mode, attach_timeout):
        prefix = self.fx.claude_prefix(self.claude_on_path, self.plugin_dirs, self.claude_extra)
        self.process = self.h.ssh_process(self.fx.frontend_command(prefix))
        self.send, self.wait = protocol(self.process, self.identity)
        if mode == 'Open':
            self.send('Open', {'directory': list(os.fsencode(str(self.workspace))),
                               'target': target, 'rows': 24, 'cols': 80})
        else:
            self.send('Attach', {'rows': 24, 'cols': 80})
        self.wait('Attached', timeout=attach_timeout)
        self.opened = True

    def receive_frame(self, timeout):
        frame = super().receive_frame(timeout)
        self.raw.append(json.dumps(frame))
        return frame

    def absorb(self, operation):
        name = operation.get('op')
        if name == 'ClaudeEvent':
            self.stream_ops.append(name)
            self.last_claude = (operation.get('data') or {}).get('snapshot') or {}
            self.claude_events.append(self.last_claude)
            return False
        if name == 'ClaudeListing':
            self.stream_ops.append(name)
            self.listings.append((operation.get('data') or {}).get('listing') or {})
            return False
        return super().absorb(operation)

    def pump(self, timeout):
        try:
            frame = self.receive_frame(timeout)
        except TimeoutError:
            return None
        self.absorb(frame['operation'])
        return frame

    def claude_state(self, timeout=10):
        """One ClaudeState round trip: the reply waits for the resnapshot."""
        request = self.send('ClaudeState')
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                frame = self.receive_frame(min(0.25, max(0.01, deadline - time.monotonic())))
            except TimeoutError:
                continue
            operation = frame['operation']
            if frame.get('request_id') == request and operation.get('op') == 'ClaudeSnapshot':
                self.last_claude = (operation.get('data') or {}).get('snapshot') or {}
                return self.last_claude
            self.absorb(operation)
        raise TimeoutError('ClaudeState was not answered')

    def trigger(self, timeout=8):
        request = self.send('ClaudeTrigger')
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                frame = self.receive_frame(min(0.25, max(0.01, deadline - time.monotonic())))
            except TimeoutError:
                continue
            if frame.get('request_id') == request and frame['operation'].get('op') == 'Ack':
                return
            self.absorb(frame['operation'])
        raise TimeoutError('ClaudeTrigger was not acknowledged')

    def wait_claude(self, predicate, timeout=8, what='Claude Code state'):
        deadline = time.monotonic() + timeout
        while True:
            if self.last_claude is not None and predicate(self.last_claude):
                return self.last_claude
            if time.monotonic() > deadline:
                raise TimeoutError(f'{what} not reached; last: {self.last_claude!r}')
            self.pump(min(0.25, max(0.01, deadline - time.monotonic())))

    def wait_listing(self, predicate, timeout=8):
        deadline = time.monotonic() + timeout
        while True:
            for listing in self.listings:
                if predicate(listing):
                    return listing
            if time.monotonic() > deadline:
                raise TimeoutError(f'no matching ClaudeListing; listings: {self.listings!r}')
            self.pump(min(0.25, max(0.01, deadline - time.monotonic())))

    def quiet(self, window):
        """Collect frames for `window` seconds and return the Claude events seen."""
        seen = len(self.claude_events)
        deadline = time.monotonic() + window
        while time.monotonic() < deadline:
            self.pump(min(0.25, max(0.01, deadline - time.monotonic())))
        return self.claude_events[seen:]

    def claude_pid(self, timeout=10):
        return int(self.wait_terminal(r'MADE_FIXTURE_CLAUDE:([0-9]+)', timeout=timeout).group(1))

    def type_claude(self, command='claude'):
        """Type a claude command in the remote shell and return the new pid."""
        before = set(re.findall(r'MADE_FIXTURE_CLAUDE:([0-9]+)',
                                self.terminal.decode('utf-8', 'replace')))
        self.type(command + '\n')
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            found = set(re.findall(r'MADE_FIXTURE_CLAUDE:([0-9]+)',
                                   self.terminal.decode('utf-8', 'replace'))) - before
            if found:
                return int(found.pop())
            self.pump(0.25)
        raise TimeoutError('the typed claude did not start')


def claude_sessions(snapshot):
    return [(s['session_id'], s['pid'], decode_cwd(s.get('cwd')), s['activity'])
            for s in (snapshot or {}).get('sessions', [])]


def claude_session_cwd(snapshot, session):
    for entry in (snapshot or {}).get('sessions', []):
        if entry['session_id'] == session:
            return decode_cwd(entry.get('cwd'))
    return None


def claude_workdirs(fx, *names):
    paths = []
    for name in names:
        path = fx.root / 'claude-work' / name
        path.mkdir(parents=True, exist_ok=True)
        paths.append(str(path))
    return paths


def staged_claude_dir(fx):
    return str(fx.companion_dir / 'plugins' / 'claude-code-made')


def claude_direct_launch(h, fx, workspace):
    """`Claude Code SSH`: the detected claude starts with no arguments directly
    as the tab child, its environment has only the staged entry (the remote
    user has none) plus this tab's spool, no host value travels, and its hook
    records become an owned session."""
    os.environ['CLAUDE_CODE_PLUGIN_DIRS'] = '/host-only/plugin'
    os.environ['MADE_CLAUDE_SPOOL'] = '/host-only/spool'
    tab = ClaudeTab(h, fx, workspace, target='Claude')
    try:
        tab.collect_terminal()
        pid = tab.claude_pid()
        if fx.claude_argv() != ['']:
            raise AssertionError(f'direct launch argv is not empty: {fx.claude_argv()}')
        env = fx.claude_env(pid)
        if env['dirs'] != staged_claude_dir(fx):
            raise AssertionError(f'direct launch plugin dirs: {env["dirs"]}')
        if ('host-only' in env['spool'] or '/remote/runtime/' not in env['spool']
                or not env['spool'].endswith('/spool.jsonl')):
            raise AssertionError(f'direct launch spool binding: {env["spool"]}')
        state = tab.claude_state()
        if not state['hooks_available']:
            raise AssertionError('the staged package is unavailable for the direct tab')
        (work,) = claude_workdirs(fx, 'direct')
        fx.hook(pid, 'SessionStart', 'direct-1', work)
        snapshot = tab.wait_claude(lambda s: claude_session_cwd(s, 'direct-1') == work)
        if claude_sessions(snapshot)[0][1] != pid:
            raise AssertionError(f'the direct session is not owned by the child: {snapshot}')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        if pid_alive(h, pid):
            raise AssertionError('the directly launched claude survived Close')
        return (f'Open/Claude launched the detected claude with no arguments (pid {pid}), the '
                f'staged plugin entry and this tab\'s spool only, no host value, and an owned '
                f'session in {work}; {CLAUDE_BOUNDARY}')
    finally:
        os.environ.pop('CLAUDE_CODE_PLUGIN_DIRS', None)
        os.environ.pop('MADE_CLAUDE_SPOOL', None)
        close_quietly(tab)


def claude_protocol_only_bare(h, fx, workspace):
    """`claude --bare` loads no plugin: the probe, title and status bytes pass
    through the remote terminal unchanged, no session is ever owned, and
    after exit the tab owns nothing."""
    tab = ClaudeTab(h, fx, workspace)
    try:
        tab.collect_terminal()
        tab.claude_state()
        pid = tab.type_claude('claude --bare')
        sequences = [b'\x1b]7501;?\x07', b'\x1b]0;Bare session\x07',
                     b'\x1b]7501;state=working:app=claude-code\x07',
                     b'\x1b]7501;state=blocked:app=claude-code\x07',
                     b'\x1b]7501;state=done:app=claude-code\x07']
        (work,) = claude_workdirs(fx, 'bare')
        fx.emit(pid, b''.join(sequences))
        fx.hook(pid, 'SessionStart', 'bare-1', work)
        deadline = time.monotonic() + 8
        while not all(seq in bytes(tab.terminal) for seq in sequences):
            if time.monotonic() > deadline:
                raise AssertionError('the status protocol bytes were not relayed verbatim')
            tab.pump(0.25)
        events = tab.quiet(2.0)
        if any(claude_sessions(event) for event in events):
            raise AssertionError(f'--bare produced an owned session: {events}')
        fx.control(pid, 'exit')
        tab.quiet(1.5)
        if pid_alive(h, pid):
            raise AssertionError('the bare claude did not exit')
        if claude_sessions(tab.claude_state()):
            raise AssertionError('a session survived the bare exit')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        return ('`claude --bare` in a remote shell: probe, title and working/blocked/done bytes '
                'relayed verbatim through the remote terminal, no owned session, nothing after '
                'exit; the probe reply, dot and tooltip are model-level; ' + CLAUDE_BOUNDARY)
    finally:
        close_quietly(tab)


def claude_shell_typed_plugin_dirs(h, fx, workspace):
    """A shell-typed claude whose remote user already has two plugin entries
    gets them in order plus Made's single staged entry; Made's entry already
    present is not added twice; the user's `~/.claude` is untouched."""
    fx.seed_claude_home()
    before = tree_fingerprint(fx.claude_home)
    user = '/u/one:/u/two'
    tab = ClaudeTab(h, fx, workspace, plugin_dirs=user)
    again = None
    try:
        tab.collect_terminal()
        pid = tab.type_claude()
        dirs = fx.claude_env(pid)['dirs']
        if dirs != user + ':' + staged_claude_dir(fx):
            raise AssertionError(f'composed plugin dirs: {dirs}')
        fx.control(pid, 'exit')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        already = user + ':' + staged_claude_dir(fx)
        again = ClaudeTab(h, fx, workspace, plugin_dirs=already)
        again.collect_terminal()
        pid = again.type_claude()
        if fx.claude_env(pid)['dirs'] != already:
            raise AssertionError('Made\'s entry was added a second time')
        fx.control(pid, 'exit')
        if not again.close():
            raise AssertionError('remote cleanup unconfirmed')
        if tree_fingerprint(fx.claude_home) != before:
            raise AssertionError('the remote user\'s ~/.claude changed')
        return ('shell-typed claude: user entries kept in order plus one staged entry, an '
                'already-present entry not duplicated, ~/.claude byte-identical; '
                + CLAUDE_BOUNDARY)
    finally:
        close_quietly(tab)
        close_quietly(again)


def claude_hook_follow(h, fx, workspace):
    """Hook follow without the protocol: session start in another directory,
    prompt, permission request, stop and a rename move the relayed session
    directory and drive idle, busy, attention, idle with the sanitized title,
    each within the SSH-09 two-second budget."""
    tab = ClaudeTab(h, fx, workspace)
    try:
        tab.collect_terminal()
        tab.claude_state()
        pid = tab.type_claude()
        first, second = claude_workdirs(fx, 'follow-a', 'follow-b')
        budgets = []
        steps = [
            (('SessionStart', 'f-1', first), lambda s: claude_session_cwd(s, 'f-1') == first
             and s['sessions'][0]['activity'] == 'Idle'),
            (('UserPromptSubmit', 'f-1', second),
             lambda s: claude_session_cwd(s, 'f-1') == second
             and s['sessions'][0]['activity'] == 'Busy'),
            (('PermissionRequest', 'f-1', second),
             lambda s: s['sessions'][0]['activity'] == 'Attention'),
            (('Stop', 'f-1', second), lambda s: s['sessions'][0]['activity'] == 'Idle'),
        ]
        for (event, session, cwd), predicate in steps:
            started = time.monotonic()
            fx.hook(pid, event, session, cwd)
            tab.wait_claude(predicate, timeout=4, what=event)
            budgets.append(time.monotonic() - started)
        started = time.monotonic()
        fx.hook(pid, 'UserPromptSubmit', 'f-1', second, title='Renamed‮ task\nnow')
        snapshot = tab.wait_claude(lambda s: s['sessions'][0]['title'].startswith('Renamed'),
                                   timeout=4, what='rename')
        budgets.append(time.monotonic() - started)
        title = snapshot['sessions'][0]['title']
        if '‮' in title or '\n' in title:
            raise AssertionError(f'the relayed title is not sanitized: {title!r}')
        if max(budgets) > 2.0:
            raise AssertionError(f'a hook move exceeded the two-second budget: {budgets}')
        fx.control(pid, 'exit')
        tab.wait_claude(lambda s: not s['sessions'], timeout=4, what='exit')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        return (f'hook follow: directory moves and idle/busy/attention/idle plus the sanitized '
                f'rename, slowest {max(budgets):.2f}s within the 2s SSH-09 budget (loopback, '
                f'not the controlled profile); exit removes the session; ' + CLAUDE_BOUNDARY)
    finally:
        close_quietly(tab)


def claude_cd_via_listing(h, fx, workspace):
    """`/cd` fires no hook: a client trigger makes the server run the detected
    claude with exactly `agents --json`, whose listed cwd for the owned pid
    moves the session; an idle session with no trigger runs no listing."""
    tab = ClaudeTab(h, fx, workspace)
    try:
        tab.collect_terminal()
        tab.claude_state()
        pid = tab.type_claude()
        first, second = claude_workdirs(fx, 'cd-a', 'cd-b')
        fx.hook(pid, 'SessionStart', 'cd-1', first)
        tab.wait_claude(lambda s: claude_session_cwd(s, 'cd-1') == first)
        tab.quiet(3.0)
        if 'agents --json' in fx.claude_argv():
            raise AssertionError('a listing ran with no trigger pending')
        fx.set_agents([{'pid': pid, 'cwd': second, 'kind': 'interactive', 'name': 'cd',
                        'prompt': CLAUDE_MARKER}])
        started = time.monotonic()
        tab.trigger()
        tab.wait_claude(lambda s: claude_session_cwd(s, 'cd-1') == second, timeout=4,
                        what='/cd follow')
        elapsed = time.monotonic() - started
        runs = [argv for argv in fx.claude_argv() if argv.startswith('agents')]
        if not runs or set(runs) != {'agents --json'}:
            raise AssertionError(f'listing argv: {runs}')
        if elapsed > 2.0:
            raise AssertionError(f'/cd follow took {elapsed:.2f}s')
        # The next hook record with the same directory leaves it there.
        fx.hook(pid, 'UserPromptSubmit', 'cd-1', second)
        tab.wait_claude(lambda s: s['sessions'][0]['activity'] == 'Busy')
        if claude_session_cwd(tab.last_claude, 'cd-1') != second:
            raise AssertionError('the next hook record moved the session away')
        # Listing failures change nothing: malformed output keeps the hook cwd.
        fx.agents_file.write_text('not json')
        tab.trigger()
        tab.quiet(3.0)
        if claude_session_cwd(tab.last_claude, 'cd-1') != second:
            raise AssertionError('a malformed listing changed the session')
        fx.control(pid, 'exit')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        return (f'/cd via listing: trigger -> exactly `agents --json` on the server -> owned '
                f'session moved in {elapsed:.2f}s; no listing while idle; malformed output '
                f'changed nothing; ' + CLAUDE_BOUNDARY)
    finally:
        close_quietly(tab)


def claude_agent_view_listing(h, fx, workspace):
    """A fresh listing relays the agent-view background entries (normalized
    name and directory only) for the client's title selection."""
    tab = ClaudeTab(h, fx, workspace)
    try:
        tab.collect_terminal()
        tab.claude_state()
        pid = tab.type_claude()
        own, background = claude_workdirs(fx, 'av-own', 'av-bg')
        fx.hook(pid, 'SessionStart', 'av-1', own)
        tab.wait_claude(lambda s: claude_session_cwd(s, 'av-1') == own)
        fx.set_agents([
            {'pid': pid, 'cwd': own, 'kind': 'interactive', 'name': 'own'},
            {'pid': pid + 100000, 'cwd': background, 'kind': 'background',
             'name': '✳ Spec task', 'transcript': CLAUDE_MARKER},
        ])
        tab.trigger()
        listing = tab.wait_listing(lambda l: l.get('background'))
        entries = [(entry['name'], decode_cwd(entry['cwd'])) for entry in listing['background']]
        if entries != [('Spec task', background)]:
            raise AssertionError(f'relayed background entries: {entries}')
        fx.control(pid, 'exit')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        return ('agent-view listing: one background entry relayed as normalized name '
                '`Spec task` plus its directory, nothing else; the title selection is '
                'model-level; ' + CLAUDE_BOUNDARY)
    finally:
        close_quietly(tab)


def claude_two_tab_isolation(h, fx, workspace):
    """Two tabs on one server: each owns only its own process tree's sessions."""
    tab_a = ClaudeTab(h, fx, workspace)
    tab_b = None
    try:
        tab_a.collect_terminal()
        tab_a.claude_state()
        tab_b = ClaudeTab(h, fx, workspace)
        tab_b.collect_terminal()
        tab_b.claude_state()
        pid_a = tab_a.type_claude()
        pid_b = tab_b.type_claude()
        dir_a, dir_b = claude_workdirs(fx, 'tab-a', 'tab-b')
        fx.hook(pid_a, 'SessionStart', 'same-id', dir_a)
        tab_a.wait_claude(lambda s: claude_session_cwd(s, 'same-id') == dir_a)
        fx.hook(pid_b, 'SessionStart', 'same-id', dir_b)
        tab_b.wait_claude(lambda s: claude_session_cwd(s, 'same-id') == dir_b)
        tab_a.quiet(1.5)
        if claude_sessions(tab_a.last_claude) != [('same-id', pid_a, dir_a, 'Idle')]:
            raise AssertionError(f'tab A state crossed tabs: {tab_a.last_claude}')
        if claude_sessions(tab_b.claude_state()) != [('same-id', pid_b, dir_b, 'Idle')]:
            raise AssertionError('tab B state crossed tabs')
        for pid in (pid_a, pid_b):
            fx.control(pid, 'exit')
        if not tab_a.close() or not tab_b.close():
            raise AssertionError('remote cleanup unconfirmed')
        return ('two tabs on one server with the same session id: each owns only its own '
                'ancestry; ' + CLAUDE_BOUNDARY)
    finally:
        close_quietly(tab_a)
        close_quietly(tab_b)


def claude_foreign_and_reparented(h, fx, workspace):
    """A claude reparented out of the tab's tree (as under a tmux server) and a
    claude in a separate SSH login with a copied spool binding write to the
    tab's spool, and neither is claimed."""
    tab = ClaudeTab(h, fx, workspace)
    foreign = None
    try:
        tab.collect_terminal()
        tab.claude_state()
        own = tab.type_claude()
        spool = fx.claude_env(own)['spool']
        fx.control(own, 'exit')
        tab.quiet(0.5)
        # Reparented away from the tab's process tree, like a tmux server child.
        tab.type('(claude > /dev/null 2>&1 &)\n')
        deadline = time.monotonic() + 10
        reparented = None
        while reparented is None:
            pids = [int(line.split('\t')[0][4:]) for line in fx.env_log.read_text().splitlines()
                    if line.startswith('pid=')]
            fresh = [pid for pid in pids if pid != own]
            reparented = fresh[-1] if fresh else None
            if reparented is None and time.monotonic() > deadline:
                raise AssertionError('the reparented claude did not start')
            tab.pump(0.25)
        # A separate SSH login of the same server user with the copied binding.
        foreign = h.ssh_process(fx.claude_prefix(extra={
            'MADE_CLAUDE_SPOOL': spool,
            'CLAUDE_CODE_PLUGIN_DIRS': staged_claude_dir(fx)}) + ' claude')
        foreign_pid = int(re.search(rb'MADE_FIXTURE_CLAUDE:([0-9]+)',
                                    foreign.stdout.readline()).group(1))
        dirs = claude_workdirs(fx, 'reparented', 'foreign')
        fx.hook(reparented, 'SessionStart', 'reparented-1', dirs[0])
        fx.hook(foreign_pid, 'SessionStart', 'foreign-1', dirs[1])
        events = tab.quiet(3.0)
        if any(claude_sessions(event) for event in events) or claude_sessions(tab.claude_state()):
            raise AssertionError(f'a foreign or reparented session was claimed: {events}')
        for pid in (reparented, foreign_pid):
            fx.control(pid, 'exit')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        return ('a reparented claude (as under tmux) and a separate-login claude with the '
                'copied spool binding were claimed by no tab; ' + CLAUDE_BOUNDARY)
    finally:
        close_quietly(tab)
        if foreign is not None and foreign.poll() is None:
            foreign.kill()
            foreign.wait(timeout=5)


def claude_loss_and_reattach(h, fx, workspace, outage=60.0):
    """A 60 s transport loss: Claude Code keeps running, its outage records
    apply on the server, and the reattached client receives one fresh snapshot
    (revision 1, after the resnapshot) with the current directory and status;
    no outage record replays individually."""
    tab = ClaudeTab(h, fx, workspace)
    fresh = None
    try:
        tab.collect_terminal()
        tab.claude_state()
        pid = tab.type_claude()
        before, during = claude_workdirs(fx, 'loss-before', 'loss-during')
        fx.hook(pid, 'SessionStart', 'loss-1', before)
        tab.wait_claude(lambda s: claude_session_cwd(s, 'loss-1') == before)
        epoch = tab.identity['epoch']
        tab.kill_transport()
        fx.hook(pid, 'UserPromptSubmit', 'loss-1', before)
        fx.hook(pid, 'CwdChanged', 'loss-1', during)
        time.sleep(outage)
        fresh = ClaudeTab(h, fx, workspace, identity=tab.identity, epoch=epoch + 1,
                          mode='Attach')
        if fresh.error is not None:
            raise AssertionError(f'the reattach failed: {fresh.error}')
        snapshot = fresh.claude_state(timeout=15)
        if snapshot['revision'] != 1:
            raise AssertionError(f'the reattach did not start a fresh revision: {snapshot}')
        if claude_sessions(snapshot) != [('loss-1', pid, during, 'Busy')]:
            raise AssertionError(f'the fresh snapshot is not the current state: {snapshot}')
        if 'TerminalOutput' not in fresh.stream_ops:
            raise AssertionError('the snapshot arrived before any resnapshot frame')
        replays = [event for event in fresh.quiet(1.5)
                   if claude_session_cwd(event, 'loss-1') == before]
        if replays:
            raise AssertionError(f'an outage record replayed individually: {replays}')
        if not pid_alive(h, pid):
            raise AssertionError('Claude Code did not survive the outage')
        fx.control(pid, 'exit')
        if not fresh.close():
            raise AssertionError('remote cleanup unconfirmed')
        fresh.opened = False
        tab.opened = False
        return (f'a {outage:.0f}s transport loss: the same claude (pid {pid}) kept running, the '
                f'reattached client got one fresh revision-1 snapshot with the outage '
                f'directory and busy status after the resnapshot, no replay; ' + CLAUDE_BOUNDARY)
    finally:
        close_quietly(tab)
        close_quietly(fresh)


def claude_older_supervisor_and_stage(h, fx, workspace):
    """Older companion artifacts without the package: shell and claude still
    start, no entry points at the missing package and the tab reports hooks
    unavailable. A client that never sends ClaudeState (as an older client
    attached to this supervisor) receives no Claude Code frame."""
    shutil.rmtree(fx.companion_dir / 'plugins' / 'claude-code-made', ignore_errors=True)
    tab = ClaudeTab(h, fx, workspace)
    quiet = None
    try:
        tab.collect_terminal()
        if tab.claude_state()['hooks_available']:
            raise AssertionError('hooks reported available without the staged package')
        pid = tab.type_claude()
        if fx.claude_env(pid)['dirs'] != '<unset>':
            raise AssertionError('a plugin entry was added without the staged package')
        fx.control(pid, 'exit')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        fx.stage_claude_package()
        quiet = ClaudeTab(h, fx, workspace)
        quiet.collect_terminal()
        pid = quiet.type_claude()
        (work,) = claude_workdirs(fx, 'quiet')
        fx.hook(pid, 'SessionStart', 'quiet-1', work)
        quiet.quiet(3.0)
        claude_ops = [op for op in quiet.stream_ops if op.startswith('Claude')]
        if claude_ops:
            raise AssertionError(f'a client that never asked received {claude_ops}')
        fx.control(pid, 'exit')
        if not quiet.close():
            raise AssertionError('remote cleanup unconfirmed')
        return ('companion artifacts without claude-code-made: shell and claude start with no '
                'plugin entry and hooks unavailable; a client that never sends ClaudeState '
                'receives no Claude Code frame; the client-side helper_version gate is '
                'model-level; an older supervisor binary was not available here; '
                + CLAUDE_BOUNDARY)
    finally:
        close_quietly(tab)
        close_quietly(quiet)
        fx.stage_claude_package()


def claude_privacy_and_home(h, fx, workspace):
    """Prompt, message, tool and transcript markers reach no relay frame and
    no app-owned spool, and the remote user's `~/.claude` stays byte-identical."""
    fx.seed_claude_home()
    before = tree_fingerprint(fx.claude_home)
    tab = ClaudeTab(h, fx, workspace)
    try:
        tab.collect_terminal()
        tab.claude_state()
        pid = tab.type_claude()
        spool = pathlib.Path(fx.claude_env(pid)['spool'])
        (work,) = claude_workdirs(fx, 'privacy')
        for event in ('SessionStart', 'UserPromptSubmit', 'PermissionRequest', 'Notification',
                      'Stop'):
            fx.hook(pid, event, 'p-1', work, notification='permission_prompt')
        fx.set_agents([{'pid': pid, 'cwd': work, 'kind': 'interactive',
                        'prompt': CLAUDE_MARKER}])
        tab.trigger()
        tab.wait_claude(lambda s: claude_session_cwd(s, 'p-1') == work)
        tab.quiet(3.0)
        spool_bytes = spool.read_bytes() if spool.exists() else b''
        if not spool_bytes:
            raise AssertionError('the spool recorded nothing to search')
        relay = [frame for frame in tab.raw if '"Claude' in frame]
        if not relay:
            raise AssertionError('no Claude Code relay frame to search')
        for name, text in (('spool', spool_bytes.decode('utf-8', 'replace')),
                           ('relay', '\n'.join(relay))):
            if CLAUDE_MARKER in text:
                raise AssertionError(f'the privacy marker reached the {name}')
        fx.control(pid, 'exit')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        if tree_fingerprint(fx.claude_home) != before:
            raise AssertionError('the remote user\'s ~/.claude changed')
        return (f'markers in prompt, message, tool and transcript fields reached neither the '
                f'{len(spool_bytes)}-byte spool nor {len(relay)} relay frames; ~/.claude stayed '
                f'byte-identical (that it was not read is the file-access audit of '
                f'crates/made-remote/tests/claude.rs); client logs/diagnostics are model-level; '
                + CLAUDE_BOUNDARY)
    finally:
        close_quietly(tab)


def claude_undetected(h, fx, workspace):
    """claude off the remote PATH the detection sees: no Claude Code launch,
    a claude started by absolute path in the shell still loads the staged
    plugin and follows hooks, and a trigger runs no listing."""
    tab = ClaudeTab(h, fx, workspace, claude=False)
    direct = None
    try:
        tab.collect_terminal()
        tab.claude_state()
        pid = tab.type_claude(shlex.quote(str(fx.claude_scripts['claude'])))
        if fx.claude_env(pid)['dirs'] != staged_claude_dir(fx):
            raise AssertionError('the shell-typed claude did not get the staged entry')
        (work,) = claude_workdirs(fx, 'undetected')
        fx.hook(pid, 'SessionStart', 'u-1', work)
        tab.wait_claude(lambda s: claude_session_cwd(s, 'u-1') == work)
        tab.trigger()
        tab.quiet(3.0)
        if any(argv.startswith('agents') for argv in fx.claude_argv()):
            raise AssertionError('a listing ran although claude was not detected')
        fx.control(pid, 'exit')
        if not tab.close():
            raise AssertionError('remote cleanup unconfirmed')
        direct = ClaudeTab(h, fx, workspace, target='Claude', claude=False,
                           attach_timeout=8)
        if direct.error is None:
            raise AssertionError('Open/Claude started without a detected claude')
        return ('claude undetected: Open/Claude refused, a shell-typed claude still loads the '
                'staged plugin and follows hooks, no listing runs; ' + CLAUDE_BOUNDARY)
    finally:
        close_quietly(tab)
        close_quietly(direct)


CLAUDE_SCENARIOS = (
    ('direct-launch', claude_direct_launch, True),
    ('protocol-only-bare', claude_protocol_only_bare, True),
    ('shell-typed-plugin-dirs', claude_shell_typed_plugin_dirs, True),
    ('hook-follow', claude_hook_follow, True),
    ('cd-via-listing', claude_cd_via_listing, True),
    ('agent-view-listing', claude_agent_view_listing, True),
    ('two-tab-isolation', claude_two_tab_isolation, True),
    ('foreign-and-reparented-not-claimed', claude_foreign_and_reparented, True),
    ('older-supervisor-and-stage', claude_older_supervisor_and_stage, True),
    ('privacy-and-claude-home', claude_privacy_and_home, True),
    ('claude-undetected', claude_undetected, True),
    ('loss-and-reattach', claude_loss_and_reattach, True),
)


def run_claude_scenarios(harness, fixture, workspace, only):
    results = []
    fixture.reset_stage()
    fixture.stage_claude_package()
    for name, scenario, needs_session in CLAUDE_SCENARIOS:
        if only and name not in only:
            continue
        fixture.reset_claude_logs()
        shutil.rmtree(fixture.claude_home, ignore_errors=True)
        if needs_session:
            reason = supervisor_unavailable(harness, fixture, workspace)
            if reason is not None:
                print(f'BLOCKED claude/{name}: scenario unrun — {reason.splitlines()[0]}')
                results.append((name, 'blocked', reason))
                continue
        try:
            evidence = scenario(harness, fixture, workspace)
            print(f'PASS claude/{name} — {evidence}')
            results.append((name, 'pass', evidence))
        except SupervisorPreflightError as exc:
            print(f'BLOCKED claude/{name}: {exc}')
            results.append((name, 'blocked', str(exc)))
        except (AssertionError, TimeoutError, EOFError, ValueError,
                subprocess.SubprocessError, OSError) as exc:
            print(f'FAIL claude/{name}: {type(exc).__name__}: {exc}', file=sys.stderr)
            results.append((name, 'fail', str(exc)))
    counts = {}
    for _, status, _ in results:
        counts[status] = counts.get(status, 0) + 1
    tally = ', '.join(f'{key}={value}' for key, value in sorted(counts.items())) or 'none ran'
    print(f'SUMMARY claude/claude-parity: {tally}')
    return 0 if counts.get('pass') and not counts.get('fail') else 2


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=(
        'inspect', 'ssh-smoke', 'capabilities', 'survival', 'container-survival',
        'grok-parity', 'grok-first-fail', 'claude-parity'))
    parser.add_argument('--frontend', help='explicit companion remote frontend/bootstrap command')
    parser.add_argument('--companion', help='Linux aarch64 companion binary to upload')
    parser.add_argument('--fingerprint', help='out-of-band verified SHA256 host-key fingerprint')
    parser.add_argument('--host-companion',
                        help='locally built companion binary for the loopback Grok scenarios')
    parser.add_argument('--scenario',
                        help='comma-separated subset of the grok or claude scenarios')
    args = parser.parse_args()
    if args.mode == 'inspect':
        print(json.dumps({'platform': platform.platform(), 'machine': platform.machine(),
                          'tools': tools(), 'evidence': 'inspection only; tmux is not a prerequisite'},
                         indent=2))
        return 0
    if args.mode == 'container-survival':
        if not args.fingerprint:
            raise PrerequisiteError('--fingerprint required')
        return container_survival(args.fingerprint, args.companion)
    if args.mode in ('grok-parity', 'grok-first-fail'):
        companion = resolve_host_companion(args.host_companion)
        only = ({name.strip() for name in args.scenario.split(',')}
                if args.scenario else None)
        with Harness() as harness:
            workspace = fixtures(harness.root)
            fixture = GrokFixture(harness.root, companion)
            print(f'command: loopback grok parity with companion {companion}')
            return run_grok_scenarios(harness, fixture, workspace, args.mode, only)
    if args.mode == 'claude-parity':
        companion = resolve_host_companion(args.host_companion)
        only = ({name.strip() for name in args.scenario.split(',')}
                if args.scenario else None)
        with Harness() as harness:
            workspace = fixtures(harness.root)
            fixture = ClaudeFixture(harness.root, companion)
            print(f'command: loopback claude parity with companion {companion}')
            return run_claude_scenarios(harness, fixture, workspace, only)
    if args.mode in ('capabilities', 'survival') and not args.frontend:
        raise PrerequisiteError('--frontend required: companion CLI/bootstrap is not frozen')
    with Harness() as harness:
        workspace = fixtures(harness.root)
        print(f'PASS isolated loopback SSH and fixtures (workspace {workspace.name}); '
              'not evidence of product, OpenCode or native UI parity')
        if args.mode == 'capabilities':
            capabilities(harness, args.frontend)
        elif args.mode == 'survival':
            survival(harness, args.frontend, workspace)
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (PrerequisiteError, subprocess.SubprocessError, OSError, ValueError,
            EOFError, TimeoutError, AssertionError) as exc:
        print(f'BLOCKED/FAIL: {exc}', file=sys.stderr)
        sys.exit(2)
