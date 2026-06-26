"""Regression tests for the simulated install fixture itself."""

from __future__ import annotations

import subprocess

from . import conftest


def test_kill_service_stops_systemd_unit_before_process_kill(monkeypatch, tmp_path):
    """The deb harness unit can restart services, so stop it before pkill."""
    calls: list[list[str]] = []

    monkeypatch.setenv("CAPSEM_DEB_INSTALLED", "1")
    monkeypatch.setattr(
        conftest.shutil,
        "which",
        lambda name: "/usr/bin/systemctl" if name == "systemctl" else None,
    )
    monkeypatch.setattr(conftest, "RUN_DIR", tmp_path)

    def fake_run(cmd, **kwargs):
        calls.append(list(cmd))
        return subprocess.CompletedProcess(cmd, 0, "", "")

    monkeypatch.setattr(conftest.subprocess, "run", fake_run)

    conftest._kill_service()

    stop_cmd = ["systemctl", "--user", "stop", "capsem"]
    assert stop_cmd in calls
    stop_index = calls.index(stop_cmd)
    pkill_indices = [
        index for index, cmd in enumerate(calls) if cmd[:2] == ["pkill", "-f"]
    ]
    assert pkill_indices
    assert stop_index < min(pkill_indices)


def test_kill_service_skips_systemd_stop_outside_deb_harness(monkeypatch, tmp_path):
    calls: list[list[str]] = []

    monkeypatch.delenv("CAPSEM_DEB_INSTALLED", raising=False)
    monkeypatch.setattr(conftest.shutil, "which", lambda name: "/usr/bin/systemctl")
    monkeypatch.setattr(conftest, "RUN_DIR", tmp_path)
    monkeypatch.setattr(
        conftest.subprocess,
        "run",
        lambda cmd, **kwargs: calls.append(list(cmd))
        or subprocess.CompletedProcess(cmd, 0, "", ""),
    )

    conftest._kill_service()

    assert ["systemctl", "--user", "stop", "capsem"] not in calls
