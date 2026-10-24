"""Exact macOS package-family pairing for the existing disposable Tart owner."""

from __future__ import annotations

import json
import re
from pathlib import Path

from ... import crypto
from .package_payload import package_payload_files
from .release_glowup import ArtifactIdentity, PairingIdentity, assert_manifest_artifact


def _sha(path: Path) -> str:
    digest = crypto.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def cohort(
    package: Path,
    manifest: Path,
    sbom: Path,
    *,
    channel: str,
    platform: str = "macos",
    architecture: str = "arm64",
) -> dict:
    document = json.loads(manifest.read_text())
    rows = [row for row in document["packages"] if row["name"] == package.name]
    if len(rows) != 1:
        raise ValueError("paired package must have one manifest descriptor")
    row = rows[0]
    artifact = ArtifactIdentity.from_path(
        package, version=row["version"], platform=platform, architecture=architecture
    )
    try:
        assert_manifest_artifact(document, artifact)
    except RuntimeError as error:
        raise ValueError(str(error)) from error
    source = row.get("source_commit", "")
    if re.fullmatch(r"[0-9a-f]{40}", source) is None:
        raise ValueError("paired package has no exact source commit")
    pairing = PairingIdentity.from_manifest_bytes(
        manifest.read_bytes(), artifact=artifact, channel=channel
    )
    evidence = [item for item in row.get("evidence", []) if item.get("kind") == "sbom"]
    if len(evidence) != 1 or evidence[0].get("status") != "current":
        raise ValueError("paired package must have one current SBOM descriptor")
    sbom_sha = _sha(sbom)
    if (
        evidence[0].get("bytes") != sbom.stat().st_size
        or evidence[0].get("digest", {}).get("sha256") != sbom_sha
    ):
        raise ValueError("paired package SBOM bytes do not match manifest")
    spdx = json.loads(sbom.read_text())
    entries = spdx.get("files", [])
    binaries = row.get("binaries", [])
    names = [item.get("name") for item in binaries]
    if (
        not binaries
        or len(names) != len(set(names))
        or not {"capsem", "capsem-service"}.issubset(names)
    ):
        raise ValueError("paired package has missing or duplicate binary roles")
    payload = package_payload_files(package)
    metadata = [
        value for name, value in payload.items() if name.endswith("/assets/manifest-metadata.json")
    ]
    if len(metadata) != 1:
        raise ValueError("paired package must declare one polling metadata payload")
    packaged_metadata = json.loads(metadata[0])
    if (
        packaged_metadata.get("schema") != "capsem.manifest_metadata.v1"
        or packaged_metadata.get("channel") != channel
        or not isinstance(packaged_metadata.get("manifest_url"), str)
    ):
        raise ValueError("paired package declares invalid polling provenance")
    for binary in binaries:
        name = binary["name"]
        for field, expected in (
            ("status", "current"),
            ("version", row["version"]),
            ("platform", platform),
            ("architecture", architecture),
        ):
            if binary.get(field) != expected:
                raise ValueError(f"paired binary has foreign or inactive {field}")
        if not isinstance(name, str) or re.fullmatch(r"capsem(?:-[a-z0-9]+)*", name) is None:
            raise ValueError("paired package declares an invalid binary role")
        references = [
            item for item in entries if item.get("SPDXID") == binary.get("sbom_component_ref")
        ]
        if len(references) != 1 or references[0].get("fileName") != binary.get("installed_path"):
            raise ValueError("paired binary has no exact SBOM component reference")
        content = payload.get(binary["installed_path"])
        checksums = [
            item.get("checksumValue")
            for item in references[0].get("checksums", [])
            if item.get("algorithm") == "SHA256"
        ]
        digest = binary.get("digest", {}).get("sha256")
        if (
            content is None
            or len(content) != binary.get("bytes")
            or checksums != [digest]
            or crypto.sha256(content).hexdigest() != digest
        ):
            raise ValueError("paired binary payload/SBOM/manifest digest mismatch")
    return {
        "artifact": artifact.as_report(),
        "source_commit": source,
        "sbom_sha256": sbom_sha,
        "manifest_sha256": pairing.manifest_sha256,
        "runtime_sha256": pairing.runtime_sha256,
        "metadata_manifest_url": packaged_metadata["manifest_url"],
        "binaries": [name for name in names if name != "capsem-app"],
    }


def validate_pair(
    *,
    before: tuple[Path, Path, Path],
    after: tuple[Path, Path, Path],
    channel: str,
    platform: str = "macos",
    architecture: str = "arm64",
) -> dict:
    previous = cohort(*before, channel=channel, platform=platform, architecture=architecture)
    candidate = cohort(*after, channel=channel, platform=platform, architecture=architecture)
    versions = [item["artifact"]["version"] for item in (previous, candidate)]
    # This fixture exercises the ordinary Apply route, which selects a newer
    # version. Keep the finite pairing subset stable-only; changed source bytes
    # alone do not make the installed updater select a same-version package.
    stable = r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
    if any(re.fullmatch(stable, value) is None for value in versions) or tuple(
        map(int, versions[1].split("."))
    ) <= tuple(map(int, versions[0].split("."))):
        raise ValueError("binary Apply pair requires a newer stable semver candidate")
    if previous["runtime_sha256"] != candidate["runtime_sha256"]:
        raise ValueError("binary Apply pair must use the identical verified runtime")
    if (
        previous["artifact"]["sha256"] == candidate["artifact"]["sha256"]
        or previous["source_commit"] == candidate["source_commit"]
    ):
        raise ValueError("binary Apply pair must identify distinct package and source bytes")
    return {
        "schema": "capsem.macos_binary_apply_pair.v1",
        "scope": "binary_apply_only",
        "channel": channel,
        "before": previous,
        "after": candidate,
    }
