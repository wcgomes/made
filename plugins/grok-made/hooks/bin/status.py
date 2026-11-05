#!/usr/bin/env python3
"""Record one Grok hook event as a session status line.

Grok leaves stdin open after the JSON object, so this reads one value and
exits. It never denies a tool or a prompt.
"""

import json
import os
import sys
import threading
import time
from datetime import datetime, timezone
from pathlib import Path


def read_event():
    return read_event_with(read_stdin_chunk)


def read_event_with(read_chunk):
    """Read one JSON object. `read_chunk` returns bytes, b"" at EOF, or None."""
    decoder = json.JSONDecoder()
    buf = ""
    deadline = time.monotonic() + 2
    while time.monotonic() < deadline:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        chunk = read_chunk(min(0.2, remaining))
        if chunk is None:
            continue
        if chunk == b"":
            break
        buf += chunk.decode("utf-8", "replace")
        text = buf.lstrip()
        if not text:
            continue
        try:
            event, _ = decoder.raw_decode(text)
        except json.JSONDecodeError:
            continue
        if isinstance(event, dict):
            return event
        return None
    return None


def read_stdin_chunk(timeout):
    if os.name == "nt":
        return read_stdin_chunk_windows(timeout)
    return read_stdin_chunk_posix(timeout)


def read_stdin_chunk_posix(timeout):
    ready, _, _ = select_stdin(timeout)
    if not ready:
        return None
    return os.read(sys.stdin.fileno(), 65536)


def read_stdin_chunk_windows(timeout):
    """One stdin chunk without select(); select cannot wait on a Windows pipe."""
    box = {}

    def worker():
        try:
            box["chunk"] = os.read(sys.stdin.fileno(), 65536)
        except OSError:
            box["chunk"] = b""

    thread = threading.Thread(target=worker, daemon=True)
    thread.start()
    thread.join(timeout)
    return box.get("chunk")


def select_stdin(timeout):
    import select

    return select.select([sys.stdin], [], [], timeout)


def text_of(event, *keys):
    for key in keys:
        value = event.get(key)
        if isinstance(value, str) and value:
            return value
    return ""


def status_of(event):
    name = text_of(event, "hookEventName", "hook_event_name").lower().replace("-", "_")
    note = text_of(event, "notificationType", "notification_type").lower()
    tool = text_of(event, "toolName", "tool_name").lower()
    asking = tool in {"ask_user_question", "askuserquestion", "askquestion"}
    stop_active = bool(event.get("stopHookActive") or event.get("stop_hook_active"))
    if name in {"session_start", "sessionstart"}:
        return "idle"
    if name in {"user_prompt_submit", "userpromptsubmit"}:
        return "working"
    if name in {"pre_tool_use", "pretooluse"} and asking:
        return "attention"
    if name in {"post_tool_use", "posttooluse"} and asking:
        return "working"
    if note == "permission_prompt":
        return "attention"
    if note == "idle_prompt":
        return "idle"
    if name in {"stop_failure", "stopfailure"}:
        return "error"
    if name in {"stop_cancelled", "stopcancelled", "session_end", "sessionend"}:
        return "idle"
    if name == "stop":
        return "working" if stop_active else "idle"
    return None


def log_path():
    home = os.environ.get("GROK_HOME") or str(Path.home() / ".grok")
    return Path(home) / "made-grok-status.jsonl"


def main():
    event = read_event()
    if not event:
        return 0
    status = status_of(event)
    if status not in {"idle", "working", "attention", "error"}:
        return 0
    line = {
        "at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "sessionId": text_of(event, "sessionId", "session_id"),
        # The parent of this hook process: the Grok session process that ran
        # the hook. Companions sharing the server-global log use it to prove
        # which tab the event belongs to.
        "pid": os.getppid(),
        "cwd": text_of(event, "cwd"),
        "event": text_of(event, "hookEventName", "hook_event_name"),
        "notification": text_of(event, "notificationType", "notification_type"),
        "status": status,
    }
    path = log_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(line, ensure_ascii=False) + "\n")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception:
        sys.exit(0)
