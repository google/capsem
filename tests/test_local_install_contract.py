"""The local install path builds one complete product and installs that package."""

from __future__ import annotations

import argparse
import importlib
import os
import subprocess
import tomllib
from pathlib import Path

import pytest
from capsem_builder.gate.command import GateCommand
from helpers.gate import RecordingRunner

ROOT = Path(__file__).resolve().parents[1]


def _executable(path: Path, body: str) -> None:
    path.write_text(f"#!/bin/sh\nset -eu\n{body}\n", encoding="utf-8")
    path.chmod(0o755)


def _plan(monkeypatch: pytest.MonkeyPatch):
    importlib.import_module("capsem_builder.gate.cli")
    monkeypatch.setattr("capsem_builder.gate.localinstall.host.on_macos", lambda: True)
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
    assert "scripts/install-local-macos-package.applescript" in rendered
    assert "sudo /usr/sbin/installer" not in rendered
    authorization = (ROOT / "scripts/install-local-macos-package.applescript").read_text(
        encoding="utf-8"
    )
    assert "quoted form of packagePath" in authorization
    assert "quoted form of targetPath" in authorization
    assert "with administrator privileges" in authorization
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))[
        "workspace"
    ]["package"]["version"]
    assert f"packages/Capsem-{version}.pkg" in rendered


def test_local_install_packages_the_verified_base_profile_pair(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    rendered = _plan(monkeypatch).describe()
    verified = ROOT / "target" / "ironbank-assets" / "code"

    assert f"--assets-dir {verified / 'assets'}" in rendered
    assert f"--config-root {verified / 'config'}" in rendered
    assert f"--assets-dir {ROOT / 'assets'}" not in rendered
    assert f"--config-root {ROOT / 'target/config'}" not in rendered


def test_public_install_warns_then_only_dispatches_local_install() -> None:
    lines = (ROOT / "justfile").read_text(encoding="utf-8").splitlines()
    start = lines.index("install:")
    body = []
    for line in lines[start + 1 :]:
        if line and not line[0].isspace():
            break
        body.append(line)

    executable = [line.strip() for line in body if line.strip()]
    assert executable == [
        "@echo \"Agent: optional hands-on local testing only; 'just install' "
        "does not qualify or unblock a release. Dispatch releases directly "
        "with 'just release-binaries ...' or 'just release-profile ...'.\"",
        "uv run capsem-gate local-install",
    ]


def test_native_package_retirement_catches_a_basename_only_owned_service(
    tmp_path: Path,
) -> None:
    """A package PID file owns the stale service even when argv lost its path."""
    # Keep fixture PIDs above Linux's hard PID ceiling so /proc can never
    # redirect the production lookup away from the fake process table.
    service_pid, gateway_pid, unrelated_pid = "9410041", "9410042", "9410099"
    fake_bin = tmp_path / "bin"
    state = tmp_path / "state"
    capsem_home = tmp_path / "home" / ".capsem"
    run_dir = capsem_home / "run"
    fake_bin.mkdir()
    state.mkdir()
    run_dir.mkdir(parents=True)
    for pid in (service_pid, gateway_pid, unrelated_pid):
        (state / pid).touch()
    (run_dir / "service.pid").write_text(f"{service_pid}\n", encoding="utf-8")

    _executable(
        fake_bin / "pgrep",
        f"""case "$4" in
  capsem-service) printf '{service_pid}\\n{unrelated_pid}\\n' ;;
  capsem-gateway) printf '{gateway_pid}\\n' ;;
esac""",
    )
    _executable(
        fake_bin / "ps",
        f"""pid="$2"
[ -e "{state}/$pid" ] || exit 1
field="${{4:-}}"
[ -n "$field" ] || exit 0
case "$pid:$field" in
  {service_pid}:uid=|{gateway_pid}:uid=|{unrelated_pid}:uid=) echo 501 ;;
  {service_pid}:comm=) echo capsem-service ;;
  {gateway_pid}:comm=) echo '{capsem_home}/bin/capsem-gateway' ;;
  {unrelated_pid}:comm=) echo '/tmp/dev/capsem-service' ;;
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

    assert kill_log.read_text(encoding="utf-8").splitlines() == [
        service_pid,
        gateway_pid,
    ]
    assert (state / unrelated_pid).exists(), "an unrelated developer service was killed"
    assert (
        f"retired native helper pid={service_pid} name=capsem-service"
        in completed.stdout
    )


def test_native_package_retirement_fails_closed_when_an_owned_pid_survives(
    tmp_path: Path,
) -> None:
    survivor_pid = "9410041"
    fake_bin = tmp_path / "bin"
    capsem_home = tmp_path / "home" / ".capsem"
    fake_bin.mkdir()
    (capsem_home / "run").mkdir(parents=True)
    _executable(
        fake_bin / "pgrep",
        f'[ "$4" != capsem-service ] || echo {survivor_pid}',
    )
    _executable(
        fake_bin / "ps",
        f"""[ "$2" = {survivor_pid} ]
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
    assert f"native helper cohort did not stop: {survivor_pid}" in completed.stderr


@pytest.mark.parametrize(
    ("cgroup", "inside_service"),
    [
        ("0::/user.slice/user-501.slice/user@501.service/app.slice/capsem.service\n", True),
        (
            "0::/user.slice/user-501.slice/user@501.service/app.slice/"
            "capsem.service/updater\n",
            True,
        ),
        (
            "0::/user.slice/user-501.slice/user@501.service/app.slice/"
            "github-runner.service\n",
            False,
        ),
        ("12:memory:/docker/1234\n", False),
    ],
)
def test_linux_package_detects_only_its_own_service_cgroup(
    tmp_path: Path,
    cgroup: str,
    inside_service: bool,
) -> None:
    cgroup_file = tmp_path / "cgroup"
    cgroup_file.write_text(cgroup, encoding="utf-8")

    completed = subprocess.run(
        [
            "bash",
            "-c",
            'source "$1"; capsem_install_runs_inside_service "$2"',
            "bash",
            str(ROOT / "scripts/pkg-scripts/service-owned-update"),
            str(cgroup_file),
        ],
        text=True,
        capture_output=True,
        check=False,
    )

    assert (completed.returncode == 0) is inside_service


def test_deb_preinstall_preserves_a_service_owned_update_until_activation() -> None:
    preinstall = (ROOT / "scripts/deb-preinst.sh").read_text(encoding="utf-8")

    branch = preinstall[preinstall.index("if capsem_install_runs_inside_service") :]
    preserve, ordinary = branch.split("else", maxsplit=1)
    ordinary, _ = ordinary.rsplit("\nfi", maxsplit=1)

    assert "/proc/self/cgroup" in preserve
    assert "event=preserve_service_owned_update" in preserve
    assert "systemctl --user stop capsem.service" not in preserve
    assert "capsem_retire_native_cohort" not in preserve
    assert "systemctl --user stop capsem.service" in ordinary
    assert "capsem_retire_native_cohort" in ordinary


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
