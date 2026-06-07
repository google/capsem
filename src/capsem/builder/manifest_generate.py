"""Profile catalog manifest generation for capsem-admin."""

from __future__ import annotations

from pathlib import Path
from urllib.parse import quote

import blake3

from capsem.builder.profiles import (
    ManifestProfile,
    ManifestProfileRevision,
    ProfileManifest,
    ProfilePayloadV2,
    ProfileRevisionStatus,
    validate_profile_json,
    validate_profile_toml,
)


def _profile_paths(profiles_dir: Path) -> list[Path]:
    return sorted(
        path
        for path in profiles_dir.rglob("*")
        if path.is_file()
        and path.suffix.lower() in {".json", ".toml"}
        and not path.name.endswith(".minisig")
    )


def _load_profile(path: Path) -> tuple[ProfilePayloadV2, bytes]:
    payload = path.read_bytes()
    if path.suffix.lower() == ".toml":
        return validate_profile_toml(path), payload
    return validate_profile_json(payload), payload


def _profile_hash(payload: bytes) -> str:
    return f"blake3:{blake3.blake3(payload).hexdigest()}"


def _revision_key(revision: str) -> tuple[int, int, int]:
    year, month_day, patch = revision.split(".")
    return int(year), int(month_day), int(patch)


def _status_for(
    profile: ProfilePayloadV2,
    status_overrides: dict[str, str] | None,
) -> ProfileRevisionStatus:
    if not status_overrides:
        return ProfileRevisionStatus.ACTIVE
    return ProfileRevisionStatus(
        status_overrides.get(f"{profile.id}@{profile.revision}", "active")
    )


def _profile_url(profiles_dir: Path, path: Path, base_url: str | None) -> str:
    if base_url is None:
        return path.as_uri()
    relative = path.relative_to(profiles_dir).as_posix()
    encoded = "/".join(quote(part) for part in relative.split("/"))
    return base_url.rstrip("/") + "/" + encoded


def generate_profile_manifest(
    profiles_dir: Path,
    *,
    base_url: str | None = None,
    status_overrides: dict[str, str] | None = None,
    current_overrides: dict[str, str] | None = None,
) -> ProfileManifest:
    profiles_dir = profiles_dir.resolve()
    records: dict[str, dict[str, ManifestProfileRevision]] = {}

    for path in _profile_paths(profiles_dir):
        profile, payload = _load_profile(path)
        profile_records = records.setdefault(profile.id, {})
        if profile.revision in profile_records:
            raise ValueError(f"duplicate profile revision: {profile.id}@{profile.revision}")
        url = _profile_url(profiles_dir, path, base_url)
        profile_records[profile.revision] = ManifestProfileRevision(
            status=_status_for(profile, status_overrides),
            min_binary=profile.compatibility.min_binary,
            max_binary=profile.compatibility.max_binary or None,
            profile_url=url,
            profile_hash=_profile_hash(payload),
            profile_signature_url=f"{url}.minisig",
        )

    if not records:
        raise ValueError(f"no Profile V2 JSON or TOML payloads found in {profiles_dir}")

    manifest_profiles: dict[str, ManifestProfile] = {}
    for profile_id, revisions in sorted(records.items()):
        current_revision = (
            current_overrides.get(profile_id)
            if current_overrides and profile_id in current_overrides
            else _latest_active_revision(profile_id, revisions)
        )
        manifest_profiles[profile_id] = ManifestProfile(
            current_revision=current_revision,
            revisions=dict(sorted(revisions.items(), key=lambda item: _revision_key(item[0]))),
        )

    return ProfileManifest(format=1, profiles=manifest_profiles)


def _latest_active_revision(
    profile_id: str,
    revisions: dict[str, ManifestProfileRevision],
) -> str:
    active = [
        revision
        for revision, record in revisions.items()
        if record.status is ProfileRevisionStatus.ACTIVE
    ]
    if not active:
        raise ValueError(f"profile '{profile_id}' has no active revision")
    return max(active, key=_revision_key)
