"""Shared capsem-mock-server launcher for release and integration checks."""

from __future__ import annotations

import fcntl
import json
import selectors
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Any


PROJECT_ROOT = Path(__file__).resolve().parents[1]
MOCK_SERVER_BINARY = PROJECT_ROOT / "target" / "debug" / "capsem-mock-server"
MOCK_SERVER_ADDR = "127.0.0.1:3713"
MOCK_SERVER_LOCK = Path(tempfile.gettempdir()) / "capsem-mock-server-3713.lock"


def _acquire_lock(timeout_s: float = 120) -> Any:
    lock_file = MOCK_SERVER_LOCK.open("w")
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            return lock_file
        except BlockingIOError:
            time.sleep(0.1)
    lock_file.close()
    raise TimeoutError(f"timed out waiting for {MOCK_SERVER_LOCK}")


def read_ready_json(proc: subprocess.Popen[str], timeout_s: float = 10) -> dict[str, Any]:
    if proc.stdout is None:
        raise RuntimeError("capsem-mock-server stdout must be piped")
    selector = selectors.DefaultSelector()
    selector.register(proc.stdout, selectors.EVENT_READ)
    deadline = time.monotonic() + timeout_s
    lines: list[str] = []
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise RuntimeError(
                f"capsem-mock-server exited early with code {proc.returncode}: "
                f"{''.join(lines)}"
            )
        for key, _ in selector.select(timeout=0.2):
            line = key.fileobj.readline()
            if not line:
                continue
            lines.append(line)
            try:
                payload = json.loads(line)
            except json.JSONDecodeError:
                continue
            if payload.get("service") == "capsem-mock-server":
                return payload
    raise TimeoutError(
        "capsem-mock-server did not print ready JSON; "
        f"stdout={''.join(lines)!r}"
    )


def stop_process(proc: subprocess.Popen[str] | None) -> None:
    if proc is None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)
    if proc.stdout is not None:
        proc.stdout.close()
    lock_file = getattr(proc, "_capsem_mock_server_lock", None)
    if lock_file is not None:
        fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)
        lock_file.close()


def start_mock_server() -> tuple[subprocess.Popen[str], dict[str, Any]]:
    if not MOCK_SERVER_BINARY.exists():
        raise FileNotFoundError(
            f"{MOCK_SERVER_BINARY} not found; run `cargo build -p capsem-mock-server`"
        )
    lock_file = _acquire_lock()
    proc = subprocess.Popen(
        [str(MOCK_SERVER_BINARY), "--addr", MOCK_SERVER_ADDR],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    proc._capsem_mock_server_lock = lock_file  # type: ignore[attr-defined]
    try:
        ready = read_ready_json(proc)
    except Exception:
        stop_process(proc)
        raise
    return proc, ready


def local_fixture_env(base_url: str) -> dict[str, str]:
    return {"CAPSEM_MOCK_SERVER_BASE_URL": base_url}
