#!/usr/bin/env python3
"""Read-only inspection of the explicitly authorized localhost:2222 fixture.

Verify the expected host fingerprint out-of-band before use. Password is read
from the invoking terminal, passed to ssh through its PTY, and never persisted.
"""
import argparse
import base64
import getpass
import hashlib
import os
import pathlib
import pty
import select
import shlex
import subprocess
import sys
import tempfile
import termios
import time

from harness import BASE

REMOTE_SCRIPT = r'''set -u
printf 'kernel: '; uname -srm
printf 'identity: '; id
printf 'shell: %s\n' "$SHELL"
printf 'working directory: '; pwd
printf 'home metadata: '; ls -ld "$HOME"
for p in "$HOME/.local" "$HOME/.local/share" "$HOME/.config" /workspaces; do
  if [ -e "$p" ]; then ls -ld "$p"; else printf 'absent: %s\n' "$p"; fi
done
for t in sh bash zsh fish opencode git rustc cargo rustup node npm cc pkg-config; do
  p=$(command -v "$t" 2>/dev/null || :)
  if [ -n "$p" ]; then
    printf '%s: %s\n' "$t" "$p"
    case "$t" in
      opencode|git|rustc|cargo|node|npm|cc|pkg-config)
        env HOME=/nonexistent XDG_CONFIG_HOME=/nonexistent XDG_DATA_HOME=/nonexistent XDG_CACHE_HOME=/nonexistent "$p" --version 2>/dev/null | head -n 1 || : ;;
    esac
  else printf '%s: unavailable\n' "$t"; fi
done
if command -v rustup >/dev/null 2>&1; then rustup target list --installed 2>/dev/null || :; fi
if command -v ldd >/dev/null 2>&1; then ldd --version 2>&1 | head -n 1; fi
printf 'additional executable locations (metadata only):\n'
for p in "$HOME/.cargo/bin/rustc" "$HOME/.cargo/bin/cargo" "$HOME/.cargo/bin/rustup" "$HOME/.opencode/bin/opencode" "$HOME/.local/bin/opencode" /lib/ld-musl-*; do
  if [ -e "$p" ]; then ls -ld "$p"; fi
done
printf 'checksum capability: '; command -v sha256sum || command -v shasum || :
printf 'temporary directory: '; ls -ld /tmp
printf 'private app-root candidate writable: '
if [ -w "$HOME/.local/share" ]; then printf 'yes\n'; else printf 'no\n'; fi
printf 'workspace candidates (metadata only):\n'
for p in /workspaces/* "$HOME"/workspace "$HOME"/project; do
  if [ -d "$p" ]; then ls -ld "$p"; [ ! -d "$p/.git" ] || printf 'git repository: %s\n' "$p"; fi
done
'''


def fingerprint(key):
    blob = base64.b64decode(key, validate=True)
    return 'SHA256:' + base64.b64encode(hashlib.sha256(blob).digest()).decode().rstrip('=')


def inspect(expected):
    if not BASE.is_dir():
        raise RuntimeError('approved temporary base is unavailable')
    scan = subprocess.run(['ssh-keyscan', '-T', '5', '-p', '2222', '127.0.0.1'],
                          stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                          stderr=subprocess.DEVNULL, timeout=10, check=True)
    matches = []
    for line in scan.stdout.decode('ascii').splitlines():
        parts = line.split()
        if len(parts) == 3 and fingerprint(parts[2]) == expected:
            matches.append(line)
    if not matches:
        raise RuntimeError('host key does not match the explicitly supplied fingerprint')
    password = getpass.getpass('Fixture password (not stored): ')
    with tempfile.TemporaryDirectory(prefix='made-container-inspect-', dir=BASE) as directory:
        os.chmod(directory, 0o700)
        known = pathlib.Path(directory) / 'known_hosts'
        known.write_text('\n'.join(matches) + '\n')
        known.chmod(0o600)
        master, slave = pty.openpty()
        attrs = termios.tcgetattr(slave)
        attrs[3] &= ~termios.ECHO
        termios.tcsetattr(slave, termios.TCSANOW, attrs)
        # ssh reads its password from /dev/tty; setsid + TIOCSCTTY gives it the
        # private slave without putting credentials in argv, env or a file.
        def child_setup():
            os.setsid()
            import fcntl
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        args = ['ssh', '-F', '/dev/null', '-T', '-p', '2222',
                '-o', 'IdentityAgent=none', '-o', 'IdentitiesOnly=yes',
                '-o', 'PubkeyAuthentication=no', '-o', 'PreferredAuthentications=password,keyboard-interactive',
                '-o', 'NumberOfPasswordPrompts=1', '-o', 'ConnectTimeout=5',
                '-o', f'UserKnownHostsFile={known}', '-o', 'GlobalKnownHostsFile=/dev/null',
                '-o', 'StrictHostKeyChecking=yes', '-o', 'ForwardAgent=no',
                '-o', 'ClearAllForwardings=yes', '-o', 'PermitLocalCommand=no',
                'vscode@127.0.0.1', 'sh -c ' + shlex.quote(REMOTE_SCRIPT)]
        try:
            child = subprocess.Popen(args, stdin=slave, stdout=slave, stderr=slave,
                                     preexec_fn=child_setup, close_fds=True)
        finally:
            os.close(slave)
        output = bytearray()
        sent = False
        deadline = time.monotonic() + 30
        try:
            while time.monotonic() < deadline:
                readable, _, _ = select.select([master], [], [], max(0, deadline - time.monotonic()))
                if not readable:
                    break
                try:
                    data = os.read(master, 4096)
                except OSError:
                    break
                if not data:
                    break
                output.extend(data)
                if not sent and b'password:' in output.lower():
                    os.write(master, password.encode() + b'\n')
                    sent = True
            try:
                child.wait(timeout=2)
            except subprocess.TimeoutExpired:
                child.terminate()
                child.wait(timeout=5)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            os.close(master)
        text = output.decode(errors='replace').replace(password, '<redacted>') if password else ''
        print(text, end='')
        if not sent or child.returncode != 0:
            raise RuntimeError(f'SSH inspection failed (exit {child.returncode}, password prompt observed: {sent})')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--fingerprint', required=True, help='out-of-band verified SHA256 host-key fingerprint')
    try:
        inspect(parser.parse_args().fingerprint)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as exc:
        print(f'BLOCKED: {exc}', file=sys.stderr)
        sys.exit(2)
