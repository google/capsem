"""APT stalls must expire with their children, preserving package authority."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

import psutil
import pytest
from capsem_builder.image.tools.bootstrap import linux_workspace as provisioner

ROOT = Path(__file__).resolve().parents[3]


@pytest.mark.parametrize("invalid", ["0", "-1", "true", '"300"'])
def test_invalid_apt_bounds_fail_before_starting_packages(tmp_path: Path, invalid: str) -> None:
    config = tmp_path / "gate.toml"
    config.write_text(
        (ROOT / "config/gate.toml")
        .read_text()
        .replace("update_timeout_seconds = 300", f"update_timeout_seconds = {invalid}", 1)
    )
    with pytest.raises(provisioner.ProvisionError, match="APT limits must be positive integers"):
        provisioner._apt_command(config, "update")


@pytest.mark.parametrize("invalid", ["0", "-1", "nan", "inf", "true"])
def test_invalid_stop_grace_is_rejected(tmp_path: Path, invalid: str) -> None:
    config = tmp_path / "gate.toml"
    config.write_text(
        (ROOT / "config/gate.toml")
        .read_text()
        .replace("cancellation_grace_seconds = 10.0", f"cancellation_grace_seconds = {invalid}", 1)
    )
    with pytest.raises(provisioner.ProvisionError, match="cancellation grace"):
        provisioner._apt_command(config, "update")


def test_stalled_apt_update_kills_its_child_and_never_starts_install(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    assert shutil.which("timeout"), "GNU timeout is a Linux provisioning prerequisite"
    config = tmp_path / "gate.toml"
    config.write_text(
        (ROOT / "config/gate.toml")
        .read_text()
        .replace("update_timeout_seconds = 300", "update_timeout_seconds = 1", 1)
        .replace("cancellation_grace_seconds = 10.0", "cancellation_grace_seconds = 0.1", 1)
    )
    child_pid = tmp_path / "child.pid"
    calls = tmp_path / "calls"
    apt = tmp_path / "apt-get"
    apt.write_text(
        f"#!{sys.executable}\n"
        "import pathlib, signal, subprocess, sys, time\n"
        "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
        f"with pathlib.Path({str(calls)!r}).open('a') as log: log.write(repr(sys.argv[1:])+'\\n')\n"
        "child = subprocess.Popen([sys.executable, '-c', "
        "'import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(60)'])\n"
        f"pathlib.Path({str(child_pid)!r}).write_text(str(child.pid))\n"
        "time.sleep(60)\n"
    )
    apt.chmod(0o700)
    monkeypatch.setenv("PATH", f"{tmp_path}{os.pathsep}{os.environ['PATH']}")
    monkeypatch.setattr(provisioner.os, "geteuid", lambda: 0)
    started = time.monotonic()
    with pytest.raises(subprocess.CalledProcessError):
        provisioner.install_and_verify(
            config, provisioner.PackageManager.APT, architecture=provisioner.Arch.ARM64
        )
    assert time.monotonic() - started < 4
    assert len(calls.read_text().splitlines()) == 1
    assert "update" in calls.read_text()
    try:
        child = psutil.Process(int(child_pid.read_text()))
        try:
            child.wait(timeout=2)
        except psutil.TimeoutExpired:
            pytest.fail("timed-out APT left a live worker")
        finally:
            if child.is_running() and child.status() != psutil.STATUS_ZOMBIE:
                child.kill()
                child.wait(timeout=2)
    except psutil.NoSuchProcess:
        pass
