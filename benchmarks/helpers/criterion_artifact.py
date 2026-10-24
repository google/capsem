"""Admission of Cargo's pinned metadata/no-run protocol, not another builder.

Package IDs come from Cargo, never reconstructed from path spelling. Exactly
one selected local target must have an executable from the shared Cargo stage.
Warm compiler artifacts are valid; Criterion measurement outputs are always new.
"""

from __future__ import annotations

import stat
from dataclasses import dataclass
from pathlib import Path

from capsem_builder.cache.signedreceipt import FileStamp


@dataclass(frozen=True)
class Target:
    package_id: str
    manifest: str
    target: dict


def local_file(raw: str, root: Path) -> Path:
    path = Path(raw)
    if (
        not path.is_absolute()
        or ".." in path.parts
        or not path.resolve().is_relative_to(root.resolve())
    ):
        raise ValueError("Cargo target file is outside its owned root")
    FileStamp.capture(path)
    return path


def select_target(metadata: dict, root: Path, stage: Path, crate: str, bench: str) -> Target:
    if (
        not Path(metadata["target_directory"]).is_absolute()
        or Path(metadata["target_directory"]).resolve() != stage.resolve()
    ):
        raise ValueError("Cargo metadata does not use the canonical shared target")
    packages = [row for row in metadata["packages"] if row["name"] == crate]
    if len(packages) != 1:
        raise ValueError("Cargo selected package is missing or repeated")
    package = packages[0]
    if package["source"] is not None or package["id"] not in metadata["workspace_members"]:
        raise ValueError("Cargo selected package is not a local workspace member")
    manifest = local_file(package["manifest_path"], root)
    targets = [
        row for row in package["targets"] if row["name"] == bench and row["kind"] == ["bench"]
    ]
    if len(targets) != 1 or targets[0]["crate_types"] != ["bin"]:
        raise ValueError("Cargo selected benchmark target is missing or repeated")
    local_file(targets[0]["src_path"], manifest.parent)
    return Target(package["id"], str(manifest), targets[0])


def select_executable(
    events: list[dict],
    target: Target,
    stage: Path,
    profile: dict,
    *,
    profile_directory: str,
) -> FileStamp:
    selected = []
    finished = False
    profile_root = stage / profile_directory
    if (
        not profile_root.resolve().is_relative_to(stage.resolve())
        or profile_root.resolve() == stage.resolve()
    ):
        raise ValueError("Cargo profile directory escapes the shared stage")
    for event in events:
        reason = event.get("reason")
        if finished:
            raise ValueError("Cargo output continued after build-finished")
        if reason == "build-finished":
            if event.get("success") is not True:
                raise ValueError("Cargo no-run build failed")
            finished = True
        elif reason == "compiler-artifact":
            if event.get("package_id") != target.package_id:
                continue
            if event.get("target", {}).get("name") != target.target["name"]:
                continue
            if (
                event.get("manifest_path") != target.manifest
                or event.get("target") != target.target
            ):
                raise ValueError("Cargo artifact manifest/source/target identity mismatch")
            actual = event.get("profile", {})
            if actual.keys() != profile.keys() or any(
                type(actual[key]) is not type(value) or actual[key] != value
                for key, value in profile.items()
            ):
                raise ValueError("Cargo benchmark artifact has wrong optimization profile")
            raw = event.get("executable")
            if not isinstance(raw, str) or raw not in event.get("filenames", []):
                raise ValueError("Cargo benchmark artifact has no declared executable")
            path = local_file(raw, profile_root)
            stamp = FileStamp.capture(path)
            if not stamp.mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH):
                raise ValueError("Cargo benchmark artifact is not executable")
            selected.append(stamp)
        elif reason not in ("compiler-message", "build-script-executed"):
            raise ValueError("unknown Cargo no-run protocol message")
    if not finished or len(selected) != 1:
        raise ValueError("Cargo no-run artifact coverage is missing or repeated")
    return selected[0]
