#!/usr/bin/env python3
"""Real loopback SSH probe and protocol checkpoint, not native GUI certification.

    --frontend is the explicit remote command implementing the bootstrap; no executable is guessed.
It must consume v1 frames on stdin and emit only v1 frames on stdout.
"""
import argparse
import base64
import getpass
import json
import os
import platform
import re
import select
import selectors
import shlex
import struct
import subprocess
import sys
import time

from harness import (
    ContainerSession, Harness, PrerequisiteError, cargo_lock_busy, confirm_authorized_host,
    cross_build_blockage, fixtures, linux_arm64_elf, matching_host_keys, tools,
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


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=(
        'inspect', 'ssh-smoke', 'capabilities', 'survival', 'container-survival'))
    parser.add_argument('--frontend', help='explicit companion remote frontend/bootstrap command')
    parser.add_argument('--companion', help='Linux aarch64 companion binary to upload')
    parser.add_argument('--fingerprint', help='out-of-band verified SHA256 host-key fingerprint')
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
