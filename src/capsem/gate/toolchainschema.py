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


class LinuxWorkspaceConfig(Strict):
    """Native dependencies shared by bootstrap, CI, and the Linux builder."""

    apt_packages: tuple[LinuxPackage, ...]
    dnf_packages: tuple[LinuxPackage, ...]
    pkg_config_modules: tuple[PkgConfigModule, ...]
    required_commands: tuple[LinuxPackage, ...]

    @model_validator(mode="after")
    def inventories_are_nonempty_and_unique(self) -> LinuxWorkspaceConfig:
        for name in (
            "apt_packages",
            "dnf_packages",
            "pkg_config_modules",
            "required_commands",
        ):
            values = getattr(self, name)
            if not values or len(values) != len(set(values)):
                raise ValueError(f"toolchain.linux.{name} must be non-empty and unique")
        return self


class ToolchainConfig(Strict):
    sync: tuple[str, ...]
    node_workspaces: tuple[str, ...]
    node_install: tuple[str, ...]
    node_env: dict[str, str]
    rust_targets: tuple[str, ...]
    rust_components: tuple[str, ...]
    linux: LinuxWorkspaceConfig
    crates: tuple[CrateTool, ...]
