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
import threading
import time
from unittest.mock import MagicMock

import psutil
import pytest

from conftest import (
    _ancestry,
    _CAUGHT_THREAD_EXCEPTIONS,
    _is_pytest_descendant,
    _missing_required_artifacts,
    _thread_exception_hook,
    get_capsem_processes,
)


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


# ---------------------------------------------------------------------------
# Global thread-exception hook. Daemon threads that raise (typical source:
# async fixture teardown races, server loops) previously surfaced as
# PytestUnhandledThreadExceptionWarning -- reported, but not gating. With
# the hook installed at conftest import time, every such exception is
# captured and pytest_sessionfinish fails the session.
# ---------------------------------------------------------------------------


def test_thread_exception_hook_is_installed():
    # Installed unconditionally at conftest import so even fixture setup
    # and collection-phase thread leaks are caught.
    assert threading.excepthook is _thread_exception_hook


def test_thread_exception_hook_captures_daemon_thread_exception():
    before = len(_CAUGHT_THREAD_EXCEPTIONS)

    def boom():
        raise ValueError("expected-test-exception-please-ignore")

    t = threading.Thread(target=boom, daemon=True)
    t.start()
    t.join(timeout=2)
    assert not t.is_alive()

    try:
        # join() returns only after the thread finishes, by which time
        # the interpreter has already invoked threading.excepthook.
        assert len(_CAUGHT_THREAD_EXCEPTIONS) == before + 1
        last = _CAUGHT_THREAD_EXCEPTIONS[-1]
        assert last.exc_type is ValueError
        assert "expected-test-exception" in str(last.exc_value)
    finally:
        # Pop our synthetic exception so the session-finish gate does not
        # fail the real run on a test-planted fake.
        while len(_CAUGHT_THREAD_EXCEPTIONS) > before:
            _CAUGHT_THREAD_EXCEPTIONS.pop()


# ---------------------------------------------------------------------------
# CI artifact gate. Locally, tests that depend on built artifacts (manifest,
# initrd, cross-compiled agent) skip when those artifacts are absent. In CI
# that silent skip is a bug -- stages earlier in `just test` were supposed
# to build the artifact, and a skip masks the breakage. `CAPSEM_REQUIRE
# _ARTIFACTS=1` flips the gate to fail-fast.
# ---------------------------------------------------------------------------


def test_missing_required_artifacts_returns_empty_when_env_unset(tmp_path):
    # Without the opt-in env var, the gate is inert even if paths are missing.
    missing_path = tmp_path / "does-not-exist"
    assert _missing_required_artifacts({}, {"fake": missing_path}) == []
    # Empty string is falsy too -- do not strict-gate on a blank env var.
    assert _missing_required_artifacts(
        {"CAPSEM_REQUIRE_ARTIFACTS": ""},
        {"fake": missing_path},
    ) == []


def test_missing_required_artifacts_lists_missing_when_env_set(tmp_path):
    present = tmp_path / "exists"
    present.write_text("x")
    missing = tmp_path / "missing"
    got = _missing_required_artifacts(
        {"CAPSEM_REQUIRE_ARTIFACTS": "1"},
        {"present": present, "missing": missing},
    )
    assert got == ["missing"]


def test_required_artifacts_manifest_path_is_flat():
    """The asset manifest lives at assets/manifest.json, not per-arch.

    Every production reader (capsem-service boot, capsem setup, gen_manifest,
    release workflow) and the builder's generate_checksums writer agree on
    the flat top-level path. A per-arch path in _REQUIRED_ARTIFACTS would
    never resolve on a freshly built tree and would fail the CI gate for a
    successful build -- which is exactly how we found this.
    """
    from tests.conftest import _PROJECT_ROOT, _REQUIRED_ARTIFACTS

    assert "assets/manifest.json" in _REQUIRED_ARTIFACTS
    assert (
        _REQUIRED_ARTIFACTS["assets/manifest.json"]
        == _PROJECT_ROOT / "assets" / "manifest.json"
    )


# ---------------------------------------------------------------------------
# MCP proc-teardown helper. The capsem-mcp fixtures and the capsem-e2e MCP
# tests spawn capsem-mcp via subprocess.Popen with stdin=PIPE, stdout=PIPE,
# then terminate + wait on shutdown without closing the PIPE fds. Under
# filterwarnings=error that leaks as PytestUnraisableExceptionWarning on
# every test in the dir. kill_mcp_proc centralizes the correct sequence.
# ---------------------------------------------------------------------------


def test_kill_mcp_proc_closes_stdio_pipes():
    import subprocess
    import sys

    from helpers.mcp import kill_mcp_proc

    proc = subprocess.Popen(
        [sys.executable, "-c", "import sys; sys.stdin.read()"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    kill_mcp_proc(proc)

    assert proc.returncode is not None, "process should be reaped"
    # All three pipe fds must be closed so no ResourceWarning escapes at
    # test teardown. Before the fix, only proc.returncode was set; the
    # PIPE fds stayed open until GC.
    assert proc.stdin is None or proc.stdin.closed
    assert proc.stdout is None or proc.stdout.closed
    assert proc.stderr is None or proc.stderr.closed
