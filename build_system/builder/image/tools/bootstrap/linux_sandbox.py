"""Prove the configured Linux kernel sandbox and narrowly repair hosted Ubuntu."""

from __future__ import annotations

import argparse
import os
import socket
import subprocess
import sys
import tomllib
from collections.abc import Callable, Mapping
from pathlib import Path
from typing import Protocol

from capsem_builder.gate import project_root

LAUNCHER = project_root() / "scripts/prepare-linux-sandbox.py"

class PreparationError(RuntimeError):
    """The configured kernel boundary could not be established."""


class CommandRunner(Protocol):
    def __call__(
        self,
        argv: tuple[str, ...],
        *,
        capture_output: bool,
        text: bool,
        check: bool,
    ) -> subprocess.CompletedProcess[str]: ...


def _run(
    argv: tuple[str, ...],
    *,
    capture_output: bool,
    text: bool,
    check: bool,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(argv, capture_output=capture_output, text=text, check=check)


def _settings(root: Path) -> dict:
    config_path = root / "config" / "gate.toml"
    return tomllib.loads(config_path.read_text(encoding="utf-8"))["sandbox"]


def _probe_command(root: Path, settings: dict) -> tuple[str, ...]:
    return (
        settings["linux_command"],
        *settings["linux_args"],
        "--",
        sys.executable,
        str(LAUNCHER),
        "--probe-child",
        "--root",
        str(root),
    )


def _probe_child(root: Path) -> None:
    settings = _settings(root)
    interfaces = {name for _index, name in socket.if_nameindex()}
    if interfaces != {"lo"}:
        raise PreparationError(
            f"sandbox exposes interfaces {sorted(interfaces)!r}, expected only loopback"
        )

    device = Path(settings["linux_device_mount"]) / "null"
    with device.open("wb") as sink:
        sink.write(b"")

    timeout = float(settings["linux_probe_timeout_seconds"])
    loopback = str(settings["linux_probe_loopback_host"])
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.settimeout(timeout)
        listener.bind((loopback, 0))
        listener.listen(1)
        with socket.create_connection(listener.getsockname(), timeout=timeout):
            connection, _address = listener.accept()
            connection.close()

    egress = (
        str(settings["linux_probe_egress_host"]),
        int(settings["linux_probe_egress_port"]),
    )
    try:
        connection = socket.create_connection(egress, timeout=timeout)
    except OSError:
        return
    connection.close()
    raise PreparationError(f"sandbox unexpectedly reached configured egress probe {egress}")


def _read_restriction(name: str) -> str:
    path = Path("/proc/sys") / Path(*name.split("."))
    return path.read_text(encoding="utf-8").strip()


def _failure(result: subprocess.CompletedProcess[str]) -> str:
    detail = (result.stderr or result.stdout).strip()
    return detail or f"probe exited {result.returncode} without output"


def prepare(
    root: Path,
    *,
    allow_hosted_repair: bool = False,
    environment: Mapping[str, str] = os.environ,
    run: CommandRunner = _run,
    read_restriction: Callable[[str], str] = _read_restriction,
) -> None:
    """Prove the boundary, with one fail-closed repair for ephemeral GitHub Ubuntu."""
    settings = _settings(root)
    probe = _probe_command(root, settings)
    first = run(probe, capture_output=True, text=True, check=False)
    if first.returncode == 0:
        return

    marker = str(settings["linux_hosted_failure_marker"])
    hosted = (
        environment.get("GITHUB_ACTIONS") == "true"
        and environment.get("RUNNER_ENVIRONMENT") == "github-hosted"
    )
    if not allow_hosted_repair or marker not in _failure(first) or not hosted:
        raise PreparationError(f"Linux sandbox probe failed: {_failure(first)}")

    sysctl = str(settings["linux_hosted_userns_sysctl"])
    required = str(settings["linux_hosted_userns_required_value"])
    if read_restriction(sysctl) != required:
        raise PreparationError(
            f"known hosted failure occurred without {sysctl}={required}; refusing repair"
        )
    repair = (
        *settings["linux_hosted_repair_command"],
        f"{sysctl}={settings['linux_hosted_userns_repair_value']}",
    )
    repaired = run(repair, capture_output=True, text=True, check=False)
    if repaired.returncode != 0:
        raise PreparationError(f"hosted sandbox repair failed: {_failure(repaired)}")

    second = run(probe, capture_output=True, text=True, check=False)
    if second.returncode != 0:
        raise PreparationError(
            f"Linux sandbox still fails after the narrow hosted repair: {_failure(second)}"
        )


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repair-hosted-runner", action="store_true")
    parser.add_argument("--probe-child", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--root", type=Path, default=project_root())
    args = parser.parse_args(argv)
    try:
        if args.probe_child:
            _probe_child(args.root)
        else:
            prepare(args.root, allow_hosted_repair=args.repair_hosted_runner)
    except (OSError, KeyError, PreparationError, tomllib.TOMLDecodeError) as error:
        print(f"Linux sandbox preparation failed: {error}", file=sys.stderr)
        return 1
    print("Linux Bubblewrap boundary: loopback only, direct egress denied")
    return 0
