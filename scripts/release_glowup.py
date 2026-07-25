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
import re
from collections.abc import Sequence
from enum import Enum
from pathlib import Path
from typing import Mapping, cast


REPORT_SCHEMA = "capsem.release_glowup.v1"
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")


class GlowupContractError(RuntimeError):
    """The candidate failed a platform-neutral release invariant."""


class PackageArchitecture(str, Enum):
    ARM64 = "arm64"
    AMD64 = "amd64"


class TransitionKind(str, Enum):
    """Installed-product transitions every public release path must prove."""

    FRESH_INSTALL = "fresh_install"
    BINARY_ONLY = "binary_only"
    PROFILE_ONLY = "profile_only"
    PROFILE_THEN_BINARY = "profile_then_binary"
    CHANNEL_SWITCH = "channel_switch"
    TAMPER_REJECTION = "tamper_rejection"


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
        architecture: PackageArchitecture,
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
        try:
            package_architecture = PackageArchitecture(architecture)
        except ValueError as error:
            raise GlowupContractError(
                f"unsupported package architecture: {architecture}"
            ) from error
        validate_package_identity(path.name, platform, package_architecture)
        return cls(
            path=path,
            name=path.name,
            version=version,
            platform=platform,
            architecture=package_architecture,
            bytes=size,
            sha256=digest.hexdigest(),
        )

    def as_report(self) -> dict[str, object]:
        return {
            "name": self.name,
            "version": self.version,
            "platform": self.platform,
            "architecture": self.architecture.value,
            "bytes": self.bytes,
            "sha256": self.sha256,
        }


class PairingIdentity:
    """Exact installed channel, package, and profile-set identity."""

    __slots__ = (
        "channel",
        "manifest_sha256",
        "package_version",
        "package_sha256",
        "profiles_sha256",
    )

    def __init__(
        self,
        *,
        channel: str,
        manifest_sha256: str,
        package_version: str,
        package_sha256: str,
        profiles_sha256: str,
    ) -> None:
        if not channel:
            raise GlowupContractError("pairing channel must not be empty")
        if not package_version:
            raise GlowupContractError("pairing package version must not be empty")
        for field, value in (
            ("manifest_sha256", manifest_sha256),
            ("package_sha256", package_sha256),
            ("profiles_sha256", profiles_sha256),
        ):
            if not isinstance(value, str) or SHA256_PATTERN.fullmatch(value) is None:
                raise GlowupContractError(f"pairing {field} must be a lowercase sha256 digest")
        self.channel = channel
        self.manifest_sha256 = manifest_sha256
        self.package_version = package_version
        self.package_sha256 = package_sha256
        self.profiles_sha256 = profiles_sha256

    @classmethod
    def from_manifest_bytes(
        cls,
        contents: bytes,
        *,
        artifact: ArtifactIdentity,
        channel: str,
        allow_empty_profiles: bool = False,
    ) -> PairingIdentity:
        manifest = load_manifest_bytes(contents)
        assert_manifest_artifact(manifest, artifact)
        profiles = manifest.get("profiles")
        if not isinstance(profiles, dict) or (not profiles and not allow_empty_profiles):
            raise GlowupContractError("candidate manifest profiles must be a non-empty object")
        profiles_bytes = json.dumps(
            profiles,
            sort_keys=True,
            separators=(",", ":"),
        ).encode()
        return cls(
            channel=channel,
            manifest_sha256=hashlib.sha256(contents).hexdigest(),
            package_version=artifact.version,
            package_sha256=artifact.sha256,
            profiles_sha256=hashlib.sha256(profiles_bytes).hexdigest(),
        )

    @classmethod
    def from_report(cls, report: Mapping[str, object]) -> PairingIdentity:
        def require_string(field: str) -> str:
            value = report.get(field)
            if not isinstance(value, str):
                raise GlowupContractError(f"transition pairing identity {field} must be a string")
            return value

        return cls(
            channel=require_string("channel"),
            manifest_sha256=require_string("manifest_sha256"),
            package_version=require_string("package_version"),
            package_sha256=require_string("package_sha256"),
            profiles_sha256=require_string("profiles_sha256"),
        )

    def as_report(self) -> dict[str, object]:
        return {
            "channel": self.channel,
            "manifest_sha256": self.manifest_sha256,
            "package_version": self.package_version,
            "package_sha256": self.package_sha256,
            "profiles_sha256": self.profiles_sha256,
        }


def validate_package_identity(
    name: str,
    platform: str,
    architecture: PackageArchitecture,
) -> None:
    if platform == "linux":
        expected_suffix = f"_{architecture.value}.deb"
        if not name.endswith(expected_suffix):
            raise GlowupContractError(f"linux package {name} must end in {expected_suffix}")
        return
    if platform == "macos":
        if not name.endswith(".pkg"):
            raise GlowupContractError(f"macOS package {name} must end in .pkg")
        if architecture is not PackageArchitecture.ARM64:
            raise GlowupContractError("macOS package architecture must be arm64")
        return
    raise GlowupContractError(f"unsupported package platform: {platform}")


def assert_manifest_artifact(
    manifest: Mapping[str, object],
    artifact: ArtifactIdentity,
    *,
    manifest_architecture_aliases: Sequence[str] = (),
) -> Mapping[str, object]:
    """Require one current release record to describe the exact package bytes."""

    packages = manifest.get("packages")
    if not isinstance(packages, list):
        raise GlowupContractError("candidate manifest packages must be an array")
    accepted_architectures = {
        artifact.architecture.value,
        *manifest_architecture_aliases,
    }
    matches: list[Mapping[str, object]] = []
    for candidate in packages:
        if not isinstance(candidate, dict):
            continue
        package = cast(Mapping[str, object], candidate)
        if (
            package.get("name") == artifact.name
            and package.get("platform") == artifact.platform
            and package.get("architecture") in accepted_architectures
        ):
            matches.append(package)
    if len(matches) != 1:
        raise GlowupContractError(
            "candidate manifest must contain exactly one package record for "
            f"{artifact.name} ({artifact.platform}/{artifact.architecture.value}); "
            f"found {len(matches)}"
        )
    package = matches[0]
    expected = {
        "name": artifact.name,
        "version": artifact.version,
        "platform": artifact.platform,
        "architecture": artifact.architecture.value,
        "bytes": artifact.bytes,
        "status": "current",
    }
    for field, value in expected.items():
        actual = package.get(field)
        if field == "architecture":
            matches_expected = actual in accepted_architectures
        else:
            matches_expected = actual == value
        if not matches_expected:
            raise GlowupContractError(
                f"candidate manifest package {field} is {actual!r}, expected {value!r}"
            )
    digest = package.get("digest")
    actual_sha256 = (
        cast(Mapping[str, object], digest).get("sha256") if isinstance(digest, dict) else None
    )
    if actual_sha256 != artifact.sha256:
        raise GlowupContractError(
            f"candidate manifest package sha256 is {actual_sha256!r}, expected {artifact.sha256!r}"
        )
    return package


def tamper_profile_artifact_digest(
    manifest: dict[str, object],
    *,
    profile_ids: Sequence[str] = (),
) -> str:
    """Corrupt one current profile artifact digest for a rejection proof.

    The caller must pass a private copy of the manifest.  Returning the selected
    profile id lets adapters record which profile supplied the adversarial
    candidate without inventing a second mutation contract.
    """

    profiles = manifest.get("profiles")
    if not isinstance(profiles, dict) or not profiles:
        raise GlowupContractError("adversarial candidate manifest has no profiles")
    profile_map = cast(dict[str, object], profiles)
    selected = tuple(profile_ids) or tuple(sorted(profile_map))
    for profile_id in selected:
        profile = profile_map.get(profile_id)
        if not isinstance(profile_id, str) or not isinstance(profile, dict):
            continue
        profile_fields = cast(dict[str, object], profile)
        architectures = profile_fields.get("architectures")
        if not isinstance(architectures, list):
            continue
        for architecture in architectures:
            if not isinstance(architecture, dict):
                continue
            architecture_fields = cast(dict[str, object], architecture)
            for section in ("config", "images", "evidence"):
                rows = architecture_fields.get(section)
                if not isinstance(rows, list):
                    continue
                for row in rows:
                    if not isinstance(row, dict):
                        continue
                    row_fields = cast(dict[str, object], row)
                    if row_fields.get("status", "current") != "current":
                        continue
                    digest = row_fields.get("digest")
                    if not isinstance(digest, dict):
                        continue
                    digest_fields = cast(dict[str, object], digest)
                    sha256 = digest_fields.get("sha256")
                    if not isinstance(sha256, str):
                        continue
                    digest_fields["sha256"] = (
                        "1" * 64 if sha256 == "0" * 64 else "0" * 64
                    )
                    blake3 = digest_fields.get("blake3")
                    if isinstance(blake3, str):
                        digest_fields["blake3"] = (
                            "1" * 64 if blake3 == "0" * 64 else "0" * 64
                        )
                    return profile_id
    raise GlowupContractError(
        "adversarial candidate has no current digest-bearing profile artifact"
    )


def artifact_identity_from_manifest_package(
    contents: bytes,
    package_path: Path,
) -> ArtifactIdentity:
    """Resolve one exact current package record without guessing its metadata."""

    manifest = load_manifest_bytes(contents)
    packages = manifest.get("packages")
    if not isinstance(packages, list):
        raise GlowupContractError("candidate manifest packages must be an array")
    matches: list[Mapping[str, object]] = []
    for candidate in packages:
        if not isinstance(candidate, dict):
            continue
        package = cast(Mapping[str, object], candidate)
        if package.get("name") == package_path.name and package.get("status") == "current":
            matches.append(package)
    if len(matches) != 1:
        raise GlowupContractError(
            "release pairing manifest must contain exactly one current package "
            f"record for {package_path.name}; found {len(matches)}"
        )
    package = matches[0]
    version = package.get("version")
    platform = package.get("platform")
    architecture = package.get("architecture")
    if not all(isinstance(value, str) and value for value in (version, platform, architecture)):
        raise GlowupContractError(
            f"release pairing package metadata is incomplete for {package_path.name}"
        )
    canonical_architecture = cast(str, architecture)
    manifest_architecture_aliases: tuple[str, ...] = ()
    if (
        platform == "linux"
        and architecture == "x86_64"
        and package_path.name.endswith("_amd64.deb")
    ):
        canonical_architecture = "amd64"
        manifest_architecture_aliases = ("x86_64",)
    artifact = ArtifactIdentity.from_path(
        package_path,
        version=cast(str, version),
        platform=cast(str, platform),
        architecture=canonical_architecture,
    )
    assert_manifest_artifact(
        manifest,
        artifact,
        manifest_architecture_aliases=manifest_architecture_aliases,
    )
    return artifact


def validate_pairing_inputs(
    *,
    kind: TransitionKind | str,
    channel: str,
    before_manifest_bytes: bytes,
    after_manifest_bytes: bytes,
    before_artifact: ArtifactIdentity,
    after_artifact: ArtifactIdentity,
    changed_profiles: Sequence[str] = (),
) -> tuple[PairingIdentity, PairingIdentity]:
    """Validate an exact public-before/candidate-after release-lane pairing."""

    try:
        transition_kind = TransitionKind(kind)
    except ValueError as error:
        raise GlowupContractError(f"unsupported release transition: {kind}") from error
    if transition_kind not in {
        TransitionKind.BINARY_ONLY,
        TransitionKind.PROFILE_ONLY,
        TransitionKind.PROFILE_THEN_BINARY,
    }:
        raise GlowupContractError(
            f"{transition_kind.value} is not a release-lane pairing transition"
        )
    before_manifest = load_manifest_bytes(before_manifest_bytes)
    after_manifest = load_manifest_bytes(after_manifest_bytes)
    for label, manifest in (
        ("public-before", before_manifest),
        ("candidate-after", after_manifest),
    ):
        if manifest.get("channel") != channel:
            raise GlowupContractError(
                f"{label} manifest channel is {manifest.get('channel')!r}, expected {channel!r}"
            )

    before = PairingIdentity.from_manifest_bytes(
        before_manifest_bytes,
        artifact=before_artifact,
        channel=channel,
        allow_empty_profiles=transition_kind
        in {TransitionKind.PROFILE_ONLY, TransitionKind.PROFILE_THEN_BINARY},
    )
    after = PairingIdentity.from_manifest_bytes(
        after_manifest_bytes,
        artifact=after_artifact,
        channel=channel,
    )

    before_profiles = before_manifest.get("profiles")
    after_profiles = after_manifest.get("profiles")
    if not isinstance(before_profiles, dict) or not isinstance(after_profiles, dict):
        raise GlowupContractError("release pairing manifests must contain profile objects")
    before_profile_map = cast(Mapping[str, object], before_profiles)
    after_profile_map = cast(Mapping[str, object], after_profiles)
    changed_profile_ids = tuple(changed_profiles)
    if len(changed_profile_ids) != len(set(changed_profile_ids)):
        raise GlowupContractError("release pairing changed profile ids must be unique")
    if transition_kind is TransitionKind.BINARY_ONLY:
        if changed_profile_ids:
            raise GlowupContractError("binary_only release pairing cannot select a changed profile")
    else:
        if not changed_profile_ids:
            raise GlowupContractError(
                f"{transition_kind.value} release pairing requires changed profiles"
            )
        if transition_kind is TransitionKind.PROFILE_ONLY and len(changed_profile_ids) != 1:
            raise GlowupContractError("profile_only release pairing requires exactly one profile")
        for profile_id in changed_profile_ids:
            if profile_id not in after_profile_map:
                raise GlowupContractError(
                    f"candidate-after manifest lacks changed profile {profile_id!r}"
                )
        profile_ids = set(before_profile_map) | set(after_profile_map)
        for profile_id in profile_ids - set(changed_profile_ids):
            if before_profile_map.get(profile_id) != after_profile_map.get(profile_id):
                raise GlowupContractError(
                    f"{transition_kind.value} release pairing changed unselected profile "
                    f"{profile_id!r}"
                )

    _validate_transition_pairing(
        transition_kind=transition_kind,
        before=before,
        after=after,
        result="activated",
        staged_profiles_sha256=(
            after.profiles_sha256 if transition_kind is TransitionKind.PROFILE_THEN_BINARY else None
        ),
        preserved_previous=False,
    )
    return before, after


def classify_pairing_inputs(
    *,
    channel: str,
    before_manifest_bytes: bytes,
    after_manifest_bytes: bytes,
    before_artifact: ArtifactIdentity,
    after_artifact: ArtifactIdentity,
) -> tuple[TransitionKind, tuple[str, ...]]:
    """Classify a binary lane and return its complete staged profile set."""

    before_manifest = load_manifest_bytes(before_manifest_bytes)
    after_manifest = load_manifest_bytes(after_manifest_bytes)
    before_profiles = before_manifest.get("profiles")
    after_profiles = after_manifest.get("profiles")
    if not isinstance(before_profiles, dict) or not isinstance(after_profiles, dict):
        raise GlowupContractError("release pairing manifests must contain profile objects")
    before_profile_map = cast(Mapping[str, object], before_profiles)
    after_profile_map = cast(Mapping[str, object], after_profiles)
    if not all(
        isinstance(profile_id, str)
        for profile_id in set(before_profile_map) | set(after_profile_map)
    ):
        raise GlowupContractError("release pairing profile ids must be strings")
    changed = sorted(
        profile_id
        for profile_id in set(before_profile_map) | set(after_profile_map)
        if before_profile_map.get(profile_id) != after_profile_map.get(profile_id)
    )
    if not changed:
        transition_kind = TransitionKind.BINARY_ONLY
    else:
        transition_kind = TransitionKind.PROFILE_THEN_BINARY
    validate_pairing_inputs(
        kind=transition_kind,
        channel=channel,
        before_manifest_bytes=before_manifest_bytes,
        after_manifest_bytes=after_manifest_bytes,
        before_artifact=before_artifact,
        after_artifact=after_artifact,
        changed_profiles=changed,
    )
    return transition_kind, tuple(changed)


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
        raise GlowupContractError("installed evidence profiles_ready must equal profiles_total")
    return evidence


def _require_same_channel(
    kind: TransitionKind,
    before: PairingIdentity,
    after: PairingIdentity,
) -> None:
    if before.channel != after.channel:
        raise GlowupContractError(f"{kind.value} transition must remain in the same channel")


def _require_package_changed(
    kind: TransitionKind,
    before: PairingIdentity,
    after: PairingIdentity,
) -> None:
    if (
        before.package_version == after.package_version
        or before.package_sha256 == after.package_sha256
    ):
        raise GlowupContractError(f"{kind.value} transition must change the exact package identity")


def _validate_transition_pairing(
    *,
    transition_kind: TransitionKind,
    before: PairingIdentity | None,
    after: PairingIdentity,
    result: str,
    staged_profiles_sha256: str | None,
    preserved_previous: bool,
) -> None:
    """Validate pairing deltas without claiming that runtime probes have run."""

    if result not in {"activated", "rejected"}:
        raise GlowupContractError("release transition result must be 'activated' or 'rejected'")
    if transition_kind is TransitionKind.FRESH_INSTALL:
        if before is not None or result != "activated":
            raise GlowupContractError(
                "fresh_install transition must activate without a previous pairing"
            )
    elif before is None:
        raise GlowupContractError(f"{transition_kind.value} transition requires a previous pairing")

    if before is not None:
        if transition_kind is TransitionKind.BINARY_ONLY:
            _require_same_channel(transition_kind, before, after)
            _require_package_changed(transition_kind, before, after)
            if before.profiles_sha256 != after.profiles_sha256:
                raise GlowupContractError("binary_only transition must preserve exact profiles")
        elif transition_kind is TransitionKind.PROFILE_ONLY:
            _require_same_channel(transition_kind, before, after)
            if (
                before.package_version != after.package_version
                or before.package_sha256 != after.package_sha256
            ):
                raise GlowupContractError("profile_only transition must preserve the exact package")
            if before.profiles_sha256 == after.profiles_sha256:
                raise GlowupContractError("profile_only transition must change the profile set")
        elif transition_kind is TransitionKind.PROFILE_THEN_BINARY:
            _require_same_channel(transition_kind, before, after)
            _require_package_changed(transition_kind, before, after)
            if before.profiles_sha256 == after.profiles_sha256:
                raise GlowupContractError(
                    "profile_then_binary transition must change the profile set"
                )
            if (
                staged_profiles_sha256 is None
                or SHA256_PATTERN.fullmatch(staged_profiles_sha256) is None
                or staged_profiles_sha256 != after.profiles_sha256
            ):
                raise GlowupContractError(
                    "profile_then_binary transition must reuse the exact staged profile set"
                )
        elif transition_kind is TransitionKind.CHANNEL_SWITCH:
            if before.channel == after.channel:
                raise GlowupContractError(
                    "channel_switch transition must change the selected channel"
                )
        elif transition_kind is TransitionKind.TAMPER_REJECTION:
            if result != "rejected":
                raise GlowupContractError("tamper_rejection transition must reject the candidate")
            if before.as_report() != after.as_report() or preserved_previous is not True:
                raise GlowupContractError(
                    "tamper_rejection transition must preserve the exact previous working state"
                )

    if transition_kind is not TransitionKind.TAMPER_REJECTION:
        if result != "activated":
            raise GlowupContractError(
                f"{transition_kind.value} transition must activate its candidate"
            )
        if preserved_previous:
            raise GlowupContractError(
                f"{transition_kind.value} transition cannot report previous-state preservation"
            )


def build_transition_evidence(
    *,
    kind: TransitionKind | str,
    before: PairingIdentity | None,
    after: PairingIdentity,
    result: str,
    doctor_passed: bool,
    winterfell_passed: bool,
    staged_profiles_sha256: str | None = None,
    preserved_previous: bool = False,
) -> dict[str, object]:
    """Validate and normalize one installed release transition proof."""

    try:
        transition_kind = TransitionKind(kind)
    except ValueError as error:
        raise GlowupContractError(f"unsupported release transition: {kind}") from error
    if doctor_passed is not True:
        raise GlowupContractError(
            f"{transition_kind.value} transition must pass full capsem-doctor"
        )
    if winterfell_passed is not True:
        raise GlowupContractError(f"{transition_kind.value} transition must pass Winterfell")

    _validate_transition_pairing(
        transition_kind=transition_kind,
        before=before,
        after=after,
        result=result,
        staged_profiles_sha256=staged_profiles_sha256,
        preserved_previous=preserved_previous,
    )

    evidence: dict[str, object] = {
        "kind": transition_kind.value,
        "result": result,
        "before": before.as_report() if before is not None else None,
        "after": after.as_report(),
        "probes": {
            "doctor": doctor_passed,
            "winterfell": winterfell_passed,
        },
        "preserved_previous": preserved_previous,
    }
    if staged_profiles_sha256 is not None:
        evidence["staged_profiles_sha256"] = staged_profiles_sha256
    return evidence


def _expected_transition_values(
    expected_transitions: Sequence[TransitionKind | str] | None,
) -> list[str]:
    if expected_transitions is None:
        return [kind.value for kind in TransitionKind]
    try:
        expected = [TransitionKind(kind).value for kind in expected_transitions]
    except ValueError as error:
        raise GlowupContractError(f"unsupported expected release transition: {error}") from error
    if not expected:
        raise GlowupContractError("declared transition scope must not be empty")
    if expected[0] != TransitionKind.FRESH_INSTALL.value:
        raise GlowupContractError("declared transition scope must begin with fresh_install")
    if len(expected) != len(set(expected)):
        raise GlowupContractError("declared transition scope must not contain duplicates")
    if (
        TransitionKind.TAMPER_REJECTION.value in expected
        and expected[-1] != TransitionKind.TAMPER_REJECTION.value
    ):
        raise GlowupContractError("tamper_rejection must be the final declared transition")
    return expected


def validate_transition_sequence(
    transitions: Sequence[Mapping[str, object]],
    *,
    expected_transitions: Sequence[TransitionKind | str] | None = None,
) -> list[dict[str, object]]:
    """Require ordered proof for the complete or explicitly lane-scoped transition set."""

    expected = _expected_transition_values(expected_transitions)
    actual = [transition.get("kind") for transition in transitions]
    if actual != expected:
        raise GlowupContractError(f"transition sequence must contain exactly {expected} in order")

    normalized: list[dict[str, object]] = []
    for transition in transitions:
        before_report = transition.get("before")
        after_report = transition.get("after")
        if before_report is not None and not isinstance(before_report, Mapping):
            raise GlowupContractError(
                "transition pairing identity before must be an object or null"
            )
        if not isinstance(after_report, Mapping):
            raise GlowupContractError("transition pairing identity after must be an object")
        probes = transition.get("probes")
        if not isinstance(probes, Mapping):
            raise GlowupContractError("transition probes must be an object")
        staged_digest = transition.get("staged_profiles_sha256")
        if staged_digest is not None and not isinstance(staged_digest, str):
            raise GlowupContractError("transition staged profile digest must be a string")
        normalized.append(
            build_transition_evidence(
                kind=str(transition["kind"]),
                before=(
                    PairingIdentity.from_report(cast(Mapping[str, object], before_report))
                    if before_report is not None
                    else None
                ),
                after=PairingIdentity.from_report(cast(Mapping[str, object], after_report)),
                result=str(transition.get("result")),
                doctor_passed=cast(Mapping[str, object], probes).get("doctor") is True,
                winterfell_passed=(cast(Mapping[str, object], probes).get("winterfell") is True),
                staged_profiles_sha256=staged_digest,
                preserved_previous=transition.get("preserved_previous") is True,
            )
        )
    return normalized


def build_report(
    *,
    adapter: str,
    artifact: ArtifactIdentity,
    installed: Mapping[str, object],
    capabilities: Mapping[str, object],
    transitions: Sequence[Mapping[str, object]] | None = None,
    expected_transitions: Sequence[TransitionKind | str] | None = None,
) -> dict[str, object]:
    if not adapter:
        raise GlowupContractError("glow-up adapter name must not be empty")
    validate_installed_evidence(installed)
    report: dict[str, object] = {
        "schema": REPORT_SCHEMA,
        "adapter": adapter,
        "artifact": artifact.as_report(),
        "installed": dict(installed),
        "capabilities": dict(capabilities),
    }
    if transitions is None:
        if expected_transitions is not None:
            raise GlowupContractError(
                "declared transition scope requires transition evidence"
            )
    else:
        transition_scope = _expected_transition_values(expected_transitions)
        report["transition_scope"] = transition_scope
        report["transitions"] = validate_transition_sequence(
            transitions,
            expected_transitions=transition_scope,
        )
    return report


def load_manifest_bytes(contents: bytes) -> Mapping[str, object]:
    try:
        manifest = json.loads(contents)
    except json.JSONDecodeError as error:
        raise GlowupContractError(f"candidate manifest is invalid JSON: {error}") from error
    if not isinstance(manifest, dict):
        raise GlowupContractError("candidate manifest must be a JSON object")
    return manifest
