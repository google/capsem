"""Tests for scripts/lib/exec_lock.sh::acquire_exec_lock.

The helper is the single source of truth for every lock acquisition in
the justfile (dev/shell/run/test/smoke/bench/...), so flaking here would
silently re-enable the concurrent-just-test race it exists to prevent.
"""

import os
import subprocess
from pathlib import Path

REPO_ROOT = Path(__file__).parent.parent
HELPER = REPO_ROOT / "scripts/lib/exec_lock.sh"


def _spawn_holder(lock_path, hold_seconds, env=None):
    """Spawn a bash subshell that sources the helper, acquires the lock, and holds.

    Prints HELD on stdout once the advisory lock is taken; that's the sync point
    the test uses to know the lock is definitely held before probing.
    """
    cmd = [
        "bash", "-c",
        f"source '{HELPER}' && acquire_exec_lock '{lock_path}' && "
        f"echo HELD && sleep {hold_seconds}",
    ]
    return subprocess.Popen(
        cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env,
    )


def _try_acquire(lock_path, timeout=5, env=None):
    """Synchronously try to take the lock; return (returncode, stderr)."""
    result = subprocess.run(
        ["bash", "-c",
         f"source '{HELPER}' && acquire_exec_lock '{lock_path}' && echo OK"],
        capture_output=True, text=True, timeout=timeout, env=env,
    )
    return result.returncode, result.stderr


def _force_python_lock_env():
    env = os.environ.copy()
    env["CAPSEM_EXEC_LOCK_FORCE_PYTHON"] = "1"
    return env


def test_acquire_blocks_concurrent_holder(tmp_path):
    """Second acquire on a held lock must exit non-zero with a clear message."""
    lock = tmp_path / "concurrent.lock"
    # Popen as a context manager closes stdout/stderr pipes on exit --
    # without it, pytest's filterwarnings=error surfaces the leftover
    # _io.FileIO handles as PytestUnraisableExceptionWarning.
    with _spawn_holder(lock, hold_seconds=2) as holder:
        assert holder.stdout.readline().strip() == "HELD", (
            "holder failed to acquire before probe"
        )
        rc, err = _try_acquire(lock)
        assert rc != 0, "concurrent acquire should have failed but exited 0"
        assert "another agent holds" in err, (
            f"expected concurrent-holder message, got stderr: {err!r}"
        )
        assert str(lock) in err, (
            f"stderr should include the lock path, got: {err!r}"
        )
        holder.wait(timeout=5)


def test_python_fallback_blocks_concurrent_holder(tmp_path):
    """macOS runners without flock still need the same non-blocking lock."""
    lock = tmp_path / "python-fallback.lock"
    env = _force_python_lock_env()
    with _spawn_holder(lock, hold_seconds=2, env=env) as holder:
        assert holder.stdout.readline().strip() == "HELD", (
            "python fallback holder failed to acquire before probe"
        )
        rc, err = _try_acquire(lock, env=env)
        assert rc != 0, "fallback concurrent acquire should fail"
        assert "another agent holds" in err
        holder.wait(timeout=5)


def test_acquire_reacquires_after_holder_exits(tmp_path):
    """Once the holder releases (process exit), the lock must be reclaimable."""
    lock = tmp_path / "reacquire.lock"
    with _spawn_holder(lock, hold_seconds=0.1) as holder:
        assert holder.stdout.readline().strip() == "HELD"
        holder.wait(timeout=5)
    rc, err = _try_acquire(lock)
    assert rc == 0, f"re-acquire after holder exit should succeed, stderr: {err!r}"


def test_acquire_creates_parent_dir(tmp_path):
    """mkdir -p the parent if the lockfile's directory doesn't exist yet.

    The justfile dev/shell/run sites run before ~/.capsem/run exists on a
    freshly bootstrapped machine; the helper must create it rather than
    fail with ENOENT.
    """
    lock = tmp_path / "nested" / "sub" / "dir" / "fresh.lock"
    assert not lock.parent.exists()
    rc, err = _try_acquire(lock)
    assert rc == 0, f"first acquire should succeed on fresh path, stderr: {err!r}"
    assert lock.exists(), "lockfile should be created by acquire_exec_lock"
