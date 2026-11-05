#!/usr/bin/env python3
"""Structure tests for the release workflows (add-release-auto-update task 7.6)."""

from __future__ import annotations

import re
import unittest
from pathlib import Path

import yaml

WORKFLOWS = Path(__file__).resolve().parents[1] / "workflows"
PINNED = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")


def load(name: str) -> dict:
    data = yaml.safe_load((WORKFLOWS / name).read_text(encoding="utf-8"))
    # PyYAML reads the bare `on` key as boolean true.
    if True in data:
        data["on"] = data.pop(True)
    return data


def steps(job: dict) -> list[dict]:
    return job.get("steps", [])


def uses_of(workflow: dict) -> list[str]:
    found = []
    for job in workflow["jobs"].values():
        if "uses" in job:
            found.append(job["uses"])
        found.extend(step["uses"] for step in steps(job) if "uses" in step)
    return found


def needs(job: dict) -> set[str]:
    value = job.get("needs", [])
    return {value} if isinstance(value, str) else set(value)


class ReleaseWorkflowTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.release = load("release.yml")
        cls.upgrade = load("upgrade-check.yml")
        cls.companions = load("ci-ssh-remote.yml")
        cls.release_text = (WORKFLOWS / "release.yml").read_text(encoding="utf-8")

    def test_only_v_tags_trigger_a_release(self) -> None:
        self.assertEqual(self.release["on"], {"push": {"tags": ["v*"]}})

    def test_job_order_is_gate_build_smoke_publish_upgrade_check(self) -> None:
        jobs = self.release["jobs"]
        self.assertIn("gate", needs(jobs["build"]))
        self.assertIn("gate", needs(jobs["companions"]))
        self.assertEqual(needs(jobs["smoke"]), {"build"})
        self.assertEqual(needs(jobs["publish"]), {"gate", "build", "smoke"})
        self.assertEqual(needs(jobs["upgrade-check"]), {"publish"})
        self.assertEqual(jobs["upgrade-check"]["uses"], "./.github/workflows/upgrade-check.yml")

    def test_publish_needs_every_target_runs_in_release_environment_and_alone_writes(self) -> None:
        jobs = self.release["jobs"]
        self.assertEqual(self.release["permissions"], {"contents": "read"})
        publish = jobs["publish"]
        self.assertEqual(publish["environment"], "release")
        self.assertEqual(publish["permissions"], {"contents": "write"})
        for name, job in jobs.items():
            if name != "publish":
                self.assertNotEqual(job.get("permissions", {}).get("contents"), "write", name)
        channels = {entry["channel"] for entry in jobs["build"]["strategy"]["matrix"]["include"]}
        self.assertEqual(channels, {"win-x64", "osx-arm64", "osx-x64", "linux-x64"})
        smoke = {(e["channel"], e["form"]) for e in jobs["smoke"]["strategy"]["matrix"]["include"]}
        for expected in [("win-x64", "setup"), ("osx-arm64", "pkg"), ("osx-arm64", "zip"),
                         ("osx-x64", "pkg"), ("osx-x64", "zip"), ("linux-x64", "appimage")]:
            self.assertIn(expected, smoke)

    def test_no_release_job_restores_a_cache(self) -> None:
        for name, job in self.release["jobs"].items():
            for step in steps(job):
                uses = step.get("uses", "")
                self.assertFalse(uses.startswith(("actions/cache", "Swatinem/rust-cache")), f"{name}: {uses}")
                self.assertNotIn("cache", step.get("with", {}), f"{name}: {step.get('name')}")
        self.assertEqual(self.release["jobs"]["companions"]["with"], {"release": True})
        for name, job in self.companions["jobs"].items():
            for step in steps(job):
                if step.get("uses", "").startswith(("actions/cache", "Swatinem/rust-cache")):
                    self.assertIn("!inputs.release", step.get("if", ""), f"{name}: {step.get('name')}")

    def test_every_third_party_action_is_pinned_to_a_commit(self) -> None:
        for workflow in (self.release, self.upgrade, self.companions):
            for uses in uses_of(workflow):
                if uses.startswith("./"):
                    continue
                self.assertRegex(uses.split(" ")[0], PINNED)

    def test_publish_rechecks_the_gate_and_serializes_across_tags(self) -> None:
        publish = self.release["jobs"]["publish"]
        self.assertEqual(publish["concurrency"], {"group": "release-publish", "cancel-in-progress": False})
        runs = [step.get("run", "") for step in steps(publish)]
        create = next(i for i, run in enumerate(runs) if "gh release create" in run)
        patch = next(i for i, run in enumerate(runs) if "make_latest=true" in run)
        gates = [
            i for i, run in enumerate(runs)
            if "release-gate.py" in run and "--delete-drafts" not in run and "--check-newest" not in run
        ]
        self.assertTrue(any(create < i < patch for i in gates), "full gate between draft creation and publish")
        self.assertTrue(any("--check-newest" in run for run in runs[patch + 1:]), "newest check after publish")

    def test_container_images_are_digest_pinned(self) -> None:
        image = re.compile(r"\b(?:rust|ubuntu|debian|fedora|alpine|docker\.io/[\w./-]+):[\w.-]+(@sha256:[0-9a-f]{64})?")
        for name in ("release.yml", "upgrade-check.yml", "ci-ssh-remote.yml"):
            text = (WORKFLOWS / name).read_text(encoding="utf-8")
            for match in image.finditer(text):
                self.assertTrue(match.group(1), f"{name}: container image {match.group(0)} is not digest-pinned")
        self.assertRegex((WORKFLOWS / "ci-ssh-remote.yml").read_text(), r"rust:1-bullseye@sha256:[0-9a-f]{64}")

    def test_the_release_is_marked_latest_explicitly(self) -> None:
        publish = self.release["jobs"]["publish"]
        self.assertTrue(any("make_latest=true" in step.get("run", "") for step in steps(publish)))

    def test_the_upgrade_check_is_manual_and_callable_with_read_only_permissions(self) -> None:
        self.assertIn("workflow_dispatch", self.upgrade["on"])
        self.assertIn("workflow_call", self.upgrade["on"])
        self.assertEqual(self.upgrade["permissions"], {"contents": "read"})
        for job in self.upgrade["jobs"].values():
            self.assertNotIn("write", str(job.get("permissions", {})))

    def test_no_release_build_enables_the_test_only_update_seam(self) -> None:
        for name in ("release.yml", "upgrade-check.yml", "ci-ssh-remote.yml"):
            text = (WORKFLOWS / name).read_text(encoding="utf-8")
            self.assertNotIn("update-test-seam", text, name)
        builds = [line for line in self.release_text.splitlines() if "cargo build" in line]
        self.assertTrue(builds)
        for line in builds:
            self.assertIn("--release", line)
            self.assertNotIn("--features", line)
            self.assertNotIn("--all-features", line)


if __name__ == "__main__":
    unittest.main()
