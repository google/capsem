"""Bounded Docker command execution and storage-report parsing."""

from __future__ import annotations

import json
import re
import subprocess
from dataclasses import dataclass
from typing import Any

SIZE_UNITS = {
    "B": 1,
    "KB": 1000,
    "MB": 1000**2,
    "GB": 1000**3,
    "TB": 1000**4,
    "KIB": 1024,
    "MIB": 1024**2,
    "GIB": 1024**3,
    "TIB": 1024**4,
}


@dataclass(frozen=True)
class CommandResult:
    command: list[str]
    returncode: int
    stdout: str
    stderr: str

    @property
    def output(self) -> str:
        return "\n".join(part for part in (self.stdout, self.stderr) if part).strip()


def run_command(command: list[str], *, timeout: int = 120) -> CommandResult:
    try:
        result = subprocess.run(
            command,
            check=False,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as error:
        stdout = error.stdout if isinstance(error.stdout, str) else ""
        stderr = error.stderr if isinstance(error.stderr, str) else ""
        return CommandResult(command, 124, stdout, f"{stderr}\ncommand timed out")
    except FileNotFoundError as error:
        return CommandResult(command, 127, "", str(error))
    return CommandResult(
        command,
        result.returncode,
        (result.stdout or "").strip(),
        (result.stderr or "").strip(),
    )


def run_text(command: list[str]) -> str:
    return run_command(command).output


def parse_size_bytes(value: str) -> int:
    token = value.strip().split()[0].replace(",", "")
    match = re.fullmatch(r"([0-9]+(?:\.[0-9]+)?)([A-Za-z]+)", token)
    if not match:
        raise ValueError(f"unsupported Docker size: {value!r}")
    number, unit = match.groups()
    multiplier = SIZE_UNITS.get(unit.upper())
    if multiplier is None:
        raise ValueError(f"unsupported Docker size unit: {unit!r}")
    return int(float(number) * multiplier)


def parse_system_df(output: str) -> dict[str, dict[str, Any]]:
    rows: dict[str, dict[str, Any]] = {}
    for line in output.splitlines():
        if not line.strip():
            continue
        value = json.loads(line)
        name = re.sub(r"[^a-z0-9]+", "_", value["Type"].lower()).strip("_")
        rows[name] = {
            "count": int(value["TotalCount"]),
            "active": int(value["Active"]),
            "size_bytes": parse_size_bytes(value["Size"]),
            "reclaimable_bytes": parse_size_bytes(value["Reclaimable"]),
        }
    return rows


def parse_volume_sizes(output: str) -> dict[str, dict[str, int]]:
    rows: dict[str, dict[str, int]] = {}
    in_volumes = False
    for line in output.splitlines():
        if line == "Local Volumes space usage:":
            in_volumes = True
            continue
        if in_volumes and line == "Build cache usage:":
            break
        if not in_volumes or not line or line.startswith("VOLUME NAME"):
            continue
        match = re.match(r"^(\S+)\s+(\d+)\s+(\S+)$", line)
        if not match:
            continue
        name, links, size = match.groups()
        rows[name] = {"links": int(links), "size_bytes": parse_size_bytes(size)}
    return rows
