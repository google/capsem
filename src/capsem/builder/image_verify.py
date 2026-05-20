"""Local image asset verification for profile-derived image plans."""

from __future__ import annotations

from pathlib import Path
from typing import Literal

import blake3
from pydantic import BaseModel, ConfigDict, Field

from capsem.builder.image_plan import ImagePlan
from capsem.builder.profiles import AssetDeclaration


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, populate_by_name=True)


class ImageAssetVerification(StrictModel):
    arch: Literal["arm64", "x86_64"]
    kind: Literal["kernel", "initrd", "rootfs"]
    path: str
    exists: bool
    expected_size: int
    actual_size: int | None = None
    expected_hash: str
    actual_hash: str | None = None
    ok: bool
    failure: Literal["missing", "size_mismatch", "hash_mismatch"] | None = None


class ImageVerificationReport(StrictModel):
    schema_: Literal["capsem.image-verification.v1"] = Field(
        default="capsem.image-verification.v1",
        alias="schema",
    )
    ok: bool
    profile_id: str
    profile_revision: str
    assets_dir: str
    assets: list[ImageAssetVerification]


def _blake3_hash(path: Path) -> str:
    hasher = blake3.blake3()
    with path.open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            hasher.update(chunk)
    return f"blake3:{hasher.hexdigest()}"


def _asset_filename(asset: AssetDeclaration) -> str:
    path = asset.url.path or ""
    filename = Path(path).name
    if not filename:
        raise ValueError(f"asset URL '{asset.url}' does not include a filename")
    return filename


def _verify_one(
    assets_dir: Path,
    arch: Literal["arm64", "x86_64"],
    kind: Literal["kernel", "initrd", "rootfs"],
    asset: AssetDeclaration,
) -> ImageAssetVerification:
    path = assets_dir / arch / _asset_filename(asset)
    expected_hash = asset.hash
    expected_size = asset.size
    if not path.exists():
        return ImageAssetVerification(
            arch=arch,
            kind=kind,
            path=str(path),
            exists=False,
            expected_size=expected_size,
            expected_hash=expected_hash,
            ok=False,
            failure="missing",
        )

    actual_size = path.stat().st_size
    actual_hash = _blake3_hash(path)
    if actual_size != expected_size:
        failure: Literal["size_mismatch", "hash_mismatch"] = "size_mismatch"
    elif actual_hash != expected_hash:
        failure = "hash_mismatch"
    else:
        failure = None
    return ImageAssetVerification(
        arch=arch,
        kind=kind,
        path=str(path),
        exists=True,
        expected_size=expected_size,
        actual_size=actual_size,
        expected_hash=expected_hash,
        actual_hash=actual_hash,
        ok=failure is None,
        failure=failure,
    )


def verify_image_assets(plan: ImagePlan, assets_dir: Path) -> ImageVerificationReport:
    assets: list[ImageAssetVerification] = []
    for arch in plan.arches:
        assets.extend(
            [
                _verify_one(
                    assets_dir,
                    arch.arch,
                    "kernel",
                    arch.declared_assets.kernel,
                ),
                _verify_one(
                    assets_dir,
                    arch.arch,
                    "initrd",
                    arch.declared_assets.initrd,
                ),
                _verify_one(
                    assets_dir,
                    arch.arch,
                    "rootfs",
                    arch.declared_assets.rootfs,
                ),
            ]
        )

    return ImageVerificationReport(
        ok=all(asset.ok for asset in assets),
        profile_id=plan.profile_id,
        profile_revision=plan.profile_revision,
        assets_dir=str(assets_dir),
        assets=assets,
    )


def dump_image_verification_report_json(report: ImageVerificationReport) -> str:
    return report.model_dump_json(by_alias=True, exclude_none=True, indent=2)
