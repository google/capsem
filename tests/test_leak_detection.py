"""Unit tests for leak-detection helpers in tests/conftest.py.

These are infra tests -- they monkeypatch psutil to simulate host-level
failure modes (like macOS KERN_PROCARGS2 access errors under load) that
are hard to reproduce deterministically otherwise. They guard the leak
fixture from crashing the suite when a sibling process on the host
rejects a cmdline read.
"""

import os
import subprocess
import sys
import time
from unittest.mock import MagicMock

import psutil
import pytest

from conftest import _ancestry, _is_pytest_descendant, get_capsem_processes


class _FakeProc:
    """Minimal psutil.Process stand-in for monkeypatched process_iter."""

    def __init__(self, pid, name, cmdline_impl):
        self.pid = pid
        self.info = {"pid": pid, "name": name}
        self._cmdline_impl = cmdline_impl

    def cmdline(self):
        return self._cmdline_impl()


@pytest.fixture
def patch_iter(monkeypatch):
    def _patch(procs):
        monkeypatch.setattr(
            psutil,
            "process_iter",
            lambda attrs=None: iter(procs),
        )
    return _patch


def test_ignores_non_capsem_cmdline_errors(patch_iter):
    """SystemError from a non-capsem sibling proc must not crash iteration.

    Re-creates the failure class observed under `just test` stage 5: psutil's
    `_psosx.py::cmdline` returns a SystemError for some host proc, and that
    propagates past the old attrs-prefetch implementation of get_capsem_processes.
    """
    def angry_cmdline():
        raise SystemError("proc_cmdline returned a result with an exception set")

    patch_iter([
        _FakeProc(1, "launchd", angry_cmdline),
        _FakeProc(42, "capsem-process", lambda: ["capsem-process", "--id", "vm"]),
    ])

    got = get_capsem_processes()
    assert 42 in got
    assert got[42]["name"] == "capsem-process"
    assert "vm" in got[42]["cmdline"]


def test_tolerates_capsem_cmdline_permission_error(patch_iter):
    """A capsem-* proc with unreadable cmdline (PermissionError) still records.

    PermissionError is how macOS surfaces KERN_PROCARGS2 denials. Previously the
    conftest catch-clause only included psutil.NoSuchProcess / AccessDenied, so
    this case leaked. Now the proc is still accounted for; cmdline degrades to
    an empty string.
    """
    def denied():
        raise PermissionError(13, "force permission denied")

    patch_iter([_FakeProc(99, "capsem-gateway", denied)])

    got = get_capsem_processes()
    assert 99 in got
    assert got[99]["name"] == "capsem-gateway"
    assert got[99]["cmdline"] == ""


def test_still_skips_nosuchprocess(patch_iter):
    """A proc that vanishes between listing and cmdline fetch is dropped quietly."""
    def vanished():
        raise psutil.NoSuchProcess(pid=7)

    patch_iter([
        _FakeProc(7, "capsem-service", vanished),
        _FakeProc(8, "capsem-tray", lambda: ["capsem-tray"]),
    ])

    got = get_capsem_processes()
    assert 7 in got  # name is still known from the iter; cmdline just blanks
    assert got[7]["cmdline"] == ""
    assert 8 in got


# ---------------------------------------------------------------------------
# Ownership filter -- regression tests for the Claude-Code-spawned
# capsem-mcp false positive. The old detector flagged any capsem-* PID on
# the host; sibling tools (Claude's MCP subprocess, a dev service started
# manually in another shell) got attributed to whichever test happened to
# run first. These tests pin the ancestry-based filter that scopes leak
# detection to pytest's own process tree.
# ---------------------------------------------------------------------------


def test_ancestry_of_init_excludes_self():
    # pid 1 (launchd on macOS, init on Linux) has no pytest in its ancestry,
    # because pytest is its descendant, not its ancestor.
    assert os.getpid() not in _ancestry(1)
    assert not _is_pytest_descendant(1)


def test_ancestry_of_own_subprocess_includes_self():
    # A process we spawn with subprocess.Popen is our direct child; walking
    # its parent chain must find our PID.
    proc = subprocess.Popen(
        [sys.executable, "-c", "import time; time.sleep(10)"],
    )
    try:
        # psutil caches parent-child relationships lazily; give the kernel a
        # moment to register the child before walking (macOS proc table).
        for _ in range(20):
            if os.getpid() in _ancestry(proc.pid):
                break
            time.sleep(0.05)
        assert os.getpid() in _ancestry(proc.pid)
        assert _is_pytest_descendant(proc.pid)
    finally:
        proc.terminate()
        proc.wait(timeout=5)


def test_ancestry_returns_empty_for_missing_pid():
    # A PID that doesn't exist (or has vanished) returns an empty set without
    # raising. Leak detection must not crash when a suspect dies mid-check.
    # Use a very large PID that is vanishingly unlikely to be in use.
    assert _ancestry(2**31 - 1) == set()
    assert not _is_pytest_descendant(2**31 - 1)
