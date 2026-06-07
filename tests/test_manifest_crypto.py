from __future__ import annotations

from pathlib import Path
import shutil
import subprocess

import blake3
import pytest
from click.testing import CliRunner

from capsem.admin.cli import cli
from capsem.builder.profiles import (
    ArchAssets,
    AssetDeclaration,
    create_profile_draft,
    dump_profile_json,
)


def _require_minisign() -> None:
    if shutil.which("minisign") is None:
        pytest.skip("minisign not installed")


def _run_minisign(*args: str) -> None:
    result = subprocess.run(
        ["minisign", *args],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr or result.stdout


def _keypair(tmp_path: Path) -> tuple[Path, Path]:
    secret = tmp_path / "profile.key"
    public = tmp_path / "profile.pub"
    _run_minisign("-G", "-f", "-W", "-s", str(secret), "-p", str(public))
    return secret, public


def _sign(path: Path, secret: Path) -> Path:
    signature = path.with_name(path.name + ".minisig")
    _run_minisign("-S", "-s", str(secret), "-m", str(path), "-x", str(signature))
    return signature


def _blake3(payload: bytes) -> str:
    return f"blake3:{blake3.blake3(payload).hexdigest()}"


def _asset(path: Path, payload: bytes, secret: Path) -> AssetDeclaration:
    path.write_bytes(payload)
    signature = _sign(path, secret)
    return AssetDeclaration(
        url=path.as_uri(),
        hash=_blake3(payload),
        signature_url=signature.as_uri(),
        size=len(payload),
        content_type="application/octet-stream",
    )


def _write_signed_profile_tree(tmp_path: Path) -> tuple[Path, Path, Path]:
    secret, public = _keypair(tmp_path)
    assets_dir = tmp_path / "assets"
    profiles_dir = tmp_path / "profiles"
    assets_dir.mkdir()
    profiles_dir.mkdir()
    profile = create_profile_draft("corp-dev", revision="2026.0520.16")
    profile = profile.model_copy(
        update={
            "vm": profile.vm.model_copy(
                update={
                    "assets": {
                        "arm64": ArchAssets(
                            kernel=_asset(assets_dir / "vmlinuz", b"kernel", secret),
                            initrd=_asset(assets_dir / "initrd.img", b"initrd", secret),
                            rootfs=_asset(
                                assets_dir / "rootfs.squashfs",
                                b"rootfs",
                                secret,
                            ),
                        )
                    }
                }
            )
        }
    )
    profile_path = profiles_dir / "corp-dev.json"
    profile_path.write_text(dump_profile_json(profile), encoding="utf-8")
    _sign(profile_path, secret)
    return profiles_dir, secret, public


def test_manifest_check_download_verifies_minisign_profile_and_asset_signatures(
    tmp_path: Path,
) -> None:
    _require_minisign()
    profiles_dir, _, public = _write_signed_profile_tree(tmp_path)
    manifest_path = tmp_path / "manifest.json"
    generate_result = CliRunner().invoke(
        cli,
        [
            "manifest",
            "generate",
            "--profiles",
            str(profiles_dir),
            "--out",
            str(manifest_path),
        ],
    )
    assert generate_result.exit_code == 0

    result = CliRunner().invoke(
        cli,
        [
            "manifest",
            "check",
            str(manifest_path),
            "--download",
            "--download-dir",
            str(tmp_path / "downloads"),
            "--pubkey",
            str(public),
            "--json",
        ],
    )

    assert result.exit_code == 0
    assert '"ok": true' in result.output
    assert '"kind": "profile_signature"' in result.output
    assert '"kind": "vm_asset_signature"' in result.output
    assert "signature_invalid" not in result.output


def test_manifest_check_download_rejects_bad_minisign_signature(tmp_path: Path) -> None:
    _require_minisign()
    profiles_dir, _, public = _write_signed_profile_tree(tmp_path)
    profile_signature = profiles_dir / "corp-dev.json.minisig"
    profile_signature.write_text("not a minisign signature\n", encoding="utf-8")
    manifest_path = tmp_path / "manifest.json"
    generate_result = CliRunner().invoke(
        cli,
        [
            "manifest",
            "generate",
            "--profiles",
            str(profiles_dir),
            "--out",
            str(manifest_path),
        ],
    )
    assert generate_result.exit_code == 0

    result = CliRunner().invoke(
        cli,
        [
            "manifest",
            "check",
            str(manifest_path),
            "--download",
            "--pubkey",
            str(public),
            "--json",
        ],
    )

    assert result.exit_code == 1
    assert '"failure": "signature_invalid"' in result.output


def test_capsem_admin_manifest_sign_creates_minisign_signature(tmp_path: Path) -> None:
    _require_minisign()
    secret, public = _keypair(tmp_path)
    manifest_path = tmp_path / "manifest.json"
    signature_path = tmp_path / "manifest.json.minisig"
    manifest_path.write_text('{"format":1,"profiles":{}}\n', encoding="utf-8")

    result = CliRunner().invoke(
        cli,
        [
            "manifest",
            "sign",
            str(manifest_path),
            "--key",
            str(secret),
            "--out",
            str(signature_path),
            "--json",
        ],
    )

    assert result.exit_code == 0
    assert '"schema": "capsem.manifest-sign.v1"' in result.output
    assert signature_path.is_file()
    verify = CliRunner().invoke(
        cli,
        [
            "manifest",
            "verify-signature",
            str(manifest_path),
            "--signature",
            str(signature_path),
            "--pubkey",
            str(public),
            "--json",
        ],
    )
    assert verify.exit_code == 0
    assert '"schema": "capsem.manifest-signature-verification.v1"' in verify.output
    assert '"ok": true' in verify.output
