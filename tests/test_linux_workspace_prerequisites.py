from __future__ import annotations

import importlib.util
import subprocess
import sys
from pathlib import Path

import pytest
from pydantic import ValidationError

from capsem.gate import config as gate_config
from capsem.gate.toolchainschema import LinuxWorkspaceConfig

PROJECT_ROOT = Path(__file__).resolve().parents[1]
PROVISIONER = PROJECT_ROOT / "scripts" / "provision-linux-workspace.py"


def _provisioner():
    spec = importlib.util.spec_from_file_location("provision_linux_workspace", PROVISIONER)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_linux_workspace_prerequisites_are_one_validated_config_value() -> None:
    linux = gate_config.load(PROJECT_ROOT).toolchain.linux

    assert linux.apt_packages == tuple(dict.fromkeys(linux.apt_packages))
    assert linux.cross_dev_packages == tuple(dict.fromkeys(linux.cross_dev_packages))
    assert set(linux.cross_dev_packages) <= set(linux.apt_packages)
    assert linux.cross_host_packages == tuple(dict.fromkeys(linux.cross_host_packages))
    assert set(linux.cross_host_packages) <= set(linux.apt_packages)
    assert "librsvg2-dev" not in linux.cross_dev_packages
    assert linux.dnf_packages == tuple(dict.fromkeys(linux.dnf_packages))
    assert linux.pkg_config_modules == tuple(dict.fromkeys(linux.pkg_config_modules))
    assert {
        "build-essential",
        "pkg-config",
        "libssl-dev",
        "libgtk-3-dev",
        "libwebkit2gtk-4.1-dev",
        "libayatana-appindicator3-dev",
        "libxdo-dev",
        "librsvg2-dev",
        "musl-tools",
    } <= set(linux.apt_packages)
    assert {
        "glib-2.0",
        "gtk+-3.0",
        "webkit2gtk-4.1",
        "ayatana-appindicator3-0.1",
        "openssl",
        "librsvg-2.0",
    } <= set(linux.pkg_config_modules)


def test_cross_dev_packages_must_come_from_the_native_apt_inventory() -> None:
    document = gate_config.load(PROJECT_ROOT).toolchain.linux.model_dump()
    document["cross_dev_packages"] = ["not-installed-dev-package"]

    with pytest.raises(ValidationError, match="cross_dev_packages"):
        LinuxWorkspaceConfig.model_validate(document)


def test_cross_host_packages_must_come_from_the_native_apt_inventory() -> None:
    document = gate_config.load(PROJECT_ROOT).toolchain.linux.model_dump()
    document["cross_host_packages"] = ["not-installed-host-package"]

    with pytest.raises(ValidationError, match="cross_host_packages"):
        LinuxWorkspaceConfig.model_validate(document)


@pytest.mark.parametrize("manager", ["apt", "dnf"])
def test_provisioner_installs_configured_packages_then_proves_pkg_config(
    manager: str, monkeypatch: pytest.MonkeyPatch
) -> None:
    provisioner = _provisioner()
    calls: list[tuple[str, ...]] = []

    def record(argv: list[str], *, check: bool) -> subprocess.CompletedProcess[str]:
        assert check is True
        calls.append(tuple(argv))
        return subprocess.CompletedProcess(argv, 0)

    monkeypatch.setattr(provisioner.subprocess, "run", record)
    monkeypatch.setattr(provisioner.os, "geteuid", lambda: 0)
    monkeypatch.setattr(provisioner.shutil, "which", lambda command: f"/usr/bin/{command}")
    config = gate_config.load(PROJECT_ROOT).toolchain.linux

    provisioner.install_and_verify(
        PROJECT_ROOT / "config/gate.toml", provisioner.PackageManager(manager)
    )

    packages = config.apt_packages if manager == "apt" else config.dnf_packages
    if manager == "apt":
        assert calls[:2] == [
            ("apt-get", "update"),
            ("apt-get", "install", "-y", "--no-install-recommends", *packages),
        ]
    else:
        assert calls[0] == ("dnf", "install", "-y", *packages)
    assert calls[-1] == ("pkg-config", "--exists", *config.pkg_config_modules)


def test_ci_and_host_builder_consume_the_same_prerequisite_authority() -> None:
    ci = (PROJECT_ROOT / ".github/workflows/ci.yaml").read_text(encoding="utf-8")
    fast = (PROJECT_ROOT / ".github/workflows/fast-gate.yaml").read_text(encoding="utf-8")
    host_builder = (PROJECT_ROOT / "docker/Dockerfile.host-builder").read_text(encoding="utf-8")
    hostimage = (PROJECT_ROOT / "src/capsem/gate/hostimage.py").read_text(encoding="utf-8")
    invocation = "sudo python3 scripts/provision-linux-workspace.py --install apt"

    assert invocation in ci
    assert invocation in fast
    assert "WORKSPACE_APT_PACKAGES" in host_builder
    assert "WORKSPACE_APT_PACKAGES" in hostimage
    for package in gate_config.load(PROJECT_ROOT).toolchain.linux.apt_packages:
        assert f"    {package} \\\n" not in host_builder
