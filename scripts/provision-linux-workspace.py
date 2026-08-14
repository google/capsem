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
from enum import StrEnum
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
from capsem.gate.architecture import Arch

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = ROOT / "config/gate.toml"
SAFE_VALUE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9+._-]*$")


class PackageManager(StrEnum):
    APT = "apt"
    DNF = "dnf"


class ProvisionError(RuntimeError):
    pass


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


def _architectures(config_path: Path) -> dict[Arch, tuple[tuple[str, ...], str]]:
    """Load config names into the existing enum and validate their aliases."""
    try:
        document = tomllib.loads(config_path.read_text(encoding="utf-8"))
        raw_architectures = document["architectures"]
    except (OSError, KeyError, tomllib.TOMLDecodeError) as exc:
        raise ProvisionError(f"cannot load architectures from {config_path}: {exc}") from exc
    if not isinstance(raw_architectures, dict) or not raw_architectures:
        raise ProvisionError("architectures must be a non-empty table")

    architectures: dict[Arch, tuple[tuple[str, ...], str]] = {}
    aliases_seen: set[str] = set()
    for name, raw in raw_architectures.items():
        if not isinstance(name, str) or not isinstance(raw, dict):
            raise ProvisionError("every architecture must be a named table")
        target = raw.get("rust_target")
        aliases_raw = raw.get("aliases")
        if (
            not isinstance(target, str)
            or SAFE_VALUE.fullmatch(target) is None
            or not isinstance(aliases_raw, list)
            or not aliases_raw
        ):
            raise ProvisionError(f"architecture {name!r} has invalid aliases or Rust target")
        aliases = tuple(aliases_raw)
        if any(
            not isinstance(alias, str) or SAFE_VALUE.fullmatch(alias) is None for alias in aliases
        ):
            raise ProvisionError(f"architecture {name!r} has an unsafe alias")
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
        architectures[architecture] = normalized, target

    concrete = set(Arch) - {Arch.ANY, Arch.HOST}
    if set(architectures) != concrete:
        raise ProvisionError("architecture config and the Arch enum disagree")
    return architectures


def host_architecture(config_path: Path, machine: str | None = None) -> Arch:
    """Parse the external kernel spelling once into the architecture enum."""
    architectures = _architectures(config_path)

    selected_machine = (machine or platform.machine()).strip().lower()
    hosts = [
        arch for arch, (aliases, _target) in architectures.items() if selected_machine in aliases
    ]
    if len(hosts) != 1:
        raise ProvisionError(f"unsupported host architecture: {selected_machine!r}")
    return hosts[0]


def cross_rust_target(config_path: Path, architecture: Arch) -> str:
    """Return the one configured Rust target foreign to a typed host."""
    if not isinstance(architecture, Arch):
        raise TypeError("architecture must be Arch")
    architectures = _architectures(config_path)
    if architecture not in architectures:
        raise ProvisionError(f"unsupported host architecture: {architecture.name}")
    targets = [
        target for arch, (_aliases, target) in architectures.items() if arch is not architecture
    ]
    if len(targets) != 1:
        raise ProvisionError("architecture config must provide exactly one cross Rust target")
    return targets[0]


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


def install_and_verify(config_path: Path, manager: PackageManager) -> None:
    if not isinstance(manager, PackageManager):
        raise TypeError("manager must be PackageManager")
    if os.geteuid() != 0:
        raise ProvisionError("package installation requires root; invoke this script through sudo")
    selected = packages(config_path, manager)
    if manager is PackageManager.APT:
        subprocess.run(["apt-get", "update"], check=True)
        subprocess.run(
            ["apt-get", "install", "-y", "--no-install-recommends", *selected],
            check=True,
        )
    else:
        subprocess.run(["dnf", "install", "-y", *selected], check=True)
    verify(config_path)


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
