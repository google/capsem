"""Shared immutable release-input resolution and verification.

The selected manifest is the authority. ``release-inputs.json`` is only a
transport inventory and must exactly reproduce the artifact set derived from
that manifest before any byte can enter a release test harness.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path, PurePosixPath
from typing import Any, Iterable, cast
from urllib.parse import unquote, urljoin, urlparse

import blake3


def safe_component(value: object, label: str) -> str:
    if (
        not isinstance(value, str)
        or not value
        or value in {".", ".."}
        or "/" in value
        or "\\" in value
    ):
        raise ValueError(f"unsafe {label}: {value!r}")
    return value


def safe_relative(value: object, label: str = "release input path") -> Path:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{label} is invalid")
    relative = PurePosixPath(value)
    if relative.is_absolute() or any(part in {"", ".", ".."} for part in relative.parts):
        raise ValueError(f"{label} is unsafe: {value!r}")
    return Path(*relative.parts)


def safe_name(url: str, fallback: str) -> str:
    value = unquote(PurePosixPath(urlparse(url).path).name) or fallback
    return safe_component(value, "release artifact name")


def required_digest(record: dict[str, Any], label: str) -> tuple[str, str, int]:
    digest = record.get("digest")
    if not isinstance(digest, dict):
        raise ValueError(f"{label} has no digest")
    sha256 = digest.get("sha256")
    b3 = digest.get("blake3")
    size = record.get("bytes")
    if (
        not isinstance(sha256, str)
        or len(sha256) != 64
        or not all(character in "0123456789abcdefABCDEF" for character in sha256)
    ):
        raise ValueError(f"{label} has invalid SHA-256")
    if (
        not isinstance(b3, str)
        or len(b3) != 64
        or not all(character in "0123456789abcdefABCDEF" for character in b3)
    ):
        raise ValueError(f"{label} has invalid BLAKE3")
    if not isinstance(size, int) or isinstance(size, bool) or size < 0:
        raise ValueError(f"{label} has invalid byte size")
    return sha256.lower(), b3.lower(), size


def verify_payload(payload: bytes, record: dict[str, Any], label: str) -> None:
    sha256, b3, size = required_digest(record, label)
    if len(payload) != size:
        raise ValueError(f"{label} byte size mismatch: {len(payload)} != {size}")
    if hashlib.sha256(payload).hexdigest() != sha256:
        raise ValueError(f"{label} SHA-256 mismatch")
    if blake3.blake3(payload).hexdigest() != b3:
        raise ValueError(f"{label} BLAKE3 mismatch")


def _artifact_row(
    relative: Path,
    url: str,
    record: dict[str, Any],
    label: str,
) -> dict[str, Any]:
    sha256, b3, size = required_digest(record, label)
    return {
        "relative": relative,
        "url": url,
        "record": record,
        "label": label,
        "bytes": size,
        "sha256": sha256,
        "blake3": b3,
    }


def _package_rows(manifest: dict[str, Any], manifest_url: str) -> Iterable[dict[str, Any]]:
    packages = manifest.get("packages")
    if not isinstance(packages, list) or not packages:
        raise ValueError("release manifest contains no packages")
    selected = [
        package
        for package in packages
        if isinstance(package, dict) and package.get("status") == "current"
    ]
    if not selected:
        raise ValueError("release manifest contains no current packages")
    for index, package in enumerate(selected):
        url = package.get("url")
        if not isinstance(url, str) or not url:
            raise ValueError(f"package[{index}] has no URL")
        absolute = urljoin(manifest_url, url)
        name = safe_name(absolute, f"package-{index}")
        yield _artifact_row(Path(name), absolute, package, f"package {name}")

        package_id = safe_component(package.get("id") or f"package-{index}", "package identity")
        evidence = package.get("evidence", [])
        if not isinstance(evidence, list):
            raise ValueError(f"package {name} evidence is malformed")
        for evidence_index, record in enumerate(evidence):
            if not isinstance(record, dict):
                raise ValueError(f"package {name} evidence[{evidence_index}] is malformed")
            record = cast(dict[str, Any], record)
            if record.get("status") == "revoked":
                continue
            evidence_url = record.get("url")
            if not isinstance(evidence_url, str) or not evidence_url:
                raise ValueError(f"package {name} evidence[{evidence_index}] has no URL")
            evidence_absolute = urljoin(manifest_url, evidence_url)
            evidence_name = safe_name(evidence_absolute, f"evidence-{evidence_index}")
            yield _artifact_row(
                Path("package-evidence") / package_id / evidence_name,
                evidence_absolute,
                record,
                f"package {name} evidence {evidence_name}",
            )


def _profile_rows(
    manifest: dict[str, Any],
    manifest_url: str,
    selected_architecture: str | None = None,
) -> Iterable[dict[str, Any]]:
    profiles = manifest.get("profiles")
    if not isinstance(profiles, dict) or not profiles:
        raise ValueError("release manifest contains no profiles")
    for profile_id_value, profile in sorted(profiles.items()):
        profile_id = safe_component(profile_id_value, "profile identity")
        if not isinstance(profile, dict):
            raise ValueError(f"release manifest profile {profile_id} is malformed")
        profile = cast(dict[str, Any], profile)
        if profile.get("status") == "revoked":
            continue
        architectures = profile.get("architectures")
        if not isinstance(architectures, list) or not architectures:
            raise ValueError(f"profile {profile_id} has no architectures")
        for architecture_record in architectures:
            if not isinstance(architecture_record, dict):
                raise ValueError(f"profile {profile_id} architecture is malformed")
            arch = safe_component(
                architecture_record.get("architecture"),
                f"profile {profile_id} architecture identity",
            )
            if selected_architecture is not None and arch != selected_architecture:
                continue
            for section in ("config", "images", "evidence"):
                records = architecture_record.get(section, [])
                if not isinstance(records, list):
                    raise ValueError(f"profile {profile_id}/{arch} {section} is malformed")
                for index, record in enumerate(records):
                    if not isinstance(record, dict):
                        raise ValueError(
                            f"profile {profile_id}/{arch} {section}[{index}] is malformed"
                        )
                    record = cast(dict[str, Any], record)
                    if record.get("status") == "revoked":
                        continue
                    url = record.get("url")
                    if not isinstance(url, str) or not url:
                        raise ValueError(
                            f"profile {profile_id}/{arch} {section}[{index}] has no URL"
                        )
                    absolute = urljoin(manifest_url, url)
                    label = str(
                        record.get("name")
                        or record.get("path")
                        or record.get("kind")
                        or f"{section}-{index}"
                    )
                    name = safe_name(absolute, f"{section}-{index}")
                    yield _artifact_row(
                        Path("profiles") / profile_id / arch / section / name,
                        absolute,
                        record,
                        f"profile {profile_id}/{arch}/{label}",
                    )


def resolved_artifact_rows(
    manifest: dict[str, Any],
    manifest_url: str,
    kind: str,
    *,
    allow_empty_profiles: bool = False,
    allow_empty_packages: bool = False,
    architecture: str | None = None,
) -> list[dict[str, Any]]:
    if kind not in {"packages", "profiles"}:
        raise ValueError(f"invalid release input kind: {kind!r}")
    if allow_empty_profiles and kind != "profiles":
        raise ValueError("empty release inputs are permitted only for profiles")
    if allow_empty_packages and kind != "packages":
        raise ValueError("empty release inputs are permitted only for packages")
    if architecture is not None:
        architecture = safe_component(architecture, "release input architecture")
        if kind != "profiles":
            raise ValueError("architecture filtering is permitted only for profiles")
    # An absent channel has no public before-state at all: no profiles, and no
    # packages either. Bootstrapping inherits the donor channel's cohort so a
    # new channel's first profile can be proved against shipped binaries, but
    # when that donor has itself been retired the inherited URLs are dead, and
    # a manifest claiming packages nobody can fetch is worse than one claiming
    # none. Empty is only ever accepted when the caller states it explicitly.
    if {"profiles": allow_empty_profiles, "packages": allow_empty_packages}[
        kind
    ] and manifest.get(kind) == {"profiles": {}, "packages": []}[kind]:
        return []
    source = (
        _package_rows(manifest, manifest_url)
        if kind == "packages"
        else _profile_rows(manifest, manifest_url, architecture)
    )
    rows: list[dict[str, Any]] = []
    by_url: dict[str, tuple[str, str, int]] = {}
    by_path: dict[Path, str] = {}
    for row in source:
        identity = (row["sha256"], row["blake3"], row["bytes"])
        previous_identity = by_url.setdefault(row["url"], identity)
        if previous_identity != identity:
            raise ValueError(f"manifest records disagree on immutable input {row['url']}")
        if row["url"] in {existing["url"] for existing in rows}:
            continue
        previous_url = by_path.setdefault(row["relative"], row["url"])
        if previous_url != row["url"]:
            raise ValueError(f"manifest artifacts collide at release input path {row['relative']}")
        rows.append(row)
    if not rows:
        raise ValueError(f"release manifest resolved no {kind}")
    return rows


def report_artifacts(rows: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        {
            "path": row["relative"].as_posix(),
            "url": row["url"],
            "bytes": row["bytes"],
            "sha256": row["sha256"],
            "blake3": row["blake3"],
        }
        for row in rows
    ]


def _load_json_object(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"invalid {label} {path}: {error}") from error
    if not isinstance(value, dict):
        raise ValueError(f"{label} must be a JSON object")
    return value


def load_verified_release_inputs(
    input_dir: Path,
) -> tuple[dict[str, Any], dict[str, Any], dict[str, Any]]:
    report_path = input_dir / "release-inputs.json"
    manifest_path = input_dir / "manifest.json"
    report = _load_json_object(report_path, "release input report")
    if report.get("schema") != "capsem.release_inputs.v1":
        raise ValueError("release input report has an unsupported schema")
    kind = report.get("kind")
    if kind not in {"packages", "profiles"}:
        raise ValueError("release input report has an invalid artifact kind")
    manifest_url = report.get("manifest_url")
    if not isinstance(manifest_url, str) or not manifest_url:
        raise ValueError("release input report lacks its manifest URL")
    manifest = _load_json_object(manifest_path, "resolved manifest")

    allow_empty_profiles = report.get("allow_empty_profiles", False)
    if not isinstance(allow_empty_profiles, bool):
        raise ValueError("release input report has an invalid empty-profile policy")
    allow_empty_packages = report.get("allow_empty_packages", False)
    if not isinstance(allow_empty_packages, bool):
        raise ValueError("release input report has an invalid empty-package policy")
    architecture = report.get("architecture")
    if architecture is not None:
        architecture = safe_component(architecture, "release input report architecture")
        if kind != "profiles":
            raise ValueError("release input report architecture is permitted only for profiles")
    expected_rows = resolved_artifact_rows(
        manifest,
        manifest_url,
        kind,
        allow_empty_profiles=allow_empty_profiles,
        allow_empty_packages=allow_empty_packages,
        architecture=architecture,
    )
    expected = report_artifacts(expected_rows)
    artifacts = report.get("artifacts")
    if artifacts != expected:
        raise ValueError("release input report does not match the resolved manifest artifact set")

    input_root = input_dir.resolve()
    verified: list[str] = []
    for expected_row, resolved in zip(expected, expected_rows, strict=True):
        relative = safe_relative(expected_row["path"])
        path = (input_dir / relative).resolve()
        if path != input_root and input_root not in path.parents:
            raise ValueError(f"release input path escapes its root: {relative}")
        try:
            payload = path.read_bytes()
        except OSError as error:
            raise ValueError(f"read release input {relative}: {error}") from error
        verify_payload(payload, resolved["record"], str(relative))
        verified.append(relative.as_posix())

    verification = {
        "schema": "capsem.release_inputs_verification.v1",
        "ok": True,
        "kind": kind,
        "manifest": str(manifest_path),
        "verified": verified,
    }
    return report, manifest, verification
