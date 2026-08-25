"""The local install path builds one complete product and installs that package."""

from __future__ import annotations

import argparse
import importlib
import os
import subprocess
import tomllib
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate.command import GateCommand

ROOT = Path(__file__).resolve().parents[1]


def _executable(path: Path, body: str) -> None:
    path.write_text(f"#!/bin/sh\nset -eu\n{body}\n", encoding="utf-8")
    path.chmod(0o755)


def _plan(monkeypatch: pytest.MonkeyPatch):
    importlib.import_module("capsem.gate.cli")
    monkeypatch.setattr("capsem.gate.localinstall.host.on_macos", lambda: True)
    return GateCommand.registry["local-install"](
        RecordingRunner(ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
    ).plan()


def test_local_install_builds_content_package_then_installs_that_exact_package(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    plan = _plan(monkeypatch)
    labels = list(plan.labels)
    rendered = plan.describe()

    assert labels.index("assets.assemble") < labels.index("local-install.content")
    assert labels.index("local-install.content") < labels.index("local-install.package")
    assert labels.index("local-install.package") < labels.index("local-install.install")
    assert "scripts/build-test-macos-package.sh" in rendered
    assert "sudo /usr/sbin/installer -pkg" in rendered
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))[
        "workspace"
    ]["package"]["version"]
    assert f"packages/Capsem-{version}.pkg" in rendered


def test_public_install_is_only_the_local_install_dispatch() -> None:
    lines = (ROOT / "justfile").read_text(encoding="utf-8").splitlines()
    start = lines.index("install:")
    body = []
    for line in lines[start + 1 :]:
        if line and not line[0].isspace():
            break
        body.append(line)

    assert "capsem-gate local-install" in "\n".join(body)
    assert sum(bool(line.strip()) for line in body) == 1


def test_native_package_retirement_catches_a_basename_only_owned_service(
    tmp_path: Path,
) -> None:
    """A package PID file owns the stale service even when argv lost its path."""
    fake_bin = tmp_path / "bin"
    state = tmp_path / "state"
    capsem_home = tmp_path / "home" / ".capsem"
    run_dir = capsem_home / "run"
    fake_bin.mkdir()
    state.mkdir()
    run_dir.mkdir(parents=True)
    for pid in (41, 42, 99):
        (state / str(pid)).touch()
    (run_dir / "service.pid").write_text("41\n", encoding="utf-8")

    _executable(
        fake_bin / "pgrep",
        """case "$4" in
  capsem-service) printf '41\\n99\\n' ;;
  capsem-gateway) printf '42\\n' ;;
esac""",
    )
    _executable(
        fake_bin / "ps",
        f"""pid="$2"
[ -e "{state}/$pid" ] || exit 1
field="${{4:-}}"
[ -n "$field" ] || exit 0
case "$pid:$field" in
  41:uid=|42:uid=|99:uid=) echo 501 ;;
  41:comm=) echo capsem-service ;;
  42:comm=) echo '{capsem_home}/bin/capsem-gateway' ;;
  99:comm=) echo '/tmp/dev/capsem-service' ;;
  *) exit 2 ;;
esac""",
    )
    kill_log = tmp_path / "kill.log"
    _executable(
        fake_bin / "kill",
        f"""[ "$1" = "-9" ]
printf '%s\\n' "$2" >> "{kill_log}"
rm -f "{state}/$2"
""",
    )
    _executable(fake_bin / "sleep", ":")

    completed = subprocess.run(
        [
            "bash",
            "-c",
            'source "$1"; capsem_retire_native_cohort "$2" 501',
            "bash",
            str(ROOT / "scripts/pkg-scripts/retire-cohort"),
            str(capsem_home),
        ],
        env={
            **os.environ,
            "PATH": f"{fake_bin}:/usr/bin:/bin",
            "CAPSEM_INSTALL_KILL": str(fake_bin / "kill"),
        },
        text=True,
        capture_output=True,
        check=True,
    )

    assert kill_log.read_text(encoding="utf-8").splitlines() == ["41", "42"]
    assert (state / "99").exists(), "an unrelated developer service was killed"
    assert "retired native helper pid=41 name=capsem-service" in completed.stdout


def test_native_package_retirement_fails_closed_when_an_owned_pid_survives(
    tmp_path: Path,
) -> None:
    fake_bin = tmp_path / "bin"
    capsem_home = tmp_path / "home" / ".capsem"
    fake_bin.mkdir()
    (capsem_home / "run").mkdir(parents=True)
    _executable(fake_bin / "pgrep", "[ \"$4\" != capsem-service ] || echo 41")
    _executable(
        fake_bin / "ps",
        f"""[ "$2" = 41 ]
field="${{4:-}}"
case "$field" in
  "") exit 0 ;;
  uid=) echo 501 ;;
  comm=) echo '{capsem_home}/bin/capsem-service' ;;
  *) exit 2 ;;
esac""",
    )
    _executable(fake_bin / "kill", ":")
    _executable(fake_bin / "sleep", ":")

    completed = subprocess.run(
        [
            "bash",
            "-c",
            'source "$1"; capsem_retire_native_cohort "$2" 501',
            "bash",
            str(ROOT / "scripts/pkg-scripts/retire-cohort"),
            str(capsem_home),
        ],
        env={
            **os.environ,
            "PATH": f"{fake_bin}:/usr/bin:/bin",
            "CAPSEM_INSTALL_KILL": str(fake_bin / "kill"),
        },
        text=True,
        capture_output=True,
        check=False,
    )

    assert completed.returncode != 0
    assert "native helper cohort did not stop: 41" in completed.stderr


def test_public_installer_stops_the_user_service_before_package_replacement() -> None:
    for relative in ("site/public/install.sh", "docs/public/install.sh"):
        installer = (ROOT / relative).read_text(encoding="utf-8")
        stop = installer.index('"$HOME/.capsem/bin/capsem" stop')
        macos = installer[installer.index("install_macos()") : installer.index("install_linux()")]
        linux = installer[installer.index("install_linux()") :]

        assert "sudo" not in installer[stop : installer.index("}", stop)], relative
        assert macos.index("stop_existing_capsem") < macos.index(
            "sudo /usr/sbin/installer -pkg"
        ), relative
        assert linux.index("stop_existing_capsem") < linux.index("sudo apt install"), relative


def test_installed_glowup_owns_the_release_regression_story_matrix() -> None:
    local_glowup = (ROOT / "scripts/local-release-glowup.py").read_text(encoding="utf-8")
    macos_glowup = (ROOT / "scripts/macos_release_glowup.py").read_text(encoding="utf-8")
    tart_host = (ROOT / "scripts/macos_tart_glowup.py").read_text(encoding="utf-8")
    tart_guest = (ROOT / "scripts/macos_tart_guest.sh").read_text(encoding="utf-8")
    tart_regressions = (ROOT / "scripts/macos-tart-regression-probes.sh").read_text(
        encoding="utf-8"
    )
    physical_boot = (ROOT / "scripts/prove-macos-package-boot.sh").read_text(encoding="utf-8")
    native_check = (ROOT / "scripts/check-macos-native-glowup.py").read_text(encoding="utf-8")

    for source in (local_glowup, macos_glowup):
        assert "validate_checked_in_marketing_install_surface" in source
    assert "ASSET_HYDRATION_EVIDENCE" in tart_regressions
    assert '"started"' in tart_regressions
    assert "STALE_HELPER_EVIDENCE" in tart_regressions
    assert "old_service_pid" in tart_regressions
    assert "macos-tart-regression-probes.sh" in tart_host
    assert "PERSISTENT_PIN_EVIDENCE" in physical_boot
    assert "--keep-session" in physical_boot
    assert '"persistent_pin_resume": True' in physical_boot
    assert '"persistent_pin_resume"' in native_check

    # TUI rendering and interaction belong to the dedicated Ratatui suite.
    for source in (
        local_glowup,
        macos_glowup,
        tart_guest,
        tart_host,
        tart_regressions,
        physical_boot,
    ):
        assert "capsem-tui --help" not in source
        assert "CAPSEM_TUI_LATENCY" not in source
