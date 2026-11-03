import os
import pathlib
import struct
import tempfile
import unittest

from harness import (
    PREREQUISITES, cross_build_blockage, linux_arm64_elf, remote_proof_directory, tools,
    wipe_secret,
)
from parity import LIMIT, encode, envelope, receive


class FramingTests(unittest.TestCase):
    def test_envelope_roundtrip(self):
        frame = envelope({'authority': [1] * 16, 'tab': [2] * 32,
                          'epoch': 1, 'generation': 1}, 1, 'Capabilities')
        r, w = os.pipe()
        try:
            os.write(w, encode(frame))
            self.assertEqual(receive(r), frame)
        finally:
            os.close(r)
            os.close(w)

    def test_oversized_prefix_is_rejected_before_reading_body(self):
        r, w = os.pipe()
        try:
            os.write(w, struct.pack('>I', LIMIT + 1))
            with self.assertRaises(ValueError):
                receive(r)
        finally:
            os.close(r)
            os.close(w)


def elf(interp, extra=b''):
    interp_bytes = interp + b'\0'
    phoff = 64
    phentsize = 56
    interp_off = phoff + phentsize
    data = bytearray(interp_off + len(interp_bytes) + len(extra))
    data[0:4] = b'\x7fELF'
    data[4] = 2
    data[5] = 1
    data[18:20] = (183).to_bytes(2, 'little')
    data[32:40] = phoff.to_bytes(8, 'little')
    data[52:54] = (64).to_bytes(2, 'little')
    data[54:56] = phentsize.to_bytes(2, 'little')
    data[56:58] = (1).to_bytes(2, 'little')
    data[phoff:phoff + 4] = (1).to_bytes(4, 'little')
    data[phoff + 8:phoff + 16] = interp_off.to_bytes(8, 'little')
    data[phoff + 32:phoff + 40] = len(interp_bytes).to_bytes(8, 'little')
    data[interp_off:interp_off + len(interp_bytes)] = interp_bytes
    data[interp_off + len(interp_bytes):] = extra
    return bytes(data)


class HarnessContractTests(unittest.TestCase):
    def test_prerequisites_exclude_tmux(self):
        self.assertNotIn('tmux', PREREQUISITES)
        self.assertNotIn('tmux', tools())

    def test_remote_proof_directory_rejects_traversal(self):
        self.assertEqual(remote_proof_directory('/tmp/made-proof.abc123'), '/tmp/made-proof.abc123')
        for path in ('/tmp/made-proof.abc/../etc', '/home/vscode', '/tmp/other', ''):
            with self.assertRaises(ValueError):
                remote_proof_directory(path)

    def test_linux_arm64_elf_accepts_glibc_and_rejects_substitutes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            good = root / 'good'
            good.write_bytes(elf(b'/lib/ld-linux-aarch64.so.1', b'GLIBC_2.31'))
            self.assertIsNone(linux_arm64_elf(good))
            musl = root / 'musl'
            musl.write_bytes(elf(b'/lib/ld-musl-aarch64.so.1'))
            self.assertIn('musl', linux_arm64_elf(musl))
            newer = root / 'newer'
            newer.write_bytes(elf(b'/lib/ld-linux-aarch64.so.1', b'GLIBC_2.40'))
            self.assertIn('2.40', linux_arm64_elf(newer))
            macho = root / 'macho'
            macho.write_bytes(b'\xcf\xfa\xed\xfe' + b'\0' * 60)
            self.assertIn('Mach-O', linux_arm64_elf(macho))

    def test_host_companion_is_not_a_linux_binary(self):
        path = pathlib.Path(__file__).resolve().parents[2] / 'target' / 'debug' / 'made-remote'
        if not path.is_file():
            self.skipTest('host companion absent')
        self.assertIn('Mach-O', linux_arm64_elf(path))

    def test_cross_build_status_does_not_install(self):
        status = cross_build_blockage()
        self.assertIn('did not install', status['refused'])
        self.assertIsInstance(status['blocked'], bool)
        self.assertIsInstance(status['missing'], list)

    def test_wipe_secret_removes_the_file(self):
        with tempfile.TemporaryDirectory() as directory:
            secret = pathlib.Path(directory) / 'askpass.secret'
            secret.write_text('not-a-real-password\n')
            wipe_secret(secret)
            self.assertFalse(secret.exists())


if __name__ == '__main__':
    unittest.main()
