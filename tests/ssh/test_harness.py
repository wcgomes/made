import os
import pathlib
import signal
import struct
import tempfile
import unittest
from unittest import mock

from harness import (
    Harness, PREREQUISITES, cleanup_owned_companions, cross_build_blockage,
    linux_arm64_elf, owned_companion_pids, remote_proof_directory, tools, wipe_secret,
)
import parity
from parity import LIMIT, diagnose_supervisor_failure, encode, envelope, receive


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


class DiagnosisTextTests(unittest.TestCase):
    def diagnose(self, stderr='', error=None, listing=b''):
        h = mock.Mock()
        h.ssh_run.return_value = mock.Mock(stdout=listing)
        identity = {'tab': [1] * 32}
        return diagnose_supervisor_failure(
            h, identity, error or RuntimeError('supervisor exited'), stderr)

    def test_reports_captured_evidence_and_acknowledges_temp_fallback(self):
        text = self.diagnose('made-remote: operation failed',
                             RuntimeError('supervisor exited'),
                             b'.\n..\nendpoint.json\n')
        self.assertIn('made-remote: operation failed', text)
        self.assertIn('RuntimeError: supervisor exited', text)
        self.assertIn('endpoint published', text)
        self.assertIn('103-byte', text)
        self.assertIn('short OS temp root', text)
        self.assertNotIn('product defect', text)
        self.assertNotIn('cannot start on a macOS remote', text)
        self.assertNotIn('fix belongs in made-platform', text)
        self.assertNotIn('Known cause matched', text)

    def test_long_path_guidance_only_when_observed_error_matches(self):
        matched = self.diagnose('bind: File name too long',
                                OSError('File name too long'))
        self.assertIn('Known cause matched', matched)
        unmatched = self.diagnose('bind: Permission denied',
                                  PermissionError('Permission denied'))
        self.assertNotIn('Known cause matched', unmatched)


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


class CompanionCleanupTests(unittest.TestCase):
    def harness_double(self):
        harness = object.__new__(Harness)
        harness.temp = tempfile.TemporaryDirectory()
        self.addCleanup(harness.temp.cleanup)
        harness.root = pathlib.Path(harness.temp.name)
        harness.processes = []
        return harness

    def test_ownership_requires_the_exact_instance_executable_and_mode(self):
        root = pathlib.Path('/tmp/made-ssh-current')
        binary = str(root / 'companion' / 'made-remote')
        other = '/tmp/made-ssh-other/companion/made-remote'
        rows = [
            (10, [binary, '--stdio']),
            (11, [binary, '--supervisor']),
            (12, [other, '--stdio']),
            (13, ['/usr/bin/made-remote', '--stdio']),
            (14, [binary + '.old', '--stdio']),
            (15, [binary, '--child']),
            (16, [binary, '--stdio', 'extra']),
            (17, ['/bin/sleep', '30']),
        ]
        with mock.patch('harness.process_table', return_value=rows):
            self.assertEqual(owned_companion_pids(root), {10, 11})

    def test_term_is_followed_by_kill_only_for_survivors(self):
        root = pathlib.Path('/tmp/made-ssh-current')
        with mock.patch('harness.owned_companion_pids',
                        side_effect=[{10, 11}, {11}, set()]), \
                mock.patch('harness.os.kill') as kill:
            cleanup_owned_companions(root, term_timeout=0, kill_timeout=0)
        self.assertEqual(kill.call_args_list, [
            mock.call(10, signal.SIGTERM),
            mock.call(11, signal.SIGTERM),
            mock.call(11, signal.SIGKILL),
        ])

    def test_success_teardown_cleans_companions_and_removes_temp_root(self):
        harness = self.harness_double()
        root = harness.root
        with mock.patch('harness.cleanup_owned_companions') as cleanup:
            harness.__exit__()
        cleanup.assert_called_once_with(root)
        self.assertFalse(root.exists())

    def test_exception_and_intended_failure_teardown_remove_temp_root(self):
        for exception_type in (RuntimeError, AssertionError):
            with self.subTest(exception=exception_type.__name__):
                harness = self.harness_double()
                root = harness.root
                with mock.patch('harness.cleanup_owned_companions') as cleanup:
                    with self.assertRaises(exception_type):
                        try:
                            raise exception_type('scenario failure')
                        finally:
                            harness.__exit__()
                cleanup.assert_called_once_with(root)
                self.assertFalse(root.exists())

    def test_temp_root_survives_failed_companion_verification(self):
        harness = self.harness_double()
        root = harness.root
        with mock.patch('harness.cleanup_owned_companions',
                        side_effect=RuntimeError('still running')):
            with self.assertRaisesRegex(RuntimeError, 'still running'):
                harness.__exit__()
        self.assertTrue(root.exists())


class GrokNoInterpreterTests(unittest.TestCase):
    """No interpreter is a product prerequisite: the python3 scenarios and PATH
    variants are gone and the fixtures default to a remote PATH without one."""

    def test_scenario_table_has_no_python3_missing_variant(self):
        names = [name for name, _, _ in parity.GROK_SCENARIOS]
        self.assertNotIn('install-python3-missing', names)
        self.assertIn('install-without-python3', names)
        self.assertIn('native-hook-without-python3', names)

    def test_default_remote_path_holds_no_python3(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            companion = root / 'made-remote'
            companion.write_text('fixture')
            fixture = parity.GrokFixture(root, companion)
            without = fixture.prefix(grok=True)
            self.assertNotIn(str(fixture.scripts['pybin']), without)
            if parity.platform.system() == 'Linux':
                shadow = pathlib.Path(fixture.coreutils_path())
                self.assertFalse(any(entry.name.startswith('python3')
                                     for entry in shadow.iterdir()))
            with_python = fixture.prefix(grok=True, python3=True)
            self.assertIn(str(fixture.scripts['pybin']), with_python)

    def test_staged_set_names_the_native_hook_not_a_script(self):
        names = parity.current_stage_artifact_names()
        self.assertEqual(len(names), 10)
        self.assertIn(parity.NATIVE_HOOK_ARTIFACT, names)
        self.assertFalse([name for name in names if name.endswith('.py')])


if __name__ == '__main__':
    unittest.main()
