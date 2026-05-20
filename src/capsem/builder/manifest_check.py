"""Fast profile manifest checks for capsem-admin."""

from __future__ import annotations

from pathlib import Path
from typing import Literal
from urllib.error import HTTPError, URLError
from urllib.parse import unquote
from urllib.request import Request, urlopen

import blake3
from pydantic import AnyUrl, BaseModel, ConfigDict, Field, ValidationError

from capsem.builder.profiles import (
    ProfileManifest,
    ProfilePayloadV2,
    ProfileRevisionStatus,
    validate_manifest_json,
    validate_profile_json,
    validate_profile_toml,
)

_ACCEPTABLE_PROFILE_CONTENT_TYPES = {
    "application/json",
    "application/octet-stream",
    "application/toml",
    "application/x-toml",
    "text/plain",
    "text/toml",
    "text/x-toml",
}


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, populate_by_name=True)


class ManifestUrlCheck(StrictModel):
    kind: Literal["profile_payload", "profile_signature"]
    url: str
    scheme: str
    ok: bool
    status_code: int | None = None
    content_length: int | None = None
    content_type: str | None = None
    expected_hash: str | None = None
    actual_hash: str | None = None
    failure: (
        Literal[
            "missing",
            "hash_mismatch",
            "identity_mismatch",
            "invalid_payload",
            "invalid_scheme",
            "http_error",
            "head_failed",
            "content_type_mismatch",
        ]
        | None
    ) = None
    message: str | None = None


class ManifestProfileCheck(StrictModel):
    profile_id: str
    revision: str
    status: ProfileRevisionStatus
    current: bool
    checks: list[ManifestUrlCheck]

    @property
    def ok(self) -> bool:
        return all(check.ok for check in self.checks)


class ManifestCheckReport(StrictModel):
    schema_: Literal["capsem.manifest-check.v1"] = Field(
        default="capsem.manifest-check.v1",
        alias="schema",
    )
    ok: bool
    mode: Literal["fast"]
    manifest_path: str
    profiles: list[ManifestProfileCheck]


def _blake3_hash(payload: bytes) -> str:
    return f"blake3:{blake3.blake3(payload).hexdigest()}"


def _content_length(headers: object) -> int | None:
    value = headers.get("Content-Length") if hasattr(headers, "get") else None
    if value is None:
        return None
    try:
        return int(value)
    except ValueError:
        return None


def _content_type(headers: object) -> str | None:
    value = headers.get("Content-Type") if hasattr(headers, "get") else None
    if value is None:
        return None
    return value.split(";", 1)[0].strip().lower()


def _file_url_path(url: AnyUrl) -> Path:
    return Path(unquote(url.path or ""))


def _parse_local_profile_payload(path: Path) -> ProfilePayloadV2:
    if path.suffix.lower() == ".toml":
        return validate_profile_toml(path)
    return validate_profile_json(path.read_bytes())


def _check_local_payload(
    url: AnyUrl,
    *,
    expected_hash: str,
    profile_id: str,
    revision: str,
) -> ManifestUrlCheck:
    path = _file_url_path(url)
    if not path.is_file():
        return ManifestUrlCheck(
            kind="profile_payload",
            url=str(url),
            scheme=url.scheme,
            ok=False,
            expected_hash=expected_hash,
            failure="missing",
            message=f"profile payload file not found: {path}",
        )

    payload = path.read_bytes()
    actual_hash = _blake3_hash(payload)
    if actual_hash != expected_hash:
        return ManifestUrlCheck(
            kind="profile_payload",
            url=str(url),
            scheme=url.scheme,
            ok=False,
            content_length=len(payload),
            expected_hash=expected_hash,
            actual_hash=actual_hash,
            failure="hash_mismatch",
            message="profile payload hash does not match manifest",
        )

    try:
        profile = _parse_local_profile_payload(path)
    except ValidationError as error:
        return ManifestUrlCheck(
            kind="profile_payload",
            url=str(url),
            scheme=url.scheme,
            ok=False,
            content_length=len(payload),
            expected_hash=expected_hash,
            actual_hash=actual_hash,
            failure="invalid_payload",
            message=str(error),
        )

    if profile.id != profile_id or profile.revision != revision:
        return ManifestUrlCheck(
            kind="profile_payload",
            url=str(url),
            scheme=url.scheme,
            ok=False,
            content_length=len(payload),
            expected_hash=expected_hash,
            actual_hash=actual_hash,
            failure="identity_mismatch",
            message=(
                f"profile payload is {profile.id}@{profile.revision}, "
                f"expected {profile_id}@{revision}"
            ),
        )

    return ManifestUrlCheck(
        kind="profile_payload",
        url=str(url),
        scheme=url.scheme,
        ok=True,
        content_length=len(payload),
        expected_hash=expected_hash,
        actual_hash=actual_hash,
    )


def _check_local_signature(url: AnyUrl) -> ManifestUrlCheck:
    path = _file_url_path(url)
    if not path.is_file():
        return ManifestUrlCheck(
            kind="profile_signature",
            url=str(url),
            scheme=url.scheme,
            ok=False,
            failure="missing",
            message=f"profile signature file not found: {path}",
        )
    return ManifestUrlCheck(
        kind="profile_signature",
        url=str(url),
        scheme=url.scheme,
        ok=True,
        content_length=path.stat().st_size,
    )


def _check_remote_head(
    kind: Literal["profile_payload", "profile_signature"],
    url: AnyUrl,
    *,
    timeout_seconds: float,
) -> ManifestUrlCheck:
    request = Request(
        str(url),
        method="HEAD",
        headers={"User-Agent": "capsem-admin/manifest-check"},
    )
    try:
        with urlopen(request, timeout=timeout_seconds) as response:
            status_code = response.status
            headers = response.headers
    except HTTPError as error:
        return ManifestUrlCheck(
            kind=kind,
            url=str(url),
            scheme=url.scheme,
            ok=False,
            status_code=error.code,
            content_length=_content_length(error.headers),
            content_type=_content_type(error.headers),
            failure="missing" if error.code == 404 else "http_error",
            message=str(error),
        )
    except (TimeoutError, URLError) as error:
        return ManifestUrlCheck(
            kind=kind,
            url=str(url),
            scheme=url.scheme,
            ok=False,
            failure="head_failed",
            message=str(error),
        )

    content_type = _content_type(headers)
    if kind == "profile_payload" and content_type is not None:
        if content_type not in _ACCEPTABLE_PROFILE_CONTENT_TYPES:
            return ManifestUrlCheck(
                kind=kind,
                url=str(url),
                scheme=url.scheme,
                ok=False,
                status_code=status_code,
                content_length=_content_length(headers),
                content_type=content_type,
                failure="content_type_mismatch",
                message=f"unexpected profile payload content type: {content_type}",
            )

    return ManifestUrlCheck(
        kind=kind,
        url=str(url),
        scheme=url.scheme,
        ok=200 <= status_code < 400,
        status_code=status_code,
        content_length=_content_length(headers),
        content_type=content_type,
        failure=None if 200 <= status_code < 400 else "http_error",
    )


def _check_url(
    kind: Literal["profile_payload", "profile_signature"],
    url: AnyUrl,
    *,
    expected_hash: str | None = None,
    profile_id: str,
    revision: str,
    timeout_seconds: float,
) -> ManifestUrlCheck:
    if url.scheme == "file":
        if kind == "profile_payload":
            if expected_hash is None:
                raise ValueError("profile payload check requires expected_hash")
            return _check_local_payload(
                url,
                expected_hash=expected_hash,
                profile_id=profile_id,
                revision=revision,
            )
        return _check_local_signature(url)

    if url.scheme in {"http", "https"}:
        return _check_remote_head(kind, url, timeout_seconds=timeout_seconds)

    return ManifestUrlCheck(
        kind=kind,
        url=str(url),
        scheme=url.scheme,
        ok=False,
        failure="invalid_scheme",
        message=f"unsupported manifest URL scheme: {url.scheme}",
    )


def _profile_checks(
    manifest: ProfileManifest,
    *,
    timeout_seconds: float,
) -> list[ManifestProfileCheck]:
    checks: list[ManifestProfileCheck] = []
    for profile_id, profile in manifest.profiles.items():
        for revision, record in profile.revisions.items():
            item_checks = [
                _check_url(
                    "profile_payload",
                    record.profile_url,
                    expected_hash=record.profile_hash,
                    profile_id=profile_id,
                    revision=revision,
                    timeout_seconds=timeout_seconds,
                ),
                _check_url(
                    "profile_signature",
                    record.profile_signature_url,
                    profile_id=profile_id,
                    revision=revision,
                    timeout_seconds=timeout_seconds,
                ),
            ]
            checks.append(
                ManifestProfileCheck(
                    profile_id=profile_id,
                    revision=revision,
                    status=record.status,
                    current=revision == profile.current_revision,
                    checks=item_checks,
                )
            )
    return checks


def check_profile_manifest_fast(
    manifest_path: Path,
    *,
    timeout_seconds: float = 5.0,
) -> ManifestCheckReport:
    manifest = validate_manifest_json(manifest_path.read_text(encoding="utf-8"))
    profiles = _profile_checks(manifest, timeout_seconds=timeout_seconds)
    return ManifestCheckReport(
        ok=all(profile.ok for profile in profiles),
        mode="fast",
        manifest_path=str(manifest_path),
        profiles=profiles,
    )


def dump_manifest_check_report_json(report: ManifestCheckReport) -> str:
    return report.model_dump_json(by_alias=True, exclude_none=True, indent=2)
