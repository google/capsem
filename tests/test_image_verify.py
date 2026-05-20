from __future__ import annotations

from pathlib import Path

import blake3
import pytest
from click.testing import CliRunner

from capsem.admin.cli import cli
from capsem.builder.image_plan import ImagePlan, derive_image_plan
from capsem.builder.image_verify import (
    ImageVerificationReport,
    dump_image_verification_report_json,
    verify_image_assets,
)
from capsem.builder.profiles import (
    ArchAssets,
    AssetDeclaration,
    ProfilePayloadV2,
    create_profile_draft,
    dump_profile_json,
)


def _asset(url: str, data: bytes) -> AssetDeclaration:
    return AssetDeclaration(
        url=url,
        hash=f"blake3:{blake3.blake3(data).hexdigest()}",
        signature_url=f"{url}.minisig",
        size=len(data),
        content_type="application/octet-stream",
    )


def _profile_with_local_asset_contract() -> tuple[
    ProfilePayloadV2,
    ImagePlan,
    dict[tuple[str, str], bytes],
]:
    payloads: dict[tuple[str, str], bytes] = {}
    assets: dict[str, ArchAssets] = {}
    for arch in ("arm64", "x86_64"):
        kernel = f"kernel-{arch}".encode()
        initrd = f"initrd-{arch}".encode()
        rootfs = f"rootfs-{arch}".encode()
        payloads[(arch, "vmlinuz")] = kernel
        payloads[(arch, "initrd.img")] = initrd
        payloads[(arch, "rootfs.squashfs")] = rootfs
        assets[arch] = ArchAssets(
            kernel=_asset(f"https://assets.example.invalid/{arch}/vmlinuz", kernel),
            initrd=_asset(f"https://assets.example.invalid/{arch}/initrd.img", initrd),
            rootfs=_asset(
                f"https://assets.example.invalid/{arch}/rootfs.squashfs",
                rootfs,
            ),
        )
    draft = create_profile_draft("corp-dev", revision="2026.0520.11")
    profile = draft.model_copy(
        update={
            "vm": draft.vm.model_copy(update={"assets": assets})
        }
    )
    return profile, derive_image_plan(profile), payloads


def _write_assets(assets_dir: Path, payloads: dict[tuple[str, str], bytes]) -> None:
    for (arch, filename), payload in payloads.items():
        target = assets_dir / arch / filename
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(payload)


def test_verify_image_assets_accepts_matching_local_assets(tmp_path: Path) -> None:
    _, plan, payloads = _profile_with_local_asset_contract()
    _write_assets(tmp_path, payloads)

    report = verify_image_assets(plan, tmp_path)
    dumped = dump_image_verification_report_json(report)
    reparsed = ImageVerificationReport.model_validate_json(dumped)

    assert report == reparsed
    assert report.ok is True
    assert report.profile_id == "corp-dev"
    assert len(report.assets) == 6
    assert {asset.kind for asset in report.assets} == {"kernel", "initrd", "rootfs"}


def test_verify_image_assets_reports_missing_asset(tmp_path: Path) -> None:
    _, plan, payloads = _profile_with_local_asset_contract()
    payloads.pop(("arm64", "rootfs.squashfs"))
    _write_assets(tmp_path, payloads)

    report = verify_image_assets(plan, tmp_path)

    assert report.ok is False
    missing = [asset for asset in report.assets if not asset.exists]
    assert len(missing) == 1
    assert missing[0].arch == "arm64"
    assert missing[0].kind == "rootfs"
    assert missing[0].failure == "missing"


def test_verify_image_assets_reports_hash_mismatch(tmp_path: Path) -> None:
    _, plan, payloads = _profile_with_local_asset_contract()
    payloads[("x86_64", "vmlinuz")] = b"x" * len(payloads[("x86_64", "vmlinuz")])
    _write_assets(tmp_path, payloads)

    report = verify_image_assets(plan, tmp_path)

    assert report.ok is False
    mismatch = [asset for asset in report.assets if asset.failure == "hash_mismatch"]
    assert len(mismatch) == 1
    assert mismatch[0].arch == "x86_64"
    assert mismatch[0].kind == "kernel"


def test_verify_image_assets_rejects_url_without_filename(tmp_path: Path) -> None:
    _, plan, _ = _profile_with_local_asset_contract()
    bad_plan = plan.model_copy(
        update={
            "arches": [
                plan.arches[0].model_copy(
                    update={
                        "declared_assets": plan.arches[0].declared_assets.model_copy(
                            update={
                                "kernel": AssetDeclaration(
                                    url="https://assets.example.invalid/",
                                    hash="blake3:" + "a" * 64,
                                    signature_url="https://assets.example.invalid/kernel.minisig",
                                    size=1,
                                    content_type="application/octet-stream",
                                )
                            }
                        )
                    }
                )
            ]
        }
    )

    with pytest.raises(ValueError, match="does not include a filename"):
        verify_image_assets(bad_plan, tmp_path)


def test_capsem_admin_image_verify_accepts_matching_assets(tmp_path: Path) -> None:
    profile, _, payloads = _profile_with_local_asset_contract()
    profile_path = tmp_path / "profile.json"
    profile_path.write_text(dump_profile_json(profile), encoding="utf-8")
    assets_dir = tmp_path / "assets"
    _write_assets(assets_dir, payloads)

    result = CliRunner().invoke(
        cli,
        ["image", "verify", str(profile_path), "--assets-dir", str(assets_dir), "--json"],
    )

    assert result.exit_code == 0
    assert '"schema": "capsem.image-verification.v1"' in result.output
    assert '"ok": true' in result.output
    assert '"profile_id": "corp-dev"' in result.output


def test_capsem_admin_image_verify_returns_nonzero_on_mismatch(tmp_path: Path) -> None:
    profile, _, payloads = _profile_with_local_asset_contract()
    profile_path = tmp_path / "profile.json"
    profile_path.write_text(dump_profile_json(profile), encoding="utf-8")
    assets_dir = tmp_path / "assets"
    payloads[("arm64", "initrd.img")] = b"x" * len(payloads[("arm64", "initrd.img")])
    _write_assets(assets_dir, payloads)

    result = CliRunner().invoke(
        cli,
        ["image", "verify", str(profile_path), "--assets-dir", str(assets_dir), "--json"],
    )

    assert result.exit_code == 1
    assert '"ok": false' in result.output
    assert '"failure": "hash_mismatch"' in result.output
