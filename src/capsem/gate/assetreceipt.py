"""Exact byte authority for a reusable profile/architecture asset lane."""

from __future__ import annotations

import json
import os
import stat
from pathlib import Path

from capsem.obom import validate_exported_rootfs_obom

from .config import Arch, GateConfig
from .errors import GateError
from .filesystem import digest_of, write_text

SCHEMA = "capsem.asset-lane-receipt.v1"
BUILD_STAGE = "build"
PACKED_STAGE = "packed"
REUSABLE_STAGES = frozenset({BUILD_STAGE, PACKED_STAGE})
PACKED_STAGES = frozenset({PACKED_STAGE})


def _path(config: GateConfig, output: Path) -> Path:
    return output / config.assets.lane_receipt


def _required(config: GateConfig) -> tuple[str, ...]:
    return (*config.artifacts.bootable, *config.assets.evidence_artifacts)


def _snapshot(produced: Path) -> dict[str, dict[str, object]]:
    entries: dict[str, dict[str, object]] = {}
    for path in sorted(produced.rglob("*")):
        mode = path.lstat().st_mode
        relative = path.relative_to(produced).as_posix()
        permissions = f"{stat.S_IMODE(mode):04o}"
        if stat.S_ISLNK(mode):
            entries[relative] = {
                "kind": "symlink",
                "mode": permissions,
                "target": os.readlink(path),
            }
        elif stat.S_ISREG(mode):
            entries[relative] = {
                "kind": "file",
                "mode": permissions,
                "size": path.stat().st_size,
                "blake3": digest_of(path, algorithm="blake3"),
            }
        elif not stat.S_ISDIR(mode):
            raise GateError(f"asset lane output {path} is not a file, directory, or symlink")
    return entries


def _document(
    config: GateConfig,
    output: Path,
    identity: str,
    *,
    profile: str,
    arch: Arch,
    stage: str,
) -> dict[str, object]:
    if stage not in REUSABLE_STAGES:
        raise GateError(f"unknown asset lane receipt stage {stage!r}")
    produced = output / arch.name
    missing = [
        name
        for name in _required(config)
        if (produced / name).is_symlink()
        or not (produced / name).is_file()
        or (produced / name).stat().st_size == 0
    ]
    if missing:
        raise GateError(
            "asset build did not produce non-empty regular files "
            + ", ".join(str(produced / name) for name in missing)
        )
    try:
        validate_exported_rootfs_obom(
            produced / config.assets.obom_artifact,
            architecture=arch.name,
        )
    except (OSError, UnicodeError, RuntimeError) as error:
        raise GateError(
            f"asset lane produced an invalid exported-rootfs OBOM for {profile}/{arch.name}: "
            f"{error}"
        ) from error
    return {
        "schema": SCHEMA,
        "profile": profile,
        "architecture": arch.name,
        "stage": stage,
        "input_digest": identity,
        "files": _snapshot(produced),
    }


def record(
    config: GateConfig,
    output: Path,
    identity: str,
    *,
    profile: str,
    arch: Arch,
    stage: str,
) -> None:
    """Atomically bind exact output bytes to their source identity and stage."""
    document = _document(config, output, identity, profile=profile, arch=arch, stage=stage)
    write_text(
        _path(config, output),
        json.dumps(document, sort_keys=True, separators=(",", ":")) + "\n",
    )


def validates(
    config: GateConfig,
    output: Path,
    identity: str,
    *,
    profile: str,
    arch: Arch,
    stages: frozenset[str] = REUSABLE_STAGES,
) -> bool:
    """Whether exact receipt, identity, stage, and every output node agree."""
    receipt = _path(config, output)
    if output.is_symlink() or not output.is_dir() or receipt.is_symlink() or not receipt.is_file():
        return False
    try:
        document = json.loads(receipt.read_text(encoding="utf-8"))
        stage = document.get("stage") if isinstance(document, dict) else None
        if not isinstance(stage, str) or stage not in stages:
            return False
        return document == _document(
            config,
            output,
            identity,
            profile=profile,
            arch=arch,
            stage=stage,
        )
    except (OSError, UnicodeError, json.JSONDecodeError, GateError):
        return False
