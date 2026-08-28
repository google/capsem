from __future__ import annotations

import importlib.util
import subprocess
import sys
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import hostimage
from capsem_builder.gate.configschema import Arch as ArchConfig
from capsem_builder.gate.toolchainschema import LinuxWorkspaceConfig
from pydantic import ValidationError

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
    config = gate_config.load(PROJECT_ROOT)
    linux = config.toolchain.linux

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
    assert {arch.rust_target for arch in config.architectures.values()} <= set(
        config.toolchain.rust_targets
    )
    assert {
        package for arch in config.architectures.values() for package in arch.apt_cross_compilers
    } == {
        "gcc-aarch64-linux-gnu",
        "g++-aarch64-linux-gnu",
        "gcc-x86-64-linux-gnu",
        "g++-x86-64-linux-gnu",
    }


def test_cross_rust_target_requires_the_existing_architecture_enum() -> None:
    provisioner = _provisioner()

    assert (
        provisioner.cross_rust_target(PROJECT_ROOT / "config/gate.toml", provisioner.Arch.X86_64)
        == "aarch64-unknown-linux-gnu"
    )
    assert (
        provisioner.cross_rust_target(PROJECT_ROOT / "config/gate.toml", provisioner.Arch.ARM64)
        == "x86_64-unknown-linux-gnu"
    )
    assert provisioner.cross_rust_target.__annotations__["architecture"] == "Arch"


def test_cross_compiler_packages_require_the_existing_architecture_enum() -> None:
    provisioner = _provisioner()
    config_path = PROJECT_ROOT / "config/gate.toml"

    assert provisioner.cross_compiler_packages(config_path, provisioner.Arch.X86_64) == (
        "gcc-aarch64-linux-gnu",
        "g++-aarch64-linux-gnu",
    )
    assert provisioner.cross_compiler_packages(config_path, provisioner.Arch.ARM64) == (
        "gcc-x86-64-linux-gnu",
        "g++-x86-64-linux-gnu",
    )
    assert provisioner.cross_compiler_command(config_path, provisioner.Arch.ARM64) == (
        "x86_64-linux-gnu-gcc"
    )
    assert provisioner.cross_compiler_packages.__annotations__["architecture"] == "Arch"
    with pytest.raises(TypeError, match="architecture must be Arch"):
        provisioner.cross_compiler_packages(config_path, "arm64")


def test_cross_compiler_packages_are_safe_config_tokens() -> None:
    document = gate_config.load(PROJECT_ROOT).arch("arm64").model_dump()
    document["apt_cross_compilers"] = ["gcc-aarch64-linux-gnu;touch"]

    with pytest.raises(ValidationError, match="apt_cross_compilers"):
        ArchConfig.model_validate(document)


def test_cross_compiler_authority_changes_the_host_builder_identity() -> None:
    config = gate_config.load(PROJECT_ROOT)
    architectures = dict(config.architectures)
    architectures["arm64"] = architectures["arm64"].model_copy(
        update={"apt_cross_compilers": ("gcc-aarch64-linux-gnu-new",)}
    )

    changed = config.model_copy(update={"architectures": architectures})

    assert hostimage.input_key(changed) != hostimage.input_key(config)


@pytest.mark.parametrize(
    ("machine", "expected"),
    [("x86_64", "X86_64"), ("amd64", "X86_64"), ("aarch64", "ARM64"), ("arm64", "ARM64")],
)
def test_external_machine_spelling_is_parsed_once(machine: str, expected: str) -> None:
    provisioner = _provisioner()

    assert (
        provisioner.host_architecture(PROJECT_ROOT / "config/gate.toml", machine).name == expected
    )


def test_cross_rust_target_refuses_unknown_or_ambiguous_architecture_config(tmp_path: Path) -> None:
    provisioner = _provisioner()
    unknown = tmp_path / "unknown.toml"
    unknown.write_text(
        '[architectures.arm64]\nrust_target = "aarch64-unknown-linux-gnu"\n'
        'gnu = "aarch64-linux-gnu"\n'
        'apt_cross_compilers = ["gcc-aarch64-linux-gnu"]\n'
        'aliases = ["arm64", "aarch64"]\n',
        encoding="utf-8",
    )
    ambiguous = tmp_path / "ambiguous.toml"
    ambiguous.write_text(
        '[architectures.arm64]\nrust_target = "aarch64-unknown-linux-gnu"\n'
        'gnu = "aarch64-linux-gnu"\n'
        'apt_cross_compilers = ["gcc-aarch64-linux-gnu"]\n'
        'aliases = ["arm64", "aarch64"]\n'
        '[architectures.x86_64]\nrust_target = "x86_64-unknown-linux-gnu"\n'
        'gnu = "x86_64-linux-gnu"\n'
        'apt_cross_compilers = ["gcc-x86-64-linux-gnu"]\n'
        'aliases = ["x86_64", "amd64"]\n'
        '[architectures.third]\nrust_target = "third-unknown-linux-gnu"\n'
        'gnu = "third-linux-gnu"\n'
        'apt_cross_compilers = ["gcc-third-linux-gnu"]\n'
        'aliases = ["third"]\n',
        encoding="utf-8",
    )

    with pytest.raises(provisioner.ProvisionError, match="architecture config and the Arch enum"):
        provisioner.host_architecture(unknown, "x86_64")
    with pytest.raises(provisioner.ProvisionError, match="has no Arch enum member"):
        provisioner.host_architecture(ambiguous, "x86_64")


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
    probed: list[str] = []

    def present(command: str) -> str:
        probed.append(command)
        return f"/usr/bin/{command}"

    monkeypatch.setattr(provisioner.shutil, "which", present)
    config = gate_config.load(PROJECT_ROOT).toolchain.linux

    provisioner.install_and_verify(
        PROJECT_ROOT / "config/gate.toml",
        provisioner.PackageManager(manager),
        architecture=provisioner.Arch.ARM64,
    )

    packages = config.dnf_packages
    if manager == "apt":
        packages = (
            *config.apt_packages,
            "gcc-x86-64-linux-gnu",
            "g++-x86-64-linux-gnu",
        )
        assert calls[:2] == [
            ("apt-get", "update"),
            ("apt-get", "install", "-y", "--no-install-recommends", *packages),
        ]
        assert "x86_64-linux-gnu-gcc" in probed
    else:
        assert calls[0] == ("dnf", "install", "-y", *packages)
    assert calls[-1] == ("pkg-config", "--exists", *config.pkg_config_modules)


def test_ci_and_host_builder_consume_the_same_prerequisite_authority() -> None:
    ci = (PROJECT_ROOT / ".github/workflows/ci.yaml").read_text(encoding="utf-8")
    fast = (PROJECT_ROOT / ".github/workflows/fast-gate.yaml").read_text(encoding="utf-8")
    host_builder = (PROJECT_ROOT / "docker/Dockerfile.host-builder").read_text(encoding="utf-8")
    hostimage = (PROJECT_ROOT / "build_system/builder/gate/hostimage.py").read_text(encoding="utf-8")
    invocation = "sudo python3 scripts/provision-linux-workspace.py --install apt"

    assert invocation in ci
    assert invocation in fast
    assert "WORKSPACE_APT_PACKAGES" in host_builder
    assert "WORKSPACE_APT_PACKAGES" in hostimage
    assert "WORKSPACE_CROSS_APT_PACKAGES" in host_builder
    assert "WORKSPACE_CROSS_APT_PACKAGES" in hostimage
    for package in (
        "gcc-x86-64-linux-gnu",
        "g++-x86-64-linux-gnu",
        "gcc-aarch64-linux-gnu",
        "g++-aarch64-linux-gnu",
    ):
        assert package not in host_builder
    for package in gate_config.load(PROJECT_ROOT).toolchain.linux.apt_packages:
        assert f"    {package} \\\n" not in host_builder
