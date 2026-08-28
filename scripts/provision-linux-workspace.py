#!/usr/bin/env python3
"""Install and prove the config-owned native Linux workspace prerequisites."""

from __future__ import annotations

import argparse
import os
import platform
import re
import shutil
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
from capsem_builder.gate.architecture import Arch

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = ROOT / "config/gate.toml"
SAFE_VALUE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9+._-]*$")


class PackageManager(StrEnum):
    APT = "apt"
    DNF = "dnf"


class ProvisionError(RuntimeError):
    pass


@dataclass(frozen=True)
class ArchitectureSettings:
    aliases: tuple[str, ...]
    rust_target: str
    gnu: str
    apt_cross_compilers: tuple[str, ...]


def _strings(table: dict[str, Any], name: str) -> tuple[str, ...]:
    raw = table.get(name)
    if not isinstance(raw, list) or not raw:
        raise ProvisionError(f"toolchain.linux.{name} must be a non-empty array")
    values = tuple(raw)
    if any(not isinstance(value, str) or SAFE_VALUE.fullmatch(value) is None for value in values):
        raise ProvisionError(f"toolchain.linux.{name} contains an unsafe value")
    if len(values) != len(set(values)):
        raise ProvisionError(f"toolchain.linux.{name} contains a duplicate")
    return values


def _settings(config_path: Path) -> dict[str, tuple[str, ...]]:
    try:
        document = tomllib.loads(config_path.read_text(encoding="utf-8"))
        table = document["toolchain"]["linux"]
    except (OSError, KeyError, tomllib.TOMLDecodeError) as exc:
        raise ProvisionError(f"cannot load Linux prerequisites from {config_path}: {exc}") from exc
    if not isinstance(table, dict):
        raise ProvisionError("toolchain.linux must be a table")
    return {
        name: _strings(table, name)
        for name in (
            "apt_packages",
            "dnf_packages",
            "pkg_config_modules",
            "required_commands",
        )
    }


def packages(config_path: Path, manager: PackageManager) -> tuple[str, ...]:
    if not isinstance(manager, PackageManager):
        raise TypeError("manager must be PackageManager")
    key = "apt_packages" if manager is PackageManager.APT else "dnf_packages"
    return _settings(config_path)[key]


def _architectures(config_path: Path) -> dict[Arch, ArchitectureSettings]:
    """Load config names into the existing enum and validate their aliases."""
    try:
        document = tomllib.loads(config_path.read_text(encoding="utf-8"))
        raw_architectures = document["architectures"]
    except (OSError, KeyError, tomllib.TOMLDecodeError) as exc:
        raise ProvisionError(f"cannot load architectures from {config_path}: {exc}") from exc
    if not isinstance(raw_architectures, dict) or not raw_architectures:
        raise ProvisionError("architectures must be a non-empty table")

    architectures: dict[Arch, ArchitectureSettings] = {}
    aliases_seen: set[str] = set()
    for name, raw in raw_architectures.items():
        if not isinstance(name, str) or not isinstance(raw, dict):
            raise ProvisionError("every architecture must be a named table")
        target = raw.get("rust_target")
        gnu = raw.get("gnu")
        aliases_raw = raw.get("aliases")
        compilers_raw = raw.get("apt_cross_compilers")
        if (
            not isinstance(target, str)
            or SAFE_VALUE.fullmatch(target) is None
            or not isinstance(gnu, str)
            or SAFE_VALUE.fullmatch(gnu) is None
            or not isinstance(aliases_raw, list)
            or not aliases_raw
            or not isinstance(compilers_raw, list)
            or not compilers_raw
        ):
            raise ProvisionError(f"architecture {name!r} has invalid toolchain settings")
        aliases = tuple(aliases_raw)
        compilers = tuple(compilers_raw)
        if any(
            not isinstance(alias, str) or SAFE_VALUE.fullmatch(alias) is None for alias in aliases
        ):
            raise ProvisionError(f"architecture {name!r} has an unsafe alias")
        if any(
            not isinstance(package, str) or SAFE_VALUE.fullmatch(package) is None
            for package in compilers
        ):
            raise ProvisionError(f"architecture {name!r} has an unsafe cross compiler package")
        if len(compilers) != len(set(compilers)):
            raise ProvisionError(f"architecture {name!r} has duplicate cross compiler packages")
        normalized = tuple(alias.lower() for alias in aliases)
        if len(normalized) != len(set(normalized)) or aliases_seen.intersection(normalized):
            raise ProvisionError("architecture aliases must be globally unique")
        aliases_seen.update(normalized)
        try:
            architecture = Arch[name.upper()]
        except KeyError:
            raise ProvisionError(f"architecture {name!r} has no Arch enum member") from None
        if architecture in (Arch.ANY, Arch.HOST) or architecture in architectures:
            raise ProvisionError(f"architecture {name!r} is not a concrete unique Arch member")
        architectures[architecture] = ArchitectureSettings(
            aliases=normalized,
            rust_target=target,
            gnu=gnu,
            apt_cross_compilers=compilers,
        )

    concrete = set(Arch) - {Arch.ANY, Arch.HOST}
    if set(architectures) != concrete:
        raise ProvisionError("architecture config and the Arch enum disagree")
    return architectures


def host_architecture(config_path: Path, machine: str | None = None) -> Arch:
    """Parse the external kernel spelling once into the architecture enum."""
    architectures = _architectures(config_path)

    selected_machine = (machine or platform.machine()).strip().lower()
    hosts = [
        arch for arch, settings in architectures.items() if selected_machine in settings.aliases
    ]
    if len(hosts) != 1:
        raise ProvisionError(f"unsupported host architecture: {selected_machine!r}")
    return hosts[0]


def cross_rust_target(config_path: Path, architecture: Arch) -> str:
    """Return the one configured Rust target foreign to a typed host."""
    return _foreign_architecture(config_path, architecture).rust_target


def _foreign_architecture(config_path: Path, architecture: Arch) -> ArchitectureSettings:
    """Resolve the one non-host architecture without accepting stringly keys."""
    if not isinstance(architecture, Arch):
        raise TypeError("architecture must be Arch")
    architectures = _architectures(config_path)
    if architecture not in architectures:
        raise ProvisionError(f"unsupported host architecture: {architecture.name}")
    targets = [settings for arch, settings in architectures.items() if arch is not architecture]
    if len(targets) != 1:
        raise ProvisionError("architecture config must provide exactly one cross architecture")
    return targets[0]


def cross_compiler_packages(config_path: Path, architecture: Arch) -> tuple[str, ...]:
    """Return the config-owned APT compilers for the foreign architecture."""
    return _foreign_architecture(config_path, architecture).apt_cross_compilers


def cross_compiler_command(config_path: Path, architecture: Arch) -> str:
    """Return the foreign architecture's GNU C compiler command."""
    return f"{_foreign_architecture(config_path, architecture).gnu}-gcc"


def verify(config_path: Path) -> None:
    settings = _settings(config_path)
    missing = tuple(
        command for command in settings["required_commands"] if shutil.which(command) is None
    )
    if missing:
        raise ProvisionError("missing Linux workspace commands: " + ", ".join(missing))
    subprocess.run(
        ["pkg-config", "--exists", *settings["pkg_config_modules"]],
        check=True,
    )


def install_and_verify(
    config_path: Path,
    manager: PackageManager,
    *,
    architecture: Arch | None = None,
) -> None:
    if not isinstance(manager, PackageManager):
        raise TypeError("manager must be PackageManager")
    if os.geteuid() != 0:
        raise ProvisionError("package installation requires root; invoke this script through sudo")
    selected_architecture = architecture or host_architecture(config_path)
    if not isinstance(selected_architecture, Arch):
        raise TypeError("architecture must be Arch")
    selected = packages(config_path, manager)
    if manager is PackageManager.APT:
        selected += cross_compiler_packages(config_path, selected_architecture)
        subprocess.run(["apt-get", "update"], check=True)
        subprocess.run(
            ["apt-get", "install", "-y", "--no-install-recommends", *selected],
            check=True,
        )
    else:
        subprocess.run(["dnf", "install", "-y", *selected], check=True)
    verify(config_path)
    if manager is PackageManager.APT:
        compiler = cross_compiler_command(config_path, selected_architecture)
        if shutil.which(compiler) is None:
            raise ProvisionError(f"missing Linux cross compiler command: {compiler}")


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--install", type=PackageManager, choices=tuple(PackageManager))
    action.add_argument("--packages", type=PackageManager, choices=tuple(PackageManager))
    action.add_argument("--verify", action="store_true")
    action.add_argument("--cross-rust-target", action="store_true")
    return parser


def main() -> int:
    args = _parser().parse_args()
    try:
        if args.install is not None:
            install_and_verify(args.config, args.install)
        elif args.packages is not None:
            print("\n".join(packages(args.config, args.packages)))
        elif args.verify:
            verify(args.config)
        else:
            print(cross_rust_target(args.config, host_architecture(args.config)))
    except (ProvisionError, subprocess.CalledProcessError) as exc:
        print(f"Linux workspace provisioning failed: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
