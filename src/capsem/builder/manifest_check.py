"""Fast profile manifest checks for capsem-admin."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import tempfile
from typing import Literal
from urllib.error import HTTPError, URLError
from urllib.parse import unquote
from urllib.request import Request, urlopen

import blake3
from pydantic import AnyUrl, BaseModel, ConfigDict, Field, ValidationError

from capsem.builder.profiles import (
    AssetDeclaration,
    ProfileManifest,
    ProfilePayloadV2,
    ProfileRevisionStatus,
    validate_manifest_json,
    validate_profile_json,
    validate_profile_toml,
)
from capsem.builder.manifest_crypto import verify_minisign_signature

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
    kind: Literal[
        "profile_payload",
        "profile_signature",
        "vm_asset",
        "vm_asset_signature",
    ]
    url: str
    scheme: str
    ok: bool
    arch: Literal["arm64", "x86_64"] | None = None
    asset_kind: Literal["kernel", "initrd", "rootfs"] | None = None
    download_path: str | None = None
    status_code: int | None = None
    content_length: int | None = None
    content_type: str | None = None
    expected_size: int | None = None
    actual_size: int | None = None
    expected_hash: str | None = None
    actual_hash: str | None = None
    failure: (
        Literal[
            "missing",
            "hash_mismatch",
            "identity_mismatch",
            "invalid_payload",
            "invalid_scheme",
            "size_mismatch",
            "empty_signature",
            "signature_invalid",
            "signature_tool_missing",
            "http_error",
            "head_failed",
            "download_failed",
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
    mode: Literal["fast", "download"]
    manifest_path: str
    download_dir: str | None = None
    profiles: list[ManifestProfileCheck]


@dataclass(frozen=True)
class _FetchedBytes:
    payload: bytes
    status_code: int | None = None
    content_length: int | None = None
    content_type: str | None = None


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


def _safe_download_path(
    download_dir: Path,
    *,
    profile_id: str,
    revision: str,
    kind: Literal[
        "profile_payload",
        "profile_signature",
        "vm_asset",
        "vm_asset_signature",
    ],
    url: AnyUrl,
    arch: Literal["arm64", "x86_64"] | None = None,
    asset_kind: Literal["kernel", "initrd", "rootfs"] | None = None,
) -> Path:
    basename = Path(unquote(url.path or "")).name or kind
    suffix = "".join(Path(basename).suffixes)
    digest = blake3.blake3(str(url).encode()).hexdigest()[:16]
    name_parts = [kind]
    if asset_kind is not None:
        name_parts.append(asset_kind)
    name_parts.append(digest)
    filename = "-".join(name_parts) + suffix
    parts = [download_dir, Path(profile_id), Path(revision)]
    if arch is not None:
        parts.append(Path(arch))
    target_dir = Path(*parts)
    target_dir.mkdir(parents=True, exist_ok=True)
    return target_dir / filename


def _fetch_bytes(
    kind: Literal[
        "profile_payload",
        "profile_signature",
        "vm_asset",
        "vm_asset_signature",
    ],
    url: AnyUrl,
    *,
    timeout_seconds: float,
    arch: Literal["arm64", "x86_64"] | None = None,
    asset_kind: Literal["kernel", "initrd", "rootfs"] | None = None,
) -> tuple[_FetchedBytes | None, ManifestUrlCheck | None]:
    if url.scheme == "file":
        path = _file_url_path(url)
        if not path.is_file():
            return None, ManifestUrlCheck(
                kind=kind,
                url=str(url),
                scheme=url.scheme,
                ok=False,
                arch=arch,
                asset_kind=asset_kind,
                failure="missing",
                message=f"file not found: {path}",
            )
        payload = path.read_bytes()
        return _FetchedBytes(payload=payload, content_length=len(payload)), None

    if url.scheme in {"http", "https"}:
        request = Request(
            str(url),
            method="GET",
            headers={"User-Agent": "capsem-admin/manifest-check"},
        )
        try:
            with urlopen(request, timeout=timeout_seconds) as response:
                payload = response.read()
                return (
                    _FetchedBytes(
                        payload=payload,
                        status_code=response.status,
                        content_length=_content_length(response.headers),
                        content_type=_content_type(response.headers),
                    ),
                    None,
                )
        except HTTPError as error:
            return None, ManifestUrlCheck(
                kind=kind,
                url=str(url),
                scheme=url.scheme,
                ok=False,
                arch=arch,
                asset_kind=asset_kind,
                status_code=error.code,
                content_length=_content_length(error.headers),
                content_type=_content_type(error.headers),
                failure="missing" if error.code == 404 else "http_error",
                message=str(error),
            )
        except (TimeoutError, URLError) as error:
            return None, ManifestUrlCheck(
                kind=kind,
                url=str(url),
                scheme=url.scheme,
                ok=False,
                arch=arch,
                asset_kind=asset_kind,
                failure="download_failed",
                message=str(error),
            )

    return None, ManifestUrlCheck(
        kind=kind,
        url=str(url),
        scheme=url.scheme,
        ok=False,
        arch=arch,
        asset_kind=asset_kind,
        failure="invalid_scheme",
        message=f"unsupported manifest URL scheme: {url.scheme}",
    )


def _parse_local_profile_payload(path: Path) -> ProfilePayloadV2:
    if path.suffix.lower() == ".toml":
        return validate_profile_toml(path)
    return validate_profile_json(path.read_bytes())


def _download_signature(
    kind: Literal["profile_signature", "vm_asset_signature"],
    url: AnyUrl,
    *,
    download_dir: Path,
    profile_id: str,
    revision: str,
    timeout_seconds: float,
    arch: Literal["arm64", "x86_64"] | None = None,
    asset_kind: Literal["kernel", "initrd", "rootfs"] | None = None,
    signed_payload_path: Path | None = None,
    pubkey_path: Path | None = None,
) -> ManifestUrlCheck:
    fetched, failure = _fetch_bytes(
        kind,
        url,
        timeout_seconds=timeout_seconds,
        arch=arch,
        asset_kind=asset_kind,
    )
    if failure is not None:
        return failure
    assert fetched is not None
    download_path = _safe_download_path(
        download_dir,
        profile_id=profile_id,
        revision=revision,
        kind=kind,
        url=url,
        arch=arch,
        asset_kind=asset_kind,
    )
    download_path.write_bytes(fetched.payload)
    if not fetched.payload:
        return ManifestUrlCheck(
            kind=kind,
            url=str(url),
            scheme=url.scheme,
            ok=False,
            arch=arch,
            asset_kind=asset_kind,
            download_path=str(download_path),
            status_code=fetched.status_code,
            content_length=fetched.content_length,
            content_type=fetched.content_type,
            actual_size=0,
            failure="empty_signature",
            message="signature payload is empty",
        )
    if pubkey_path is not None and signed_payload_path is not None:
        verification = verify_minisign_signature(
            signed_payload_path,
            download_path,
            pubkey_path,
        )
        if not verification.ok:
            return ManifestUrlCheck(
                kind=kind,
                url=str(url),
                scheme=url.scheme,
                ok=False,
                arch=arch,
                asset_kind=asset_kind,
                download_path=str(download_path),
                status_code=fetched.status_code,
                content_length=fetched.content_length,
                content_type=fetched.content_type,
                actual_size=len(fetched.payload),
                failure=(
                    "signature_tool_missing"
                    if verification.failure == "tool_missing"
                    else "signature_invalid"
                ),
                message=verification.stderr or verification.stdout,
            )
    return ManifestUrlCheck(
        kind=kind,
        url=str(url),
        scheme=url.scheme,
        ok=True,
        arch=arch,
        asset_kind=asset_kind,
        download_path=str(download_path),
        status_code=fetched.status_code,
        content_length=fetched.content_length,
        content_type=fetched.content_type,
        actual_size=len(fetched.payload),
    )


def _download_profile_payload(
    url: AnyUrl,
    *,
    expected_hash: str,
    profile_id: str,
    revision: str,
    download_dir: Path,
    timeout_seconds: float,
) -> tuple[ManifestUrlCheck, ProfilePayloadV2 | None]:
    fetched, failure = _fetch_bytes(
        "profile_payload",
        url,
        timeout_seconds=timeout_seconds,
    )
    if failure is not None:
        return failure.model_copy(update={"expected_hash": expected_hash}), None
    assert fetched is not None

    download_path = _safe_download_path(
        download_dir,
        profile_id=profile_id,
        revision=revision,
        kind="profile_payload",
        url=url,
    )
    download_path.write_bytes(fetched.payload)
    actual_hash = _blake3_hash(fetched.payload)
    if actual_hash != expected_hash:
        return (
            ManifestUrlCheck(
                kind="profile_payload",
                url=str(url),
                scheme=url.scheme,
                ok=False,
                download_path=str(download_path),
                status_code=fetched.status_code,
                content_length=fetched.content_length,
                content_type=fetched.content_type,
                expected_size=fetched.content_length,
                actual_size=len(fetched.payload),
                expected_hash=expected_hash,
                actual_hash=actual_hash,
                failure="hash_mismatch",
                message="profile payload hash does not match manifest",
            ),
            None,
        )

    try:
        profile = _parse_local_profile_payload(download_path)
    except ValidationError as error:
        return (
            ManifestUrlCheck(
                kind="profile_payload",
                url=str(url),
                scheme=url.scheme,
                ok=False,
                download_path=str(download_path),
                status_code=fetched.status_code,
                content_length=fetched.content_length,
                content_type=fetched.content_type,
                expected_hash=expected_hash,
                actual_hash=actual_hash,
                actual_size=len(fetched.payload),
                failure="invalid_payload",
                message=str(error),
            ),
            None,
        )

    if profile.id != profile_id or profile.revision != revision:
        return (
            ManifestUrlCheck(
                kind="profile_payload",
                url=str(url),
                scheme=url.scheme,
                ok=False,
                download_path=str(download_path),
                status_code=fetched.status_code,
                content_length=fetched.content_length,
                content_type=fetched.content_type,
                expected_hash=expected_hash,
                actual_hash=actual_hash,
                actual_size=len(fetched.payload),
                failure="identity_mismatch",
                message=(
                    f"profile payload is {profile.id}@{profile.revision}, "
                    f"expected {profile_id}@{revision}"
                ),
            ),
            None,
        )

    return (
        ManifestUrlCheck(
            kind="profile_payload",
            url=str(url),
            scheme=url.scheme,
            ok=True,
            download_path=str(download_path),
            status_code=fetched.status_code,
            content_length=fetched.content_length,
            content_type=fetched.content_type,
            expected_hash=expected_hash,
            actual_hash=actual_hash,
            actual_size=len(fetched.payload),
        ),
        profile,
    )


def _download_vm_asset(
    url: AnyUrl,
    *,
    asset: AssetDeclaration,
    profile_id: str,
    revision: str,
    arch: Literal["arm64", "x86_64"],
    asset_kind: Literal["kernel", "initrd", "rootfs"],
    download_dir: Path,
    timeout_seconds: float,
) -> ManifestUrlCheck:
    fetched, failure = _fetch_bytes(
        "vm_asset",
        url,
        timeout_seconds=timeout_seconds,
        arch=arch,
        asset_kind=asset_kind,
    )
    if failure is not None:
        return failure.model_copy(
            update={"expected_hash": asset.hash, "expected_size": asset.size}
        )
    assert fetched is not None

    download_path = _safe_download_path(
        download_dir,
        profile_id=profile_id,
        revision=revision,
        kind="vm_asset",
        url=url,
        arch=arch,
        asset_kind=asset_kind,
    )
    download_path.write_bytes(fetched.payload)
    actual_size = len(fetched.payload)
    actual_hash = _blake3_hash(fetched.payload)
    if actual_size != asset.size:
        failure_kind: Literal["size_mismatch", "hash_mismatch"] = "size_mismatch"
        message = "VM asset size does not match profile declaration"
    elif actual_hash != asset.hash:
        failure_kind = "hash_mismatch"
        message = "VM asset hash does not match profile declaration"
    else:
        failure_kind = None
        message = None

    return ManifestUrlCheck(
        kind="vm_asset",
        url=str(url),
        scheme=url.scheme,
        ok=failure_kind is None,
        arch=arch,
        asset_kind=asset_kind,
        download_path=str(download_path),
        status_code=fetched.status_code,
        content_length=fetched.content_length,
        content_type=fetched.content_type,
        expected_size=asset.size,
        actual_size=actual_size,
        expected_hash=asset.hash,
        actual_hash=actual_hash,
        failure=failure_kind,
        message=message,
    )


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


def _profile_download_checks(
    manifest: ProfileManifest,
    *,
    download_dir: Path,
    timeout_seconds: float,
    pubkey_path: Path | None = None,
) -> list[ManifestProfileCheck]:
    checks: list[ManifestProfileCheck] = []
    for profile_id, manifest_profile in manifest.profiles.items():
        for revision, record in manifest_profile.revisions.items():
            payload_check, profile = _download_profile_payload(
                record.profile_url,
                expected_hash=record.profile_hash,
                profile_id=profile_id,
                revision=revision,
                download_dir=download_dir,
                timeout_seconds=timeout_seconds,
            )
            item_checks = [
                payload_check,
                _download_signature(
                    "profile_signature",
                    record.profile_signature_url,
                    download_dir=download_dir,
                    profile_id=profile_id,
                    revision=revision,
                    timeout_seconds=timeout_seconds,
                    signed_payload_path=(
                        Path(payload_check.download_path)
                        if payload_check.ok and payload_check.download_path is not None
                        else None
                    ),
                    pubkey_path=pubkey_path,
                ),
            ]
            if profile is not None:
                for arch, assets in profile.vm.assets.items():
                    for asset_kind, asset in (
                        ("kernel", assets.kernel),
                        ("initrd", assets.initrd),
                        ("rootfs", assets.rootfs),
                    ):
                        item_checks.append(
                            _download_vm_asset(
                                asset.url,
                                asset=asset,
                                profile_id=profile_id,
                                revision=revision,
                                arch=arch,
                                asset_kind=asset_kind,
                                download_dir=download_dir,
                                timeout_seconds=timeout_seconds,
                            )
                        )
                        item_checks.append(
                            _download_signature(
                                "vm_asset_signature",
                                asset.signature_url,
                                download_dir=download_dir,
                                profile_id=profile_id,
                                revision=revision,
                                timeout_seconds=timeout_seconds,
                                arch=arch,
                                asset_kind=asset_kind,
                                signed_payload_path=(
                                    Path(item_checks[-1].download_path)
                                    if item_checks[-1].ok
                                    and item_checks[-1].download_path is not None
                                    else None
                                ),
                                pubkey_path=pubkey_path,
                            )
                        )
            checks.append(
                ManifestProfileCheck(
                    profile_id=profile_id,
                    revision=revision,
                    status=record.status,
                    current=revision == manifest_profile.current_revision,
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


def check_profile_manifest_download(
    manifest_path: Path,
    *,
    download_dir: Path | None = None,
    timeout_seconds: float = 30.0,
    pubkey_path: Path | None = None,
) -> ManifestCheckReport:
    manifest = validate_manifest_json(manifest_path.read_text(encoding="utf-8"))
    resolved_download_dir = download_dir or Path(
        tempfile.mkdtemp(prefix="capsem-manifest-check-")
    )
    resolved_download_dir.mkdir(parents=True, exist_ok=True)
    profiles = _profile_download_checks(
        manifest,
        download_dir=resolved_download_dir,
        timeout_seconds=timeout_seconds,
        pubkey_path=pubkey_path,
    )
    return ManifestCheckReport(
        ok=all(profile.ok for profile in profiles),
        mode="download",
        manifest_path=str(manifest_path),
        download_dir=str(resolved_download_dir),
        profiles=profiles,
    )


def dump_manifest_check_report_json(report: ManifestCheckReport) -> str:
    return report.model_dump_json(by_alias=True, exclude_none=True, indent=2)
