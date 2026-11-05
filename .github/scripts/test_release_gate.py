#!/usr/bin/env python3
"""Tests for release-gate.py with a fake GitHub API (task 7.4)."""

from __future__ import annotations

import importlib.util
import io
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
spec = importlib.util.spec_from_file_location("release_gate", HERE / "release-gate.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)
fixture_spec = importlib.util.spec_from_file_location("test_check_versions", HERE / "test_check_versions.py")
fixtures = importlib.util.module_from_spec(fixture_spec)
fixture_spec.loader.exec_module(fixtures)


class FakeGitHub:
    def __init__(self, releases: list[dict]) -> None:
        self._releases = releases
        self.deleted: list[int] = []

    def releases(self) -> list[dict]:
        return [r for r in self._releases if r["id"] not in self.deleted]

    def delete_release(self, release_id: int) -> None:
        self.deleted.append(release_id)


def release(release_id: int, tag: str, draft: bool = False, prerelease: bool = False) -> dict:
    return {"id": release_id, "tag_name": tag, "draft": draft, "prerelease": prerelease}


class ReleaseGateTest(unittest.TestCase):
    def setUp(self) -> None:
        self._dir = tempfile.TemporaryDirectory()
        self.root = Path(self._dir.name)

    def tearDown(self) -> None:
        self._dir.cleanup()

    def run_gate(self, tag: str, github: FakeGitHub, *extra: str) -> tuple[int, str]:
        err, out = io.StringIO(), io.StringIO()
        with redirect_stderr(err), redirect_stdout(out):
            code = gate.main(["--tag", tag, *extra], github=github, root=self.root)
        return code, err.getvalue() + out.getvalue()

    def test_a_newer_stable_tag_with_agreeing_versions_passes(self) -> None:
        fixtures.write_fixture(self.root, version="0.3.0")
        github = FakeGitHub([release(1, "v0.2.0"), release(2, "v0.9.0", prerelease=True)])
        code, text = self.run_gate("v0.3.0", github)
        self.assertEqual(code, 0, text)
        self.assertEqual(self.run_gate("v0.1.0", FakeGitHub([]))[0], 1, "workspace is 0.3.0")

    def test_first_release_passes_with_no_published_release(self) -> None:
        fixtures.write_fixture(self.root, version="0.1.0")
        self.assertEqual(self.run_gate("v0.1.0", FakeGitHub([]))[0], 0)

    def test_prerelease_tags_are_refused(self) -> None:
        fixtures.write_fixture(self.root, version="0.3.0-beta.1")
        code, text = self.run_gate("v0.3.0-beta.1", FakeGitHub([]))
        self.assertEqual(code, 1)
        self.assertIn("prerelease", text)

    def test_versions_not_greater_than_the_newest_stable_release_are_refused(self) -> None:
        for tag, version in (("v0.2.1", "0.2.1"), ("v0.3.0", "0.3.0")):
            with self.subTest(tag=tag), tempfile.TemporaryDirectory() as raw:
                root = Path(raw)
                fixtures.write_fixture(root, version=version)
                github = FakeGitHub([release(1, "v0.3.0"), release(2, "v0.2.0")])
                err = io.StringIO()
                with redirect_stderr(err), redirect_stdout(io.StringIO()):
                    code = gate.main(["--tag", tag], github=github, root=root)
                self.assertEqual(code, 1)
                self.assertIn("v0.3.0", err.getvalue())

    def test_an_already_published_tag_is_refused(self) -> None:
        fixtures.write_fixture(self.root, version="0.2.0")
        code, text = self.run_gate("v0.2.0", FakeGitHub([release(1, "v0.2.0")]))
        self.assertEqual(code, 1)
        self.assertIn("already published", text)

    def test_any_version_site_disagreement_is_refused(self) -> None:
        fixtures.write_fixture(self.root, version="0.2.0", package="0.1.0")
        code, text = self.run_gate("v0.2.0", FakeGitHub([]))
        self.assertEqual(code, 1)
        self.assertIn("plugins/opencode-made/package.json", text)
        with tempfile.TemporaryDirectory() as raw:
            fixtures.write_fixture(Path(raw), version="0.1.0")
            err = io.StringIO()
            with redirect_stderr(err), redirect_stdout(io.StringIO()):
                code = gate.main(["--tag", "v0.2.0"], github=FakeGitHub([]), root=Path(raw))
            self.assertEqual(code, 1)
            self.assertIn("0.2.0", err.getvalue())
            self.assertIn("0.1.0", err.getvalue())

    def test_a_draft_left_by_a_failed_run_does_not_block_and_is_deleted(self) -> None:
        fixtures.write_fixture(self.root, version="0.2.0")
        github = FakeGitHub([release(7, "v0.2.0", draft=True), release(8, "v0.1.0"), release(9, "v0.1.5", draft=True)])
        self.assertEqual(self.run_gate("v0.2.0", github)[0], 0, "an unpublished draft is not a published release")
        code, text = self.run_gate("v0.2.0", github, "--delete-drafts")
        self.assertEqual(code, 0)
        self.assertEqual(github.deleted, [7], "only the draft for this tag is removed")
        self.assertIn("7", text)

    def test_published_releases_are_never_deleted(self) -> None:
        github = FakeGitHub([release(1, "v0.2.0")])
        self.run_gate("v0.2.0", github, "--delete-drafts")
        self.assertEqual(github.deleted, [])


if __name__ == "__main__":
    unittest.main()
