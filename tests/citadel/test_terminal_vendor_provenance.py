"""Pinned terminal patches remain an explicit, reviewable upstream delta."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
VENDOR = ROOT / "third_party" / "rust"
PROVENANCE_RATIONALE = (
    "The maintainer approved narrow terminal checkpoint patches, not an opaque "
    "fork. Preserve upstream hashes/licenses and account for every local byte; "
    "an unrecorded change or file must fail before release qualification."
)


def violations(root: Path) -> list[str]:
    upstream_path = root / "UPSTREAM.json"
    patch_path = root / "CAPSEM-PATCH.json"
    if not upstream_path.is_file() or not patch_path.is_file():
        return ["missing upstream or patch provenance"]
    upstream = json.loads(upstream_path.read_text())
    patch = json.loads(patch_path.read_text())
    source_files = upstream["files"]
    changes = patch["files"]
    expected = {**source_files, **changes}
    errors = []
    for name, digest in expected.items():
        path = Path(name)
        if path.is_absolute() or ".." in path.parts:
            errors.append(f"unsafe provenance path: {name}")
            continue
        full = root / path
        if full.is_symlink() or not full.is_file():
            errors.append(f"missing or redirected file: {name}")
        elif hashlib.sha256(full.read_bytes()).hexdigest() != digest:
            errors.append(f"unrecorded change: {name}")
        if name in changes and digest == source_files.get(name):
            errors.append(f"stale patch entry: {name}")
    inventory = {
        str(path.relative_to(root))
        for path in root.rglob("*")
        if path.is_file() or path.is_symlink()
    }
    extras = inventory - expected.keys() - {"UPSTREAM.json", "CAPSEM-PATCH.json"}
    errors.extend(f"unrecorded file: {name}" for name in sorted(extras))
    if patch["upstream_package_sha256"] != upstream["package_sha256"]:
        errors.append("patch targets a different upstream archive")
    if not patch["reason"].strip():
        errors.append("patch lacks a maintenance reason")
    if not any(name.startswith("LICENSE") for name in source_files):
        errors.append("upstream license missing from provenance")
    return errors


@pytest.mark.parametrize("crate", ["vte", "alacritty_terminal"])
def test_terminal_patch_matches_reviewed_source_and_retains_upstream_provenance(crate: str) -> None:
    assert not violations(VENDOR / crate), PROVENANCE_RATIONALE


def fixture(root: Path) -> Path:
    root.mkdir()
    (root / "LICENSE").write_text("upstream license")
    (root / "source.rs").write_text("upstream")
    def digest(name: str) -> str:
        return hashlib.sha256((root / name).read_bytes()).hexdigest()

    (root / "UPSTREAM.json").write_text(json.dumps({
        "package_sha256": "a" * 64,
        "files": {"LICENSE": digest("LICENSE"), "source.rs": digest("source.rs")},
    }))
    (root / "source.rs").write_text("narrow patch")
    (root / "CAPSEM-PATCH.json").write_text(json.dumps({
        "upstream_package_sha256": "a" * 64,
        "reason": "checkpoint interface",
        "files": {"source.rs": digest("source.rs")},
    }))
    return root


def test_guard_accepts_only_accounted_local_delta(tmp_path: Path) -> None:
    root = fixture(tmp_path / "vendor")
    assert not violations(root)
    (root / "source.rs").write_text("unreviewed mutation")
    assert "unrecorded change: source.rs" in violations(root)


def test_guard_refuses_unrecorded_files_and_redirected_sources(tmp_path: Path) -> None:
    root = fixture(tmp_path / "vendor")
    (root / "extra.rs").write_text("unreviewed")
    assert "unrecorded file: extra.rs" in violations(root)
    (root / "source.rs").unlink()
    (root / "source.rs").symlink_to(root / "LICENSE")
    assert "missing or redirected file: source.rs" in violations(root)


@pytest.mark.parametrize("path", ["../outside", "/absolute"])
def test_guard_refuses_escaping_manifest_entries(tmp_path: Path, path: str) -> None:
    root = fixture(tmp_path / "vendor")
    manifest = root / "CAPSEM-PATCH.json"
    data = json.loads(manifest.read_text())
    data["files"][path] = "0" * 64
    manifest.write_text(json.dumps(data))
    assert f"unsafe provenance path: {path}" in violations(root)
