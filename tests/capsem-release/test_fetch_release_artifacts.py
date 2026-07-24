from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path

import blake3
import pytest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "fetch-release-artifacts.py"
SPEC = importlib.util.spec_from_file_location("fetch_release_artifacts", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
FETCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FETCH)
VERIFY_SPEC = importlib.util.spec_from_file_location(
    "verify_release_inputs", ROOT / "scripts" / "verify-release-inputs.py"
)
assert VERIFY_SPEC is not None and VERIFY_SPEC.loader is not None
VERIFY = importlib.util.module_from_spec(VERIFY_SPEC)
VERIFY_SPEC.loader.exec_module(VERIFY)
STAGE_SPEC = importlib.util.spec_from_file_location(
    "stage_release_test_inputs", ROOT / "scripts" / "stage-release-test-inputs.py"
)
assert STAGE_SPEC is not None and STAGE_SPEC.loader is not None
STAGE = importlib.util.module_from_spec(STAGE_SPEC)
STAGE_SPEC.loader.exec_module(STAGE)


def _digest(payload: bytes) -> dict[str, str]:
    return {
        "sha256": hashlib.sha256(payload).hexdigest(),
        "blake3": blake3.blake3(payload).hexdigest(),
    }


def _record(url: str, payload: bytes, **extra: object) -> dict[str, object]:
    return {
        "url": url,
        "bytes": len(payload),
        "digest": _digest(payload),
        **extra,
    }


def _write_manifest(tmp_path: Path) -> tuple[Path, dict[str, bytes]]:
    artifacts = {
        "capsem.deb": b"package",
        "profile.toml": b"[profile]\nid='code'\n",
        "vmlinuz": b"kernel",
        "initrd.img": b"initrd",
        "rootfs.erofs": b"rootfs",
        "obom.cdx.json": b'{"bomFormat":"CycloneDX"}',
    }
    for name, payload in artifacts.items():
        (tmp_path / name).write_bytes(payload)

    manifest = {
        "version": "1.0.0",
        "channel": "nightly",
        "status": "current",
        "packages": [
            _record(
                "capsem.deb",
                artifacts["capsem.deb"],
                name="capsem.deb",
                status="current",
            )
        ],
        "profiles": {
            "code": {
                "version": "code-1",
                "id": "code",
                "name": "Code",
                "revision": "code-1",
                "status": "current",
                "architectures": [
                    {
                        "architecture": "x86_64",
                        "config": [
                            _record(
                                "profile.toml",
                                artifacts["profile.toml"],
                                kind="profile",
                                path="profiles/code/profile.toml",
                                status="current",
                            )
                        ],
                        "images": [
                            _record(
                                name,
                                artifacts[name],
                                kind=kind,
                                name=name,
                                status="current",
                            )
                            for name, kind in (
                                ("vmlinuz", "kernel"),
                                ("initrd.img", "initrd"),
                                ("rootfs.erofs", "rootfs"),
                            )
                        ],
                        "evidence": [
                            _record(
                                "obom.cdx.json",
                                artifacts["obom.cdx.json"],
                                kind="obom",
                                status="current",
                            )
                        ],
                    }
                ],
            }
        },
    }
    path = tmp_path / "manifest.json"
    path.write_text(json.dumps(manifest), encoding="utf-8")
    return path, artifacts


def test_fetches_only_current_packages_and_verifies_both_digests(
    tmp_path: Path,
) -> None:
    manifest, artifacts = _write_manifest(tmp_path)
    output = tmp_path / "packages"

    report = FETCH.fetch_release_inputs(manifest.as_uri(), "packages", output)

    assert report["kind"] == "packages"
    assert (output / "capsem.deb").read_bytes() == artifacts["capsem.deb"]
    assert (output / "manifest.json").read_bytes() == manifest.read_bytes()
    assert (output / "release-inputs.json").is_file()
    assert len(report["artifacts"]) == 1


def test_fetches_every_profile_owned_input(tmp_path: Path) -> None:
    manifest, artifacts = _write_manifest(tmp_path)
    output = tmp_path / "profiles"

    report = FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", output)

    paths = {row["path"] for row in report["artifacts"]}
    assert paths == {
        "profiles/code/x86_64/config/profile.toml",
        "profiles/code/x86_64/images/vmlinuz",
        "profiles/code/x86_64/images/initrd.img",
        "profiles/code/x86_64/images/rootfs.erofs",
        "profiles/code/x86_64/evidence/obom.cdx.json",
    }
    assert (
        output / "profiles/code/x86_64/images/rootfs.erofs"
    ).read_bytes() == artifacts["rootfs.erofs"]


@pytest.mark.parametrize("field", ["sha256", "blake3"])
def test_rejects_tampered_profile_digest(tmp_path: Path, field: str) -> None:
    manifest, _ = _write_manifest(tmp_path)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    document["profiles"]["code"]["architectures"][0]["images"][0]["digest"][
        field
    ] = "0" * 64
    manifest.write_text(json.dumps(document), encoding="utf-8")

    with pytest.raises(ValueError, match=field.replace("sha256", "SHA-256").replace("blake3", "BLAKE3")):
        FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", tmp_path / "out")


def test_rejects_manifest_without_owned_inputs(tmp_path: Path) -> None:
    manifest = tmp_path / "manifest.json"
    manifest.write_text('{"packages":[],"profiles":{}}', encoding="utf-8")

    with pytest.raises(ValueError, match="no packages"):
        FETCH.fetch_release_inputs(manifest.as_uri(), "packages", tmp_path / "out")


def test_verifier_rejects_a_tampered_resolved_input(tmp_path: Path) -> None:
    manifest, _ = _write_manifest(tmp_path)
    output = tmp_path / "packages"
    FETCH.fetch_release_inputs(manifest.as_uri(), "packages", output)
    (output / "capsem.deb").write_bytes(b"tampered")

    with pytest.raises(ValueError, match="byte size mismatch"):
        VERIFY.verify_release_inputs(output)


def test_stages_verified_profile_manifest_and_host_boot_images(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    manifest, artifacts = _write_manifest(tmp_path)
    inputs = tmp_path / "profile-inputs"
    FETCH.fetch_release_inputs(manifest.as_uri(), "profiles", inputs)
    monkeypatch.setattr(STAGE, "_host_arch", lambda: "x86_64")
    assets = tmp_path / "assets"

    staged_manifest = STAGE.stage_profiles(inputs, assets)

    document = json.loads(staged_manifest.read_text(encoding="utf-8"))
    image_url = document["profiles"]["code"]["architectures"][0]["images"][0][
        "url"
    ]
    assert image_url.startswith("file://")
    assert (assets / "x86_64/vmlinuz").read_bytes() == artifacts["vmlinuz"]
    assert (assets / "x86_64/initrd.img").read_bytes() == artifacts["initrd.img"]
    assert (assets / "x86_64/rootfs.erofs").read_bytes() == artifacts["rootfs.erofs"]
