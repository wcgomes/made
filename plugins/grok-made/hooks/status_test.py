#!/usr/bin/env python3
"""Lock the grok-made status hook contract.

One JSON object on stdin, with stdin left open, must append sessionId, cwd,
event, and status, then exit 0. PreToolUse ask_user_question and a permission
notification are attention. PostToolUse ask_user_question returns to working.
"""

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


HOOKS = Path(__file__).resolve().parent
STATUS = HOOKS / "bin" / "status.py"
HOOKS_JSON = HOOKS / "hooks.json"


def run_status(event, grok_home):
    """Run one event and require exit before stdin is closed."""
    proc = subprocess.Popen(
        [sys.executable, str(STATUS)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env={"GROK_HOME": str(grok_home), "PATH": "/usr/bin:/bin"},
    )
    assert proc.stdin is not None
    proc.stdin.write(json.dumps(event).encode())
    proc.stdin.flush()
    try:
        code = proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
        raise AssertionError("status.py waited for stdin to close") from None
    stdout = proc.stdout.read() if proc.stdout is not None else b""
    stderr = proc.stderr.read() if proc.stderr is not None else b""
    proc.stdin.close()
    assert code == 0, stderr.decode()
    assert stdout == b""
    assert stderr == b""
    log = Path(grok_home) / "made-grok-status.jsonl"
    lines = log.read_text(encoding="utf-8").splitlines()
    assert len(lines) == 1, lines
    return json.loads(lines[0])


class StatusHookTest(unittest.TestCase):
    def test_one_object_appends_fields_while_stdin_stays_open(self):
        with tempfile.TemporaryDirectory() as grok_home:
            proc = subprocess.Popen(
                [sys.executable, str(STATUS)],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env={"GROK_HOME": grok_home, "PATH": "/usr/bin:/bin"},
            )
            assert proc.stdin is not None
            payload = json.dumps(
                {
                    "hookEventName": "UserPromptSubmit",
                    "sessionId": "sess-1",
                    "cwd": "/work/app",
                }
            ).encode()
            proc.stdin.write(payload)
            proc.stdin.flush()
            try:
                code = proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()
                self.fail("status.py waited for stdin to close")
            stdout = proc.stdout.read() if proc.stdout is not None else b""
            stderr = proc.stderr.read() if proc.stderr is not None else b""
            proc.stdin.close()
            self.assertEqual(code, 0, stderr.decode())
            self.assertEqual(stdout, b"")
            self.assertEqual(stderr, b"")
            line = json.loads(
                (Path(grok_home) / "made-grok-status.jsonl").read_text(encoding="utf-8")
            )
            self.assertEqual(line["sessionId"], "sess-1")
            self.assertEqual(line["cwd"], "/work/app")
            self.assertEqual(line["event"], "UserPromptSubmit")
            self.assertEqual(line["status"], "working")

    def test_question_permission_stop_and_session_end(self):
        cases = [
            (
                {
                    "hookEventName": "SessionStart",
                    "sessionId": "s",
                    "cwd": "/work",
                },
                "idle",
            ),
            (
                {
                    "hookEventName": "PreToolUse",
                    "toolName": "ask_user_question",
                    "sessionId": "s",
                    "cwd": "/work",
                },
                "attention",
            ),
            (
                {
                    "hookEventName": "PostToolUse",
                    "toolName": "ask_user_question",
                    "sessionId": "s",
                    "cwd": "/work",
                },
                "working",
            ),
            (
                {
                    "hookEventName": "Notification",
                    "notificationType": "permission_prompt",
                    "sessionId": "s",
                    "cwd": "/work",
                },
                "attention",
            ),
            (
                {
                    "hookEventName": "Notification",
                    "notificationType": "idle_prompt",
                    "sessionId": "s",
                    "cwd": "/work",
                },
                "idle",
            ),
            (
                {
                    "hookEventName": "Stop",
                    "stopHookActive": True,
                    "sessionId": "s",
                    "cwd": "/work",
                },
                "working",
            ),
            (
                {
                    "hookEventName": "Stop",
                    "stopHookActive": False,
                    "sessionId": "s",
                    "cwd": "/work",
                },
                "idle",
            ),
            (
                {
                    "hookEventName": "StopFailure",
                    "sessionId": "s",
                    "cwd": "/work",
                },
                "error",
            ),
            (
                {
                    "hookEventName": "StopCancelled",
                    "sessionId": "s",
                    "cwd": "/work",
                },
                "idle",
            ),
            (
                {
                    "hookEventName": "SessionEnd",
                    "sessionId": "s",
                    "cwd": "/work",
                },
                "idle",
            ),
        ]
        for event, status in cases:
            with self.subTest(event=event["hookEventName"], status=status):
                with tempfile.TemporaryDirectory() as grok_home:
                    line = run_status(event, grok_home)
                    self.assertEqual(line["sessionId"], "s")
                    self.assertEqual(line["cwd"], "/work")
                    self.assertEqual(line["event"], event["hookEventName"])
                    self.assertEqual(line["status"], status)

    def test_unrecognized_event_is_not_logged(self):
        with tempfile.TemporaryDirectory() as grok_home:
            proc = subprocess.Popen(
                [sys.executable, str(STATUS)],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env={"GROK_HOME": grok_home, "PATH": "/usr/bin:/bin"},
            )
            assert proc.stdin is not None
            proc.stdin.write(
                json.dumps(
                    {
                        "hookEventName": "NotARealHook",
                        "sessionId": "s",
                        "cwd": "/work",
                    }
                ).encode()
            )
            proc.stdin.flush()
            try:
                code = proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()
                self.fail("status.py waited for stdin to close")
            proc.stdin.close()
            self.assertEqual(code, 0)
            log = Path(grok_home) / "made-grok-status.jsonl"
            text = log.read_text(encoding="utf-8") if log.exists() else ""
            self.assertNotIn("unknown", text)
            self.assertNotIn("NotARealHook", text)
            self.assertEqual(text, "")

    def test_windows_chunk_reader_returns_one_object_without_closing_stdin(self):
        import importlib.util

        spec = importlib.util.spec_from_file_location("grok_made_status", STATUS)
        status = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(status)
        payload = json.dumps(
            {
                "hookEventName": "SessionStart",
                "sessionId": "s",
                "cwd": "/work",
            }
        ).encode()
        pending = {"chunk": payload}

        def read_chunk(_timeout):
            # One value, then no more bytes and no EOF. select() is not used.
            return pending.pop("chunk", None)

        event = status.read_event_with(read_chunk)
        self.assertEqual(event["sessionId"], "s")
        self.assertEqual(event["cwd"], "/work")
        self.assertIsNone(status.status_of({"hookEventName": "NotARealHook"}))

    def test_hooks_json_maps_ask_user_question(self):
        hooks = json.loads(HOOKS_JSON.read_text(encoding="utf-8"))["hooks"]
        pre = hooks["PreToolUse"][0]["matcher"]
        post = hooks["PostToolUse"][0]["matcher"]
        self.assertIn("ask_user_question", pre.split("|"))
        self.assertIn("ask_user_question", post.split("|"))
        permission = hooks["Notification"][0]["matcher"]
        self.assertIn("permission_prompt", permission.split("|"))
        self.assertIn("idle_prompt", permission.split("|"))


if __name__ == "__main__":
    unittest.main()
