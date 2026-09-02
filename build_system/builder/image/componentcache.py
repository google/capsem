"""Input-keyed VM component receipts backed by immutable cache objects."""

from __future__ import annotations

import json
import os
import re
import secrets
import stat
from collections.abc import Iterator
from pathlib import Path, PurePosixPath
from typing import Annotated, Any, Literal

import blake3
from pydantic import BaseModel, ConfigDict, StringConstraints, field_validator

from ..cache.config import load_policy
from ..cache.objects import ObjectRef, digest_file, import_file, materialize
from ..cache.paths import CachePaths

TOKEN = re.compile(r"^[a-z][a-z0-9-]*$")
SCHEMA = "capsem.component-cache.v1"


class ComponentReceipt(BaseModel):
    """Exact input identity mapped to immutable output objects."""

    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    schema_id: Literal["capsem.component-cache.v1"]
    component: Annotated[str, StringConstraints(pattern=r"^[a-z][a-z0-9-]*$")]
    input_digest: Annotated[str, StringConstraints(pattern=r"^[0-9a-f]{64}$")]
    files: dict[str, ObjectRef]

    @field_validator("files")
    @classmethod
    def files_are_canonical_relative_paths(
        cls, files: dict[str, ObjectRef]
    ) -> dict[str, ObjectRef]:
        if not files:
            raise ValueError("component receipt must contain an output")
        for relative in files:
            path = PurePosixPath(relative)
            if path.is_absolute() or ".." in path.parts or str(path) != relative:
                raise ValueError(f"component output is not a canonical relative path: {relative}")
        return files


def input_digest(record: dict[str, Any]) -> str:
    encoded = json.dumps(record, sort_keys=True, separators=(",", ":")).encode()
    return blake3.blake3(encoded).hexdigest()


def source_digest(root: Path, relatives: tuple[str, ...]) -> str:
    """Hash declared source trees by relative name, mode, kind, and bytes."""
    digest = blake3.blake3(b"capsem.component-source.v1\0")
    for relative in relatives:
        target = root / relative
        if not target.exists() and not target.is_symlink():
            raise ValueError(f"component source input is missing: {relative}")
        for path in sorted(_source_files(target)):
            digest.update(path.relative_to(root).as_posix().encode())
            mode = path.lstat().st_mode
            digest.update(f"\0{stat.S_IMODE(mode):04o}\0".encode())
            if stat.S_ISLNK(mode):
                digest.update(b"symlink\0")
                digest.update(os.readlink(path).encode())
            elif stat.S_ISREG(mode):
                digest.update(b"file\0")
                with path.open("rb") as stream:
                    while block := stream.read(1024 * 1024):
                        digest.update(block)
            else:
                raise ValueError(f"component source input is not a file or symlink: {path}")
            digest.update(b"\0")
    return digest.hexdigest()


def _source_files(target: Path) -> Iterator[Path]:
    if target.is_symlink() or target.is_file():
        yield target
        return
    for path in target.rglob("*"):
        if (path.is_symlink() or path.is_file()) and "__pycache__" not in path.parts:
            yield path


def build_identity(record: dict[str, Any], *, extra: dict[str, Any] | None = None) -> str:
    """Hash only byte-affecting build inputs, excluding commit and runtime labels."""
    keys = ("arch", "template", "docker_platform", "dockerfile", "build_context", "dependency_image")
    missing = [key for key in keys if key not in record]
    if missing:
        raise ValueError(f"component build identity lacks inputs: {missing}")
    stable = {key: record[key] for key in keys}
    if extra is not None:
        stable["component_config"] = extra
    return input_digest(stable)


def _paths(repository: Path) -> CachePaths:
    return CachePaths(repository_root=repository.resolve(), policy=load_policy(repository))


def _receipt(paths: CachePaths, component: str, identity: str) -> Path:
    if not TOKEN.fullmatch(component) or len(identity) != 64:
        raise ValueError("component cache identity is not canonical")
    return paths.stage("objects") / "components" / component / f"{identity}.json"


def _load_receipt(
    paths: CachePaths, component: str, identity: str
) -> ComponentReceipt | None:
    receipt_path = _receipt(paths, component, identity)
    if not receipt_path.is_file():
        return None
    receipt = ComponentReceipt.model_validate_json(receipt_path.read_text(encoding="utf-8"))
    if (
        receipt.schema_id != SCHEMA
        or receipt.component != component
        or receipt.input_digest != identity
    ):
        raise ValueError(f"component receipt identity mismatch: {receipt_path}")
    return receipt


def current(
    repository: Path,
    component: str,
    identity: str,
    output: Path,
) -> tuple[Path, ...] | None:
    """Return current exact outputs without mutating or trusting timestamps."""
    paths = _paths(repository)
    output_root = output.resolve()
    if not output_root.is_relative_to(paths.root.resolve()):
        return None
    receipt = _load_receipt(paths, component, identity)
    if receipt is None:
        return None
    found: list[Path] = []
    for relative, reference in sorted(receipt.files.items()):
        candidate = output / relative
        if candidate.is_symlink() or not candidate.is_file():
            return None
        if not candidate.resolve().is_relative_to(output_root):
            return None
        metadata = candidate.stat()
        if (
            metadata.st_size != reference.logical_bytes
            or stat.S_IMODE(metadata.st_mode) != reference.mode
            or digest_file(candidate) != reference.digest
        ):
            return None
        found.append(candidate)
    return tuple(found)


def restore(
    repository: Path,
    component: str,
    identity: str,
    output: Path,
) -> tuple[Path, ...] | None:
    """Restore one complete component generation, or report a clean miss."""
    paths = _paths(repository)
    if not output.resolve().is_relative_to(paths.root.resolve()):
        return None
    receipt = _load_receipt(paths, component, identity)
    if receipt is None:
        return None
    restored: list[Path] = []
    for relative, reference in sorted(receipt.files.items()):
        destination = output / relative
        materialize(paths, reference, destination)
        restored.append(destination)
    return tuple(restored)


def store(
    repository: Path,
    component: str,
    identity: str,
    output: Path,
    relatives: tuple[str, ...],
) -> ComponentReceipt:
    """Publish a complete component receipt after importing every output."""
    paths = _paths(repository)
    files = {relative: import_file(paths, output / relative) for relative in relatives}
    receipt = ComponentReceipt(
        schema_id=SCHEMA, component=component, input_digest=identity, files=files
    )
    destination = _receipt(paths, component, identity)
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.{secrets.token_hex(6)}")
    try:
        temporary.write_text(receipt.model_dump_json(indent=2) + "\n", encoding="utf-8")
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)
    return receipt
