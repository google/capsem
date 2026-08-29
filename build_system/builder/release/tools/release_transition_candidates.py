#!/usr/bin/env python3
"""Release-owned derivation of exact installed transition candidates."""

from __future__ import annotations

import copy
import json
from dataclasses import dataclass
from pathlib import Path
from typing import cast

from .release_glowup import tamper_profile_artifact_digest, validate_installed_evidence
from .release_transition import validate_transition_verdict


@dataclass(frozen=True)
class TransitionCandidates:
    """One valid activation followed by two distinct rejection candidates."""

    updated: Path
    tampered: Path
    incompatible: Path


def require_object(value: object, field: str) -> dict[str, object]:
    if not isinstance(value, dict):
        raise RuntimeError(f"macOS glow-up report {field} must be an object")
    return cast(dict[str, object], value)


def validate_physical_evidence(report: dict[str, object], package_sha256: str) -> None:
    if report.get("package_sha256") != package_sha256:
        raise RuntimeError("physical VZ proof did not use the Tart-tested package")
    required = (
        "guest_vm_booted",
        "full_doctor",
        "installed_winterfell",
        "persistent_pin_resume",
    )
    missing = [field for field in required if report.get(field) is not True]
    if missing:
        raise RuntimeError(f"physical VZ proof did not pass {missing}")


def _manifest(path: Path) -> dict[str, object]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise RuntimeError(f"release transition manifest is unreadable: {path}: {error}") from error
    if not isinstance(value, dict):
        raise RuntimeError(f"release transition manifest must be an object: {path}")
    return cast(dict[str, object], value)


def _selected_profile(manifest: dict[str, object]) -> tuple[str, dict[str, object]]:
    profiles = manifest.get("profiles")
    if not isinstance(profiles, dict) or not profiles:
        raise RuntimeError("release transition manifest has no profiles")
    for profile_id in sorted(profiles):
        profile = profiles[profile_id]
        profile_object = cast(dict[str, object], profile) if isinstance(profile, dict) else None
        if (
            isinstance(profile_id, str)
            and profile_object is not None
            and str(profile_object.get("status") or "current").lower() != "revoked"
        ):
            return profile_id, profile_object
    raise RuntimeError("release transition manifest has no usable profile")


def _write(path: Path, manifest: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def stage_transition_candidates(
    authority: Path,
    destination: Path,
) -> TransitionCandidates:
    """Stage a real metadata update and two failures without mutating authority."""

    authority_bytes = authority.read_bytes()
    original = _manifest(authority)
    updated = copy.deepcopy(original)
    profile_id, profile = _selected_profile(updated)
    description = profile.get("description", "")
    if not isinstance(description, str):
        raise RuntimeError(f"release profile {profile_id} description must be a string")
    profile["description"] = f"{description} [installed transition proof]".strip()

    tampered = copy.deepcopy(updated)
    tamper_profile_artifact_digest(
        tampered,
        profile_ids=(profile_id,),
        architecture="arm64",
    )
    incompatible = copy.deepcopy(updated)
    _, incompatible_profile = _selected_profile(incompatible)
    incompatible_profile["min_capsem_version"] = "9999.0.0"

    candidates = TransitionCandidates(
        updated=destination / "updated-manifest.json",
        tampered=destination / "tampered-manifest.json",
        incompatible=destination / "incompatible-manifest.json",
    )
    for path, manifest in (
        (candidates.updated, updated),
        (candidates.tampered, tampered),
        (candidates.incompatible, incompatible),
    ):
        _write(path, manifest)
    if authority.read_bytes() != authority_bytes:
        raise RuntimeError("transition staging mutated the authoritative candidate manifest")
    payloads = [authority_bytes, *(path.read_bytes() for path in candidates.__dict__.values())]
    if len(payloads) != len(set(payloads)):
        raise RuntimeError("release transition candidates must contain distinct exact bytes")
    return candidates


def validate_complete_verdicts(
    fresh: dict[str, object],
    update: dict[str, object],
    tamper: dict[str, object],
    incompatible: dict[str, object],
    *,
    source: str,
    original_sha256: str,
    updated_sha256: str,
    tampered_sha256: str,
    incompatible_sha256: str,
) -> None:
    """Require exact fetch, activation, rejection, and preservation causality."""

    if original_sha256 == updated_sha256:
        raise RuntimeError("macOS transition update does not identify distinct candidate bytes")
    validate_transition_verdict(
        fresh,
        kind="fresh_install",
        result="activated",
        source=source,
        candidate_manifest_sha256=original_sha256,
    )
    validate_transition_verdict(
        update,
        kind="profile_only",
        result="activated",
        source=source,
        candidate_manifest_sha256=updated_sha256,
    )
    if len({original_sha256, updated_sha256, tampered_sha256, incompatible_sha256}) != 4:
        raise RuntimeError("macOS transition candidates do not identify four distinct payloads")
    for verdict, kind, digest in (
        (tamper, "tampered_artifact", tampered_sha256),
        (incompatible, "incompatible_profile", incompatible_sha256),
    ):
        validate_transition_verdict(
            verdict,
            kind=kind,
            result="rejected",
            source=source,
            candidate_manifest_sha256=digest,
            previous_manifest_sha256=updated_sha256,
        )


def validate_macos_guest_report(
    report_path: Path,
    *,
    artifact_sha256: str,
    manifest_source: str,
    original_sha256: str,
    updated_sha256: str,
    tampered_sha256: str,
    incompatible_sha256: str,
) -> dict[str, object]:
    """Validate the complete exact-candidate evidence returned by Tart."""

    report = require_object(json.loads(report_path.read_text(encoding="utf-8")), "guest root")
    if report.get("schema") != "capsem.release_glowup.guest.v1":
        raise RuntimeError("Tart guest wrote an unsupported glow-up evidence schema")
    if report.get("artifact_sha256") != artifact_sha256:
        raise RuntimeError("Tart guest package SHA does not match the host candidate")
    installed = require_object(report.get("installed"), "installed")
    fresh_installed = require_object(report.get("fresh_installed"), "fresh_installed")
    preserved = require_object(report.get("preserved_installed"), "preserved_installed")
    for evidence in (fresh_installed, installed, preserved):
        validate_installed_evidence(evidence)
    if preserved != installed:
        raise RuntimeError("Tart guest did not preserve the exact activated state")
    fresh = require_object(report.get("fresh_transition"), "fresh_transition")
    update = require_object(report.get("update_transition"), "update_transition")
    tamper = require_object(report.get("tamper_rejection"), "tamper_rejection")
    incompatible = require_object(report.get("incompatible_rejection"), "incompatible_rejection")
    asset_hydration = require_object(report.get("asset_hydration"), "asset_hydration")
    for field in ("manifest_only_install", "started", "downloading", "completed_ready"):
        if asset_hydration.get(field) is not True:
            raise RuntimeError(f"Tart guest asset hydration did not prove {field}")
    stale_helper = require_object(
        report.get("stale_helper_replacement"), "stale_helper_replacement"
    )
    if stale_helper.get("old_service_retired") is not True:
        raise RuntimeError("Tart guest did not retire the stale installed helper")
    if stale_helper.get("old_service_pid") == stale_helper.get("new_service_pid"):
        raise RuntimeError("Tart guest stale helper replacement reused the old PID")
    validate_complete_verdicts(
        fresh,
        update,
        tamper,
        incompatible,
        source=manifest_source,
        original_sha256=original_sha256,
        updated_sha256=updated_sha256,
        tampered_sha256=tampered_sha256,
        incompatible_sha256=incompatible_sha256,
    )
    return report
