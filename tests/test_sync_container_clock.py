"""Contracts for the bounded macOS container-VM clock synchronizer."""

from __future__ import annotations

import importlib.util
import re
import subprocess
from pathlib import Path

import pytest


PROJECT_ROOT = Path(__file__).resolve().parent.parent
SCRIPT = PROJECT_ROOT / "scripts" / "sync-container-clock.py"


def _module():
    spec = importlib.util.spec_from_file_location("sync_container_clock_script", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_sync_uses_colima_vm_and_hard_timeout(monkeypatch) -> None:
    module = _module()
    calls: list[tuple[list[str], dict[str, object]]] = []

    def fake_run(command, **kwargs):
        calls.append((command, kwargs))
        return subprocess.CompletedProcess(command, 0, stdout="synced\n", stderr="")

    monkeypatch.setattr(module.subprocess, "run", fake_run)
    module.sync_container_clock(timeout_seconds=10)

    command, kwargs = calls[0]
    assert command[:6] == ["colima", "ssh", "--", "sudo", "date", "-u"]
    assert command[6] == "-s"
    assert kwargs["timeout"] == 10
    assert kwargs["check"] is True
    assert "docker" not in command


def test_sync_timeout_fails_closed(monkeypatch) -> None:
    module = _module()

    def time_out(command, **kwargs):
        raise subprocess.TimeoutExpired(command, kwargs["timeout"])

    monkeypatch.setattr(module.subprocess, "run", time_out)
    with pytest.raises(RuntimeError, match="timed out after 10 seconds"):
        module.sync_container_clock(timeout_seconds=10)


def test_sync_command_failure_includes_colima_error(monkeypatch) -> None:
    module = _module()

    def fail(command, **kwargs):
        raise subprocess.CalledProcessError(1, command, stderr="colima is not running")

    monkeypatch.setattr(module.subprocess, "run", fail)
    with pytest.raises(RuntimeError, match="colima is not running"):
        module.sync_container_clock(timeout_seconds=10)


def test_production_never_sets_clock_through_privileged_docker() -> None:
    roots = (
        PROJECT_ROOT / "src",
        PROJECT_ROOT / "scripts",
        PROJECT_ROOT / "docker",
        PROJECT_ROOT / ".github" / "workflows",
    )
    paths = [PROJECT_ROOT / "justfile"]
    for root in roots:
        paths.extend(path for path in root.rglob("*") if path.is_file())

    violations = []
    pattern = re.compile(r"docker.{0,240}--privileged.{0,240}date.{0,80}-s", re.DOTALL)
    for path in paths:
        text = path.read_text(encoding="utf-8", errors="replace")
        if pattern.search(text):
            violations.append(str(path.relative_to(PROJECT_ROOT)))

    assert not violations, (
        "setting the Colima VM clock from a privileged Docker container can "
        "leave the Docker client blocked after date exits; use the bounded "
        f"shared clock primitive instead: {violations}"
    )
