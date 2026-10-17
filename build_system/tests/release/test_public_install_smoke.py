"""The public installer needs a real native supervisor, even in Docker."""

from __future__ import annotations

import shutil
import signal
import subprocess
from pathlib import Path

import pytest
from capsem_builder.gate.proc import Runner
from capsem_builder.release.tools import check_public_binary_release as gate
from capsem_builder.release.tools.public_install_smoke import public_install_container


@pytest.fixture(autouse=True)
def docker_available(monkeypatch) -> None:
    original = shutil.which
    monkeypatch.setattr(shutil, "which", lambda name, *args, **kwargs: (
        "/usr/bin/docker" if name == "docker" else original(name, *args, **kwargs)
    ))


def test_public_install_starts_systemd_before_executing_the_installer(monkeypatch) -> None:
    calls: list[list[str]] = []

    def capture(argv, **_kwargs):
        calls.append(list(argv))
        return subprocess.CompletedProcess(argv, 0, stdout="running\n", stderr="")

    def execute(_runner, command):
        return capture(command.argv)

    monkeypatch.setattr(subprocess, "run", capture)
    monkeypatch.setattr(Runner, "execute", execute)
    gate.run_docker_install_smoke(
        release_base_url="https://release.capsem.org",
        install_script_url="https://capsem.org/install.sh",
        stable_manifest_url="https://release.capsem.org/assets/stable/manifest.json",
        nightly_manifest_url="https://release.capsem.org/assets/nightly/manifest.json",
        expected_version="0.7.0",
        channel_switch=False,
        upgrade=False,
        docker_image="fixture-systemd-install-image",
    )
    boots = [call for call in calls if call[:2] == ["docker", "run"] and "-d" in call]
    assert len(boots) == 1, (
        "A plain bash PID1 cannot register or start the installed user service; "
        "public install proof must boot systemd before running the installer."
    )
    boot = boots[0]
    assert Path(boot[-1]).name == "systemd"
    assert "--privileged" in boot and "--cgroupns=host" in boot
    assert boot[boot.index("--network") + 1] == "bridge"
    assert any("is-system-running" in part for call in calls for part in call)
    install = next(call for call in calls if call[:2] == ["docker", "exec"] and "curl -fsSL" in call[-1])
    assert calls.index(install) > calls.index(boot)
    assert install[install.index("-u") + 1] == "capsem"
    assert calls[-1][:4] == ["docker", "rm", "-f", "-v"]


def test_public_install_interrupt_removes_its_owned_container(monkeypatch) -> None:
    calls: list[tuple[str, ...]] = []

    def execute(_runner, command):
        calls.append(command.argv)
        return subprocess.CompletedProcess(command.argv, 0, stdout="running\n", stderr="")

    monkeypatch.setattr(Runner, "execute", execute)
    previous = signal.getsignal(signal.SIGTERM)
    with (
        pytest.raises(SystemExit, match="public install verification interrupted"),
        public_install_container(gate.PROJECT_ROOT, image="fixture-systemd-install-image"),
    ):
        signal.raise_signal(signal.SIGTERM)
    assert signal.getsignal(signal.SIGTERM) == previous
    assert list(calls[-1][:4]) == ["docker", "rm", "-f", "-v"]
