"""Typed native and language toolchain authority from ``config/gate.toml``."""

from __future__ import annotations

from typing import Annotated

from pydantic import StringConstraints, model_validator

from .configschema import Strict


class CrateTool(Strict):
    """A cargo-installed tool: how to find it, and how to get it."""

    name: str
    probe: tuple[str, ...]
    expected: str
    install: tuple[str, ...]

    @model_validator(mode="after")
    def exact_version_is_declared(self) -> CrateTool:
        if not self.probe or self.probe[0] != self.name:
            raise ValueError("Cargo tool probe must start with its configured name")
        if not self.expected:
            raise ValueError("Cargo tool expected version output may not be empty")
        if self.install[:2] != ("cargo", "install"):
            raise ValueError("Cargo tool install must use cargo install")
        if "--version" not in self.install or "--locked" not in self.install:
            raise ValueError("Cargo tool install must carry an exact version and --locked")
        return self


LinuxPackage = Annotated[str, StringConstraints(pattern=r"^[A-Za-z0-9][A-Za-z0-9+._-]*$")]
PkgConfigModule = Annotated[str, StringConstraints(pattern=r"^[A-Za-z0-9][A-Za-z0-9+._-]*$")]
Sha256 = Annotated[str, StringConstraints(pattern=r"^[0-9a-f]{64}$")]


class OrtDistribution(Strict):
    """One digest-authorized ort-sys static distribution."""

    url: str
    sha256: Sha256


class OrtToolchainConfig(Strict):
    """The pre-sandbox ORT materializer shared by every Rust build rail."""

    script: str
    archive_cache_template: str
    output_template: str
    strategy_variable: str
    location_variable: str
    strategy: str
    host_targets: dict[str, dict[str, str]]
    distributions: dict[str, OrtDistribution]

    @model_validator(mode="after")
    def every_host_target_has_a_distribution(self) -> OrtToolchainConfig:
        selected = {
            target
            for architectures in self.host_targets.values()
            for target in architectures.values()
        }
        missing = sorted(selected - set(self.distributions))
        if missing:
            raise ValueError("toolchain.ort host targets are missing: " + ", ".join(missing))
        if "{sha256}" not in self.archive_cache_template:
            raise ValueError("toolchain.ort archive cache must include {sha256}")
        required = ("{consumer}", "{target}", "{sha256}")
        if any(field not in self.output_template for field in required):
            raise ValueError("toolchain.ort output must include {consumer}, {target}, and {sha256}")
        return self


class LinuxWorkspaceConfig(Strict):
    """Native dependencies shared by bootstrap, CI, and the Linux builder."""

    apt_packages: tuple[LinuxPackage, ...]
    cross_dev_packages: tuple[LinuxPackage, ...]
    cross_host_packages: tuple[LinuxPackage, ...]
    dnf_packages: tuple[LinuxPackage, ...]
    pkg_config_modules: tuple[PkgConfigModule, ...]
    required_commands: tuple[LinuxPackage, ...]

    @model_validator(mode="after")
    def inventories_are_nonempty_and_unique(self) -> LinuxWorkspaceConfig:
        for name in (
            "apt_packages",
            "cross_dev_packages",
            "cross_host_packages",
            "dnf_packages",
            "pkg_config_modules",
            "required_commands",
        ):
            values = getattr(self, name)
            if not values or len(values) != len(set(values)):
                raise ValueError(f"toolchain.linux.{name} must be non-empty and unique")
        for name in ("cross_dev_packages", "cross_host_packages"):
            missing = sorted(set(getattr(self, name)) - set(self.apt_packages))
            if missing:
                raise ValueError(
                    f"toolchain.linux.{name} must be installed by apt_packages: "
                    + ", ".join(missing)
                )
        return self


class ToolchainConfig(Strict):
    sync: tuple[str, ...]
    node_workspaces: tuple[str, ...]
    node_install: tuple[str, ...]
    node_env: dict[str, str]
    rust_targets: tuple[str, ...]
    rust_components: tuple[str, ...]
    ort: OrtToolchainConfig
    linux: LinuxWorkspaceConfig
    crates: tuple[CrateTool, ...]
