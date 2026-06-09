"""Local debug upstream fixture helpers for network tests."""

import json
import selectors
import subprocess
import tempfile
import time
from pathlib import Path
import fcntl

PROJECT_ROOT = Path(__file__).resolve().parents[2]
DEBUG_UPSTREAM_BINARY = PROJECT_ROOT / "target" / "debug" / "capsem-debug-upstream"
DEBUG_UPSTREAM_ADDR = "127.0.0.1:3713"
DEBUG_UPSTREAM_LOCK = Path(tempfile.gettempdir()) / "capsem-debug-upstream-3713.lock"


def _acquire_lock(timeout_s=120):
    lock_file = DEBUG_UPSTREAM_LOCK.open("w")
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            return lock_file
        except BlockingIOError:
            time.sleep(0.1)
    lock_file.close()
    raise TimeoutError(f"timed out waiting for {DEBUG_UPSTREAM_LOCK}")


def read_ready_json(proc, timeout_s=10):
    selector = selectors.DefaultSelector()
    selector.register(proc.stdout, selectors.EVENT_READ)
    deadline = time.monotonic() + timeout_s
    lines = []
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise RuntimeError(
                f"capsem-debug-upstream exited early with code {proc.returncode}: "
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
            if payload.get("service") == "capsem-debug-upstream":
                return payload
    raise TimeoutError(
        "capsem-debug-upstream did not print ready JSON; "
        f"stdout={''.join(lines)!r}"
    )


def stop_process(proc):
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
    lock_file = getattr(proc, "_capsem_debug_upstream_lock", None)
    if lock_file is not None:
        fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)
        lock_file.close()


def start_debug_upstream():
    lock_file = _acquire_lock()
    proc = subprocess.Popen(
        [str(DEBUG_UPSTREAM_BINARY), "--addr", DEBUG_UPSTREAM_ADDR],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    proc._capsem_debug_upstream_lock = lock_file
    try:
        ready = read_ready_json(proc)
    except Exception:
        stop_process(proc)
        raise
    return proc, ready
