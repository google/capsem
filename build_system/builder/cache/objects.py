"""Digest-verified immutable objects and hardlink-first materialized views."""

from __future__ import annotations

import os
import secrets
import shutil
import stat
from pathlib import Path
from typing import Annotated

import blake3
from pydantic import BaseModel, ConfigDict, Field, StrictInt, StringConstraints

from .paths import CachePaths


class ObjectRef(BaseModel):
    """Stable identity and filesystem metadata for one immutable file."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    digest: Annotated[str, StringConstraints(pattern=r"^[0-9a-f]{64}$")]
    logical_bytes: Annotated[StrictInt, Field(ge=0)]
    mode: Annotated[StrictInt, Field(ge=0, le=0o7777)]


def digest_file(path: Path) -> str:
    digest = blake3.blake3()
    with path.open("rb") as stream:
        while block := stream.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def object_path(paths: CachePaths, reference: ObjectRef) -> Path:
    return paths.stage("objects") / "blake3" / reference.digest[:2] / reference.digest


def verify(paths: CachePaths, reference: ObjectRef) -> Path:
    payload = object_path(paths, reference)
    if not payload.is_file() or payload.stat().st_size != reference.logical_bytes:
        raise ValueError(f"cache object is missing or has wrong size: {reference.digest}")
    if digest_file(payload) != reference.digest:
        raise ValueError(f"cache object digest mismatch: {reference.digest}")
    return payload


def import_file(paths: CachePaths, source: Path) -> ObjectRef:
    """Import bytes once, preferring a same-filesystem hardlink."""
    if source.is_symlink() or not source.is_file():
        raise ValueError(f"cache object source must be a regular file: {source}")
    reference = ObjectRef(
        digest=digest_file(source),
        logical_bytes=source.stat().st_size,
        mode=stat.S_IMODE(source.stat().st_mode) & ~0o222,
    )
    payload = object_path(paths, reference)
    payload.parent.mkdir(parents=True, exist_ok=True)
    if payload.exists():
        verify(paths, reference)
        return reference
    temporary = payload.with_name(f".{payload.name}.{secrets.token_hex(6)}")
    try:
        try:
            os.link(source, temporary)
        except OSError:
            shutil.copyfile(source, temporary)
        temporary.chmod(reference.mode)
        temporary.replace(payload)
    finally:
        temporary.unlink(missing_ok=True)
    try:
        verify(paths, reference)
    except ValueError:
        payload.unlink(missing_ok=True)
        raise
    return reference


def materialize(paths: CachePaths, reference: ObjectRef, destination: Path) -> None:
    """Atomically install a verified object as a hardlink-first view."""
    payload = verify(paths, reference)
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.{secrets.token_hex(6)}")
    try:
        try:
            os.link(payload, temporary)
        except OSError:
            shutil.copyfile(payload, temporary)
        temporary.chmod(reference.mode)
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)
