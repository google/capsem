"""Admit prepared immutable artifacts before any disposable installed owner."""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Literal
from urllib.parse import urlsplit

from capsem_builder.gate.configschema import Strict
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.versions import require_semver
from capsem_builder.release.tools.release_inputs import load_verified_release_inputs
from capsem_builder.release.tools.release_macos_pairing import validate_pair
from pydantic import Field, field_validator


class Phase(Strict):
    package: Path
    manifest: Path
    sbom: Path
    runtime_inputs: Path

    @field_validator("*")
    @classmethod
    def absolute(cls, value: Path) -> Path:
        if not value.is_absolute():
            raise ValueError("Install input paths must name exact absolute artifacts")
        return value


class TartInputs(Strict):
    asset_share: Path
    manifest_url: str
    tampered_manifest: Path
    updated_manifest: Path
    incompatible_manifest: Path


class InstallInputs(Strict):
    schema_id: Literal["capsem.bench.install_inputs.v1"] = Field(alias="schema")
    platform: Literal["linux", "macos"]
    architecture: Literal["arm64", "amd64"]
    source_commit: str = Field(pattern=r"^[0-9a-f]{40}$")
    channel: Literal["stable", "nightly"]
    content_root: Path
    before: Phase
    after: Phase
    tart: TartInputs | None = None


@dataclass(frozen=True)
class Prepared:
    inputs: InstallInputs
    pair: dict


def read_bytes(path: Path, maximum: int) -> bytes:
    if maximum <= 0:
        raise ValueError("Install evidence byte budget must be positive")
    with path.open("rb") as stream:
        raw = stream.read(maximum + 1)
    if len(raw) > maximum:
        raise ValueError("Install input/evidence exceeds configured byte budget")
    return raw


def read_object(path: Path, maximum: int) -> dict:
    value = json.loads(read_bytes(path, maximum))
    if not isinstance(value, dict):
        raise ValueError("Install input/evidence must be an object")
    return value


def load_inputs(
    path: Path,
    *,
    maximum: int,
    package_maximum: int,
    source_commit: str,
    platform: str,
    architecture: str,
) -> Prepared:
    inputs = InstallInputs.model_validate(read_object(path, maximum))
    if (inputs.source_commit, inputs.platform, inputs.architecture) != (
        source_commit,
        platform,
        architecture,
    ):
        raise ValueError("Install inputs have a foreign recorded source/platform/architecture")
    if not inputs.content_root.is_absolute() or not inputs.content_root.is_dir():
        raise ValueError("Install requires a prepared canonical runtime content root")
    for phase in (inputs.before, inputs.after):
        if not 0 < phase.package.stat().st_size <= package_maximum:
            raise ValueError("Install package exceeds configured byte budget")
        manifest = read_object(phase.manifest, maximum)
        read_object(phase.sbom, maximum)
        report, verified, _ = load_verified_release_inputs(phase.runtime_inputs)
        if report.get("kind") != "runtime" or verified != manifest:
            raise ValueError("Install runtime inputs do not reproduce the exact package manifest")
    pair = validate_pair(
        before=(inputs.before.package, inputs.before.manifest, inputs.before.sbom),
        after=(inputs.after.package, inputs.after.manifest, inputs.after.sbom),
        channel=inputs.channel,
        platform=platform,
        architecture=architecture,
    )
    if pair["after"]["source_commit"] != source_commit:
        raise ValueError("Install after-package source differs from the recorded source")
    for name, phase in (("before", inputs.before), ("after", inputs.after)):
        minimum = read_object(phase.manifest, maximum)["runtime"].get("min_capsem_version", "")
        if not isinstance(minimum, str):
            raise ValueError("Install runtime minimum binary version is invalid")
        if minimum:
            try:
                required = tuple(
                    map(int, require_semver(minimum, source="runtime minimum").split("."))
                )
            except GateError as error:
                raise ValueError(str(error)) from error
            if tuple(map(int, pair[name]["artifact"]["version"].split("."))) < required:
                raise ValueError(
                    "Install before/after binary does not satisfy the original runtime minimum"
                )
    if platform == "macos":
        if inputs.tart is None:
            raise ValueError("Install macOS inputs must name the existing Tart fixtures")
        url = urlsplit(inputs.tart.manifest_url)
        if url.scheme != "http" or url.hostname != "127.0.0.1" or url.username or url.password:
            raise ValueError("Tart requires its existing loopback release fixture URL")
        if any(
            pair[phase]["metadata_manifest_url"] != inputs.tart.manifest_url
            for phase in ("before", "after")
        ):
            raise ValueError("Tart packages must originally own the exact fixture URL")
        if not inputs.tart.asset_share.is_dir():
            raise ValueError("Tart requires its prepared runtime asset share")
        for name in ("tampered_manifest", "updated_manifest", "incompatible_manifest"):
            read_object(getattr(inputs.tart, name), maximum)
    elif inputs.tart is not None:
        raise ValueError("Linux Install must not select Tart authority")
    return Prepared(inputs, pair)


def phase_proof(phase: dict) -> dict:
    return {
        "source_commit": phase["source_commit"],
        "version": phase["artifact"]["version"],
        "package_sha256": phase["artifact"]["sha256"],
        "sbom_sha256": phase["sbom_sha256"],
        "manifest_sha256": phase["manifest_sha256"],
        "runtime_sha256": phase["runtime_sha256"],
    }
