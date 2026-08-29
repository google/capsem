"""Rehydrate legacy profile-root bytes under their nested manifest authority."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, cast

import blake3
from capsem_builder.release.tools.release_inputs import safe_relative


def _entries(manifest_path: Path, profile_id: str) -> tuple[tuple[Path, str, int], ...]:
    try:
        document = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise ValueError(f"profile {profile_id} root manifest is unreadable: {error}") from error
    if not isinstance(document, dict) or set(document) != {"format", "files"}:
        raise ValueError(f"profile {profile_id} root manifest has an invalid shape")
    if document["format"] != "capsem.profile-root.v1":
        raise ValueError(f"profile {profile_id} root manifest has an unsupported format")
    files = document["files"]
    if not isinstance(files, list) or not files:
        raise ValueError(f"profile {profile_id} root manifest has no files")

    entries: list[tuple[Path, str, int]] = []
    seen: set[Path] = set()
    for index, row in enumerate(files):
        label = f"profile {profile_id} root manifest file[{index}]"
        if not isinstance(row, dict) or set(row) != {"path", "hash", "size"}:
            raise ValueError(f"{label} has an invalid shape")
        row = cast(dict[str, Any], row)
        relative = safe_relative(row["path"], f"{label} path")
        digest = row["hash"]
        size = row["size"]
        if (
            not isinstance(digest, str)
            or not digest.startswith("blake3:")
            or len(digest) != 71
            or any(character not in "0123456789abcdef" for character in digest[7:])
        ):
            raise ValueError(f"{label} has an invalid BLAKE3 digest")
        if not isinstance(size, int) or isinstance(size, bool) or size <= 0:
            raise ValueError(f"{label} has an invalid byte size")
        if relative in seen:
            raise ValueError(f"profile {profile_id} root manifest repeats {relative}")
        seen.add(relative)
        entries.append((relative, digest[7:], size))
    return tuple(entries)


def _verified_source(root: Path, relative: Path, digest: str, size: int) -> bytes:
    if root.is_symlink() or not root.is_dir():
        raise ValueError(f"legacy profile root source is missing or unsafe: {root}")
    source = root
    for part in relative.parts:
        source /= part
        if source.is_symlink():
            raise ValueError(f"legacy profile root source contains a symlink: {source}")
    if not source.is_file():
        raise ValueError(f"legacy profile root source file is missing: {source}")
    payload = source.read_bytes()
    if len(payload) != size:
        raise ValueError(f"legacy profile root source size mismatch: {relative}")
    if blake3.blake3(payload).hexdigest() != digest:
        raise ValueError(f"legacy profile root source BLAKE3 mismatch: {relative}")
    return payload


def stage_legacy_root(
    shared_config_root: Path,
    config_root: Path,
    profile_id: str,
    staged_paths: set[Path],
) -> None:
    """Fill only an all-legacy root set; current graphs must publish every byte."""
    manifest = Path("profiles") / profile_id / "root.manifest.json"
    if manifest not in staged_paths:
        return
    entries = _entries(config_root / manifest, profile_id)
    root = manifest.parent / "root"
    expected = {root / relative for relative, _digest, _size in entries}
    published = {path for path in staged_paths if root in path.parents}
    if published:
        if published != expected:
            missing = sorted(str(path) for path in expected - published)
            raise ValueError(
                f"profile {profile_id} publishes an incomplete root payload: {missing}"
            )
        return

    source_root = shared_config_root / root
    for relative, digest, size in entries:
        payload = _verified_source(source_root, relative, digest, size)
        destination = config_root / root / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(payload)
