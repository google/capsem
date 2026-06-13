"""Shared capsem-mock-server launcher for release and integration checks."""

from __future__ import annotations

import fcntl
import json
import selectors
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any


PROJECT_ROOT = Path(__file__).resolve().parents[1]
MOCK_SERVER_BINARY = PROJECT_ROOT / "scripts" / "mock_server_runtime.py"
MOCK_SERVER_ADDR = "127.0.0.1:3713"
MOCK_SERVER_LOCK = Path(tempfile.gettempdir()) / "capsem-mock-server-3713.lock"


def _lock_path_for_addr(addr: str) -> Path:
    safe_addr = addr.replace(":", "-").replace(".", "-")
    return Path(tempfile.gettempdir()) / f"capsem-mock-server-{safe_addr}.lock"


def _acquire_lock(addr: str = MOCK_SERVER_ADDR, timeout_s: float = 120) -> Any:
    lock_file = _lock_path_for_addr(addr).open("w")
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            return lock_file
        except BlockingIOError:
            time.sleep(0.1)
    lock_file.close()
    raise TimeoutError(f"timed out waiting for {_lock_path_for_addr(addr)}")


def _address_in_use_error(exc: BaseException) -> bool:
    text = str(exc)
    return "Address already in use" in text or "[Errno 48]" in text or "[Errno 98]" in text


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


def start_mock_server(
    *,
    addr: str = MOCK_SERVER_ADDR,
    timeout_s: float = 120,
    retry_interval_s: float = 0.2,
) -> tuple[subprocess.Popen[str], dict[str, Any]]:
    if not MOCK_SERVER_BINARY.exists():
        raise FileNotFoundError(
            f"{MOCK_SERVER_BINARY} not found; restore scripts/mock_server_runtime.py"
        )
    lock_file = _acquire_lock(addr, timeout_s=timeout_s)
    deadline = time.monotonic() + timeout_s
    last_error: BaseException | None = None
    while time.monotonic() < deadline:
        proc = subprocess.Popen(
            [sys.executable, str(MOCK_SERVER_BINARY), "--addr", addr],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
        )
        proc._capsem_mock_server_lock = lock_file  # type: ignore[attr-defined]
        try:
            ready = read_ready_json(proc)
            return proc, ready
        except Exception as exc:
            last_error = exc
            stop_process(proc)
            if not _address_in_use_error(exc):
                raise
            time.sleep(retry_interval_s)
            lock_file = _acquire_lock(addr, timeout_s=timeout_s)
    lock_file.close()
    raise TimeoutError(f"timed out starting capsem-mock-server on {addr}") from last_error


def local_fixture_env(base_url: str) -> dict[str, str]:
    return {"CAPSEM_MOCK_SERVER_BASE_URL": base_url}
