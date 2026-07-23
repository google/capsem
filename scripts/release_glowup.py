#!/usr/bin/env python3
"""Platform-neutral contracts for native release glow-up adapters.

Operating-system adapters own only execution: Docker/systemd on Linux and
Tart/launchd on macOS.  Candidate identity, manifest coherence, installed
health, and the durable evidence schema live here so both adapters prove the
same release properties.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Mapping


REPORT_SCHEMA = "capsem.release_glowup.v1"


class GlowupContractError(RuntimeError):
    """The candidate failed a platform-neutral release invariant."""


class ArtifactIdentity:
    """Content and release-graph identity for the exact native package."""

    __slots__ = (
        "path",
        "name",
        "version",
        "platform",
        "architecture",
        "bytes",
        "sha256",
    )

    def __init__(
        self,
        *,
        path: Path,
        name: str,
        version: str,
        platform: str,
        architecture: str,
        bytes: int,
        sha256: str,
    ) -> None:
        self.path = path
        self.name = name
        self.version = version
        self.platform = platform
        self.architecture = architecture
        self.bytes = bytes
        self.sha256 = sha256

    @classmethod
    def from_path(
        cls,
        path: Path,
        *,
        version: str,
        platform: str,
        architecture: str,
    ) -> ArtifactIdentity:
        path = path.resolve()
        if not path.is_file():
            raise GlowupContractError(f"candidate package is missing: {path}")
        size = path.stat().st_size
        if size <= 0:
            raise GlowupContractError(f"candidate package is empty: {path}")
        digest = hashlib.sha256()
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
        return cls(
            path=path,
            name=path.name,
            version=version,
            platform=platform,
            architecture=normalize_architecture(architecture),
            bytes=size,
            sha256=digest.hexdigest(),
        )

    def as_report(self) -> dict[str, object]:
        return {
            "name": self.name,
            "version": self.version,
            "platform": self.platform,
            "architecture": self.architecture,
            "bytes": self.bytes,
            "sha256": self.sha256,
        }


def normalize_architecture(value: str) -> str:
    normalized = {
        "aarch64": "arm64",
        "arm64": "arm64",
        "amd64": "amd64",
        "x86_64": "amd64",
    }.get(value)
    if normalized is None:
        raise GlowupContractError(f"unsupported package architecture: {value}")
    return normalized


def assert_manifest_artifact(
    manifest: Mapping[str, object],
    artifact: ArtifactIdentity,
) -> Mapping[str, object]:
    """Require one current release record to describe the exact package bytes."""

    packages = manifest.get("packages")
    if not isinstance(packages, list):
        raise GlowupContractError("candidate manifest packages must be an array")
    matches = [
        package
        for package in packages
        if isinstance(package, dict)
        and package.get("name") == artifact.name
        and package.get("platform") == artifact.platform
        and package.get("architecture") == artifact.architecture
    ]
    if len(matches) != 1:
        raise GlowupContractError(
            "candidate manifest must contain exactly one package record for "
            f"{artifact.name} ({artifact.platform}/{artifact.architecture}); "
            f"found {len(matches)}"
        )
    package = matches[0]
    expected = {
        "name": artifact.name,
        "version": artifact.version,
        "platform": artifact.platform,
        "architecture": artifact.architecture,
        "bytes": artifact.bytes,
        "status": "current",
    }
    for field, value in expected.items():
        if package.get(field) != value:
            raise GlowupContractError(
                f"candidate manifest package {field} is {package.get(field)!r}, "
                f"expected {value!r}"
            )
    digest = package.get("digest")
    actual_sha256 = digest.get("sha256") if isinstance(digest, dict) else None
    if actual_sha256 != artifact.sha256:
        raise GlowupContractError(
            f"candidate manifest package sha256 is {actual_sha256!r}, "
            f"expected {artifact.sha256!r}"
        )
    return package


def validate_installed_evidence(
    evidence: Mapping[str, object],
) -> Mapping[str, object]:
    """Validate normalized install health without knowing the host OS."""

    for field in ("package_version", "channel", "manifest_url"):
        if not isinstance(evidence.get(field), str) or not evidence[field]:
            raise GlowupContractError(f"installed evidence {field} must be a non-empty string")
    for field in ("package_receipt", "binary_cohort", "installed", "running"):
        if evidence.get(field) is not True:
            raise GlowupContractError(f"installed evidence {field} must be true")
    for field in ("service", "gateway"):
        if evidence.get(field) != "ok":
            raise GlowupContractError(f"installed evidence {field} must be 'ok'")
    ready = evidence.get("profiles_ready")
    total = evidence.get("profiles_total")
    if not isinstance(total, int) or isinstance(total, bool) or total <= 0:
        raise GlowupContractError("installed evidence profiles_total must be positive")
    if not isinstance(ready, int) or isinstance(ready, bool) or ready != total:
        raise GlowupContractError(
            "installed evidence profiles_ready must equal profiles_total"
        )
    return evidence


def build_report(
    *,
    adapter: str,
    artifact: ArtifactIdentity,
    installed: Mapping[str, object],
    capabilities: Mapping[str, object],
) -> dict[str, object]:
    if not adapter:
        raise GlowupContractError("glow-up adapter name must not be empty")
    validate_installed_evidence(installed)
    return {
        "schema": REPORT_SCHEMA,
        "adapter": adapter,
        "artifact": artifact.as_report(),
        "installed": dict(installed),
        "capabilities": dict(capabilities),
    }


def load_manifest_bytes(contents: bytes) -> Mapping[str, object]:
    try:
        manifest = json.loads(contents)
    except json.JSONDecodeError as error:
        raise GlowupContractError(f"candidate manifest is invalid JSON: {error}") from error
    if not isinstance(manifest, dict):
        raise GlowupContractError("candidate manifest must be a JSON object")
    return manifest
