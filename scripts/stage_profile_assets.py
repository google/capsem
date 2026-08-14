"""Stage one architecture's verified profile assets for sealed qualification."""

from __future__ import annotations

import shutil
import sys
import tomllib
from pathlib import Path
from typing import Any, cast
from urllib.parse import unquote, urlparse

import tomli_w
from release_inputs import safe_component, safe_relative


def hash_filename(logical_name: str, digest: str) -> str:
    prefix = digest[:16]
    if "." in logical_name:
        stem, extension = logical_name.split(".", 1)
        return f"{stem}-{prefix}.{extension}"
    return f"{logical_name}-{prefix}"


def active_profile_architectures(
    manifest: dict[str, Any], arch: str
) -> list[tuple[str, bool, dict[str, Any]]]:
    profiles = manifest.get("profiles")
    if not isinstance(profiles, dict) or not profiles:
        raise ValueError("release manifest contains no profiles")
    selected: list[tuple[str, bool, dict[str, Any]]] = []
    for profile_id_value, profile in sorted(profiles.items()):
        profile_id = safe_component(profile_id_value, "profile identity")
        if not isinstance(profile, dict):
            raise ValueError(f"release profile {profile_id} is malformed")
        profile = cast(dict[str, Any], profile)
        if profile.get("status") == "revoked":
            continue
        legacy = "source_commit" not in profile
        if not legacy:
            source_commit = profile["source_commit"]
            if (
                not isinstance(source_commit, str)
                or len(source_commit) != 40
                or any(char not in "0123456789abcdef" for char in source_commit)
            ):
                raise ValueError(f"release profile {profile_id} has malformed source_commit")
        architectures = [
            candidate
            for candidate in profile.get("architectures", [])
            if isinstance(candidate, dict) and candidate.get("architecture") == arch
        ]
        if len(architectures) != 1:
            raise ValueError(
                f"release profile {profile_id} must have exactly one {arch} architecture"
            )
        selected.append((profile_id, legacy, architectures[0]))
    if not selected:
        raise ValueError(f"release manifest contains no active {arch} profiles")
    selected.sort(key=lambda row: (row[0] != "code", row[0]))
    return selected


def scope_profile_to_arch(path: Path, arch: str, profile_id: str) -> None:
    """Describe only the architecture the selected release inputs staged."""
    document = tomllib.loads(path.read_text(encoding="utf-8"))
    architectures = document.get("assets", {}).get("arch")
    if not isinstance(architectures, dict):
        return
    if arch not in architectures:
        raise ValueError(f"staged profile {profile_id} declares no {arch} assets to materialize")
    if set(architectures) == {arch}:
        return
    document["assets"]["arch"] = {arch: architectures[arch]}
    path.write_text(tomli_w.dumps(document), encoding="utf-8")


#: File kinds that are meaningless without their lock, and the lock that
#: completes each. The same pairs the Rust profile contract enforces;
#: `tests/citadel/test_profile_pairs_agree.py` holds the two sides to one set,
#: because they had already drifted -- npm was paired there and absent here.
PAIRED_FILES = (
    ("python_requirements", "python_requirements_lock"),
    ("npm_packages", "npm_package_lock"),
)


def require_paired_files(files: dict[str, Any], profile_id: str, *, legacy: bool) -> list[str]:
    """Allow the published legacy bridge, but refuse new unlocked profiles."""
    unpaired: list[str] = [
        f"{listing} without {lock}"
        for listing, lock in PAIRED_FILES
        if (listing in files) != (lock in files)
    ]
    if unpaired and not legacy:
        raise ValueError(f"release profile {profile_id} declares {', '.join(unpaired)}")
    for problem in unpaired:
        print(
            f"WARNING: release profile {profile_id} declares {problem}. "
            "An unlocked dependency list is an unsealed resolver; republish "
            "the profile so future source-stamped profiles remain sealed.",
            file=sys.stderr,
        )
    return unpaired


def require_profile_file_closure(
    path: Path,
    profile_id: str,
    staged_paths: set[Path],
    *,
    legacy: bool,
) -> None:
    """Require every file named by the selected profile to be manifest-owned.

    The profile document and its siblings are one release input. Nothing is
    filled from the checkout here; a missing sibling means the selected release
    graph is incomplete and must be republished.

    Only source-commit-absent legacy profiles may warn about a missing lock.
    """
    try:
        document = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ValueError(f"release profile {profile_id} is unreadable: {error}") from error
    files = document.get("files")
    if files is None:
        return
    if not isinstance(files, dict):
        raise ValueError(f"release profile {profile_id} files must be a table")

    require_paired_files(files, profile_id, legacy=legacy)

    declared: set[Path] = set()
    for kind, record in files.items():
        if not isinstance(kind, str) or not isinstance(record, dict):
            raise ValueError(f"release profile {profile_id} has malformed file {kind!r}")
        relative = safe_relative(record.get("path"), f"release profile {profile_id} {kind} path")
        if len(relative.parts) < 3 or relative.parts[:2] != ("profiles", profile_id):
            raise ValueError(
                f"release profile {profile_id} {kind} path escapes its profile: {relative}"
            )
        if relative in declared:
            raise ValueError(f"release profile {profile_id} repeats file path {relative}")
        declared.add(relative)

    missing = sorted(str(relative) for relative in declared - staged_paths)
    if missing:
        raise ValueError(f"release profile {profile_id} lacks manifest-owned files: {missing}")


def finalize_profile(
    path: Path,
    arch: str,
    profile_id: str,
    staged_paths: set[Path],
    *,
    legacy: bool,
) -> None:
    """Validate one manifest-owned profile cohort, then select its architecture."""
    require_profile_file_closure(path, profile_id, staged_paths, legacy=legacy)
    scope_profile_to_arch(path, arch, profile_id)


def configured_evidence_artifacts(shared_config_root: Path) -> dict[str, str]:
    """Map manifest evidence kinds to config-owned runtime filenames."""
    gate_config = shared_config_root / "gate.toml"
    try:
        document = tomllib.loads(gate_config.read_text(encoding="utf-8"))
        configured = document["assets"]["evidence_artifacts"]
    except (OSError, tomllib.TOMLDecodeError, KeyError, TypeError) as error:
        raise ValueError(f"read configured asset evidence from {gate_config}: {error}") from error
    if not isinstance(configured, list) or not configured:
        raise ValueError("assets.evidence_artifacts must be a non-empty list")
    by_kind: dict[str, str] = {}
    for value in configured:
        logical_name = safe_component(value, "configured evidence artifact")
        kind = logical_name.split(".", 1)[0].replace("-", "_")
        if kind in by_kind:
            raise ValueError(f"configured evidence artifacts repeat manifest kind {kind}")
        by_kind[kind] = logical_name
    return by_kind


def local_file(url: object, label: str) -> Path:
    if not isinstance(url, str):
        raise ValueError(f"{label} lacks a staged URL")
    parsed = urlparse(url)
    if parsed.scheme != "file":
        raise ValueError(f"{label} was not resolved to a local immutable input")
    return Path(unquote(parsed.path))


def _stage_file(
    arch_dir: Path,
    logical_name: str,
    digest: str,
    source: Path,
    *,
    alias: bool,
) -> None:
    hashed = arch_dir / hash_filename(logical_name, digest)
    if not hashed.exists():
        shutil.copy2(source, hashed)
    elif hashed.read_bytes() != source.read_bytes():
        raise ValueError(f"release profiles collide at immutable asset {hashed.name}")
    if alias:
        shutil.copy2(source, arch_dir / logical_name)


def stage_profile_architecture_assets(
    architecture: dict[str, Any],
    *,
    profile_id: str,
    profile_index: int,
    arch: str,
    arch_dir: Path,
    evidence_artifacts: dict[str, str],
) -> None:
    """Stage boot images and the complete config-owned evidence closure."""
    image_names = {"kernel": "vmlinuz", "initrd": "initrd.img", "rootfs": "rootfs.erofs"}
    images = architecture.get("images")
    if not isinstance(images, list):
        raise ValueError(f"release profile {profile_id}/{arch} images are malformed")
    staged_images: set[str] = set()
    for index, value in enumerate(images):
        if not isinstance(value, dict):
            raise ValueError(f"release profile {profile_id}/{arch} image[{index}] is malformed")
        record = cast(dict[str, Any], value)
        if record.get("status") == "revoked" or record.get("kind") not in image_names:
            continue
        kind = cast(str, record["kind"])
        if kind in staged_images:
            raise ValueError(f"release profile {profile_id}/{arch} repeats {kind} image")
        staged_images.add(kind)
        logical_name = safe_component(
            record.get("name") or image_names[kind],
            f"profile {profile_id}/{arch} {kind} image name",
        )
        digest = record.get("digest", {}).get("blake3")
        if not isinstance(digest, str):
            raise ValueError(f"release profile {profile_id}/{arch} {kind} lacks BLAKE3")
        _stage_file(
            arch_dir,
            logical_name,
            digest,
            local_file(record.get("url"), f"profile {profile_id}/{arch} image[{index}]"),
            alias=profile_index == 0,
        )
    missing_images = set(image_names) - staged_images
    if missing_images:
        raise ValueError(
            f"release profile {profile_id}/{arch} lacks images: {sorted(missing_images)}"
        )

    evidence = architecture.get("evidence")
    if not isinstance(evidence, list):
        raise ValueError(f"release profile {profile_id}/{arch} evidence is malformed")
    staged_evidence: set[str] = set()
    for index, value in enumerate(evidence):
        if not isinstance(value, dict):
            raise ValueError(f"release profile {profile_id}/{arch} evidence[{index}] is malformed")
        record = cast(dict[str, Any], value)
        if record.get("status") == "revoked":
            continue
        kind = record.get("kind")
        logical_name = evidence_artifacts.get(kind) if isinstance(kind, str) else None
        if logical_name is None:
            continue
        if logical_name in staged_evidence:
            raise ValueError(f"release profile {profile_id}/{arch} repeats {logical_name}")
        staged_evidence.add(logical_name)
        digest = record.get("digest", {}).get("blake3")
        if not isinstance(digest, str):
            raise ValueError(f"release profile {profile_id}/{arch} {logical_name} lacks BLAKE3")
        _stage_file(
            arch_dir,
            logical_name,
            digest,
            local_file(record.get("url"), f"profile {profile_id}/{arch} evidence[{index}]"),
            alias=profile_index == 0,
        )
    missing_evidence = set(evidence_artifacts.values()) - staged_evidence
    if missing_evidence:
        raise ValueError(
            f"release profile {profile_id}/{arch} lacks configured evidence: "
            f"{sorted(missing_evidence)}"
        )
