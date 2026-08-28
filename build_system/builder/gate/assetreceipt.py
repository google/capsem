"""Exact byte authority for a reusable profile/architecture asset lane."""

from __future__ import annotations

import json
import math
import os
import stat
import time
from pathlib import Path
from typing import cast

from ..release.obom import validate_exported_rootfs_obom
from .config import Arch, GateConfig
from .errors import GateError
from .filesystem import digest_of, write_text

SCHEMA = "capsem.asset-lane-receipt.v2"
BUILD_STAGE = "build"
PACKED_STAGE = "packed"
REUSABLE_STAGES = frozenset({BUILD_STAGE, PACKED_STAGE})
PACKED_STAGES = frozenset({PACKED_STAGE})


def _path(config: GateConfig, output: Path) -> Path:
    return output / config.assets.lane_receipt


def _location_matches(
    config: GateConfig,
    output: Path,
    identity: str,
    *,
    profile: str,
    architecture: str,
) -> bool:
    if len(identity) != 64 or identity.strip("0123456789abcdef") or not profile or Path(profile).name != profile:
        return False
    cached = (
        Path(config.prefix.vm_image_cache.format(parent=config.prefix.parent)).expanduser()
        / identity
        / profile
        / f"build-{architecture}"
    )
    local = config.path(config.assets.test_root) / profile / f"build-{architecture}"
    resolved = output.resolve()
    if resolved == cached.resolve():
        return True
    # Resolving `local` again would bless a malicious selector outside both roots.
    local_output = Path(os.path.abspath(local))
    return not output.is_symlink() and Path(os.path.abspath(output)) == local_output


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
    if not _location_matches(
        config,
        output,
        identity,
        profile=profile,
        architecture=arch.name,
    ):
        raise GateError(f"asset lane output {output} is outside its content-addressed cache path")
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


def _size(document: dict[str, object]) -> int:
    files = document["files"]
    if not isinstance(files, dict):
        raise GateError("asset lane receipt files must be an object")
    total = 0
    for entry in files.values():
        if not isinstance(entry, dict):
            continue
        fields = cast(dict[str, object], entry)
        size = fields.get("size")
        if (
            fields.get("kind") == "file"
            and isinstance(size, int)
            and not isinstance(size, bool)
        ):
            total += size
    return total


def _with_cache_metadata(document: dict[str, object], *, now: float) -> dict[str, object]:
    return {
        **document,
        "created_at": now,
        "last_used_at": now,
        "size_bytes": _size(document),
    }


def _split(document: object) -> tuple[dict[str, object], float, float, int]:
    if not isinstance(document, dict):
        raise GateError("asset lane receipt must be an object")
    stable = dict(document)
    try:
        created = stable.pop("created_at")
        last_used = stable.pop("last_used_at")
        size = stable.pop("size_bytes")
    except KeyError as error:
        raise GateError("asset lane receipt omits cache metadata") from error
    if (
        not isinstance(created, (int, float))
        or isinstance(created, bool)
        or not isinstance(last_used, (int, float))
        or isinstance(last_used, bool)
        or not isinstance(size, int)
        or isinstance(size, bool)
        or created < 0
        or last_used < created
        or size < 0
    ):
        raise GateError("asset lane receipt cache metadata is invalid")
    try:
        created_at = float(created)
        used_at = float(last_used)
    except OverflowError as error:
        raise GateError("asset lane receipt cache timestamps overflow") from error
    if not math.isfinite(created_at) or not math.isfinite(used_at):
        raise GateError("asset lane receipt cache timestamps must be finite")
    return stable, created_at, used_at, size


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
    document = _with_cache_metadata(
        _document(config, output, identity, profile=profile, arch=arch, stage=stage),
        now=time.time(),
    )
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
    touch: bool = False,
) -> bool:
    """Whether exact receipt, identity, stage, and every output node agree."""
    receipt = _path(config, output)
    if not output.is_dir() or receipt.is_symlink() or not receipt.is_file():
        return False
    try:
        document = json.loads(receipt.read_text(encoding="utf-8"))
        stable, created, last_used, size = _split(document)
        stage = stable.get("stage")
        if not isinstance(stage, str) or stage not in stages:
            return False
        expected = _document(
            config,
            output,
            identity,
            profile=profile,
            arch=arch,
            stage=stage,
        )
        now = time.time()
        if (
            stable != expected
            or created > now
            or last_used > now
            or size != _size(expected)
            or size > config.assets.cache.maximum_bytes
            or now - created > config.assets.cache.maximum_age_hours * 3600
        ):
            return False
        if touch:
            document["last_used_at"] = now
            write_text(
                receipt,
                json.dumps(document, sort_keys=True, separators=(",", ":")) + "\n",
            )
        return True
    except (OSError, UnicodeError, json.JSONDecodeError, GateError):
        return False


def cache_metadata(config: GateConfig, output: Path) -> tuple[float, float, int] | None:
    """Return trustworthy timing plus measured bytes for retention planning."""
    receipt = _path(config, output)
    if receipt.is_symlink() or not receipt.is_file():
        return None
    try:
        document = json.loads(receipt.read_text(encoding="utf-8"))
        stable, created, last_used, recorded_size = _split(document)
        now = time.time()
        if stable.get("schema") != SCHEMA or created > now or last_used > now:
            return None
        architecture = stable.get("architecture")
        identity = stable.get("input_digest")
        profile = stable.get("profile")
        if (
            not isinstance(architecture, str)
            or not isinstance(identity, str)
            or not isinstance(profile, str)
            or not _location_matches(
                config,
                output,
                identity,
                profile=profile,
                architecture=architecture,
            )
        ):
            return None
        measured = sum(
            path.lstat().st_size
            for path in (output / architecture).rglob("*")
            if path.is_file() and not path.is_symlink()
        )
        if recorded_size != measured:
            return None
        return created, last_used, measured
    except (OSError, UnicodeError, json.JSONDecodeError, GateError):
        return None
