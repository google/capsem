"""Stopping a daemon, and telling the difference from stopping nothing.

`stop_gate_pidfile` on a path no binary writes removes a file that was never
there and returns success. That is how sixteen `capsem-service` processes, each
holding a `capsem-tray`, accumulated in a single day while every run reported a
clean shutdown -- a no-op cleanup is indistinguishable from a successful one
unless something counts.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import time
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import pidfiles
from capsem_builder.gate.errors import GateError
from helpers.sign import sign_binary

PROJECT_ROOT = Path(__file__).resolve().parents[1]
SETTINGS = gate_config.load(PROJECT_ROOT).pidfiles


def _sleeper(tmp_path: Path, name: str = "sleeper") -> subprocess.Popen:
    binary = tmp_path / name
    shutil.copy("/bin/sleep", binary)
    # Copying strips the signature, and Apple Silicon kills an unsigned binary
    # at exec. A no-op on Linux.
    sign_binary(binary)
    return subprocess.Popen([str(binary), "300"])


# ---------------------------------------------------------------------------
# Liveness
# ---------------------------------------------------------------------------


def test_a_live_process_is_running(tmp_path: Path) -> None:
    process = _sleeper(tmp_path)
    try:
        assert pidfiles.running(process.pid, SETTINGS)
    finally:
        process.kill()
        process.wait()


def test_an_exited_process_is_not_running(tmp_path: Path) -> None:
    process = _sleeper(tmp_path)
    process.kill()
    process.wait()

    assert not pidfiles.running(process.pid, SETTINGS)


def test_an_unreaped_zombie_is_not_running(tmp_path: Path) -> None:
    """A zombie answers `kill -0` for as long as nobody waits on it.

    Treating that as alive makes the stop loop spend its whole timeout on a
    process that has already exited, then report a leak that is not one.
    """
    process = _sleeper(tmp_path)
    process.kill()
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline and pidfiles.running(process.pid, SETTINGS):
        time.sleep(0.05)

    try:
        assert not pidfiles.running(process.pid, SETTINGS)
    finally:
        process.wait()


# ---------------------------------------------------------------------------
# Stopping
# ---------------------------------------------------------------------------


def test_an_absent_pidfile_is_not_a_failure(tmp_path: Path) -> None:
    """The daemon may never have started; that is not this function's problem."""
    pidfiles.stop(tmp_path / "service.pid", SETTINGS)


def test_a_pidfile_naming_a_dead_process_is_just_removed(tmp_path: Path) -> None:
    process = _sleeper(tmp_path)
    process.kill()
    process.wait()
    pidfile = tmp_path / "service.pid"
    pidfile.write_text(str(process.pid))

    pidfiles.stop(pidfile, SETTINGS)

    assert not pidfile.exists()


def test_a_live_process_is_stopped_and_its_pidfile_removed(tmp_path: Path) -> None:
    process = _sleeper(tmp_path)
    pidfile = tmp_path / "service.pid"
    pidfile.write_text(f"{process.pid}\n")

    try:
        pidfiles.stop(pidfile, SETTINGS)

        assert not pidfiles.running(process.pid, SETTINGS)
        assert not pidfile.exists()
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()


def test_a_process_ignoring_sigterm_is_killed(tmp_path: Path) -> None:
    """SIGTERM, wait, SIGKILL. A daemon mid-flush does not get to stay."""
    script = tmp_path / "stubborn.sh"
    script.write_text("#!/bin/bash\ntrap '' TERM\nsleep 300\n")
    script.chmod(0o755)
    process = subprocess.Popen(["/bin/bash", str(script)])
    pidfile = tmp_path / "gateway.pid"
    pidfile.write_text(str(process.pid))

    try:
        pidfiles.stop(pidfile, SETTINGS)

        assert not pidfiles.running(process.pid, SETTINGS)
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()


def test_a_process_that_will_not_die_fails_rather_than_warns(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Returning success here is exactly what made the leak invisible."""
    pidfile = tmp_path / "service.pid"
    pidfile.write_text(str(os.getpid()))
    monkeypatch.setattr(pidfiles, "running", lambda _pid, _settings: True)
    monkeypatch.setattr(pidfiles.os, "kill", lambda _pid, _signal: None)

    with pytest.raises(GateError, match="did not exit"):
        pidfiles.stop(pidfile, SETTINGS)

    assert pidfile.exists(), "a pidfile whose process survived must not be removed"


def test_a_pidfile_holding_junk_is_removed_without_signalling(tmp_path: Path) -> None:
    pidfile = tmp_path / "service.pid"
    pidfile.write_text("not-a-pid\n")

    pidfiles.stop(pidfile, SETTINGS)

    assert not pidfile.exists()


# ---------------------------------------------------------------------------
# Order
# ---------------------------------------------------------------------------


def test_the_gateway_is_stopped_before_the_service(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The gateway owns the fixed localhost port.

    Left running past its service, it attaches the next profile to a UDS
    pointing at a run directory that has already been deleted.
    """
    stopped: list[str] = []
    monkeypatch.setattr(
        pidfiles, "stop", lambda pidfile, _settings: stopped.append(pidfile.name)
    )

    pidfiles.stop_gate_service(tmp_path, SETTINGS)

    assert stopped == ["gateway.pid", "service.pid"]


def test_every_stopped_pidfile_is_declared_in_config() -> None:
    """The names are data, so the wiring guard can check them against the crates.

    `tests/test_pidfile_cleanup_is_wired.py` proves each of these is a file some
    binary actually writes -- without which a typo reaps nothing and reports
    success.
    """
    assert SETTINGS.names
    assert all(name.endswith(".pid") for name in SETTINGS.names)
