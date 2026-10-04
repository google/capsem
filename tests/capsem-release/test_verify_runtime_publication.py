from __future__ import annotations

import hashlib
import json
from pathlib import Path

import blake3
import pytest
from capsem_builder.release.tools import stage_profile_publication as STAGE
from capsem_builder.release.tools import verify_profile_publication as VERIFY

ROOT = Path(__file__).resolve().parents[2]


def _record(url: str, payload: bytes, *, name: str | None = None) -> dict[str, object]:
    record: dict[str, object] = {
        "url": url,
        "bytes": len(payload),
        "digest": {
            "sha256": hashlib.sha256(payload).hexdigest(),
            "blake3": blake3.blake3(payload).hexdigest(),
        },
    }
    if name is not None:
        record["name"] = name
    return record


def _publication(tmp_path: Path) -> tuple[Path, Path, str]:
    base = "https://github.com/google/capsem/releases/download/profile-nightly-code-r1"
    release_dir = tmp_path / "profile-nightly-code-r1"
    release_dir.mkdir()
    config = b"id = 'code'\n"
    kernel = b"kernel"
    obom = b'{"bomFormat":"CycloneDX"}'
    software_inventory = (
        b'{"schema":"capsem.profile_software_inventory.v1","architecture":"x86_64",'
        b'"packages":[{"name":"python","version":"3.12.11","source":"apt"}]}'
    )
    files = {
        "x86_64-profile.toml": config,
        "x86_64-vmlinuz": kernel,
        "x86_64-obom.cdx.json": obom,
        "x86_64-software-inventory.json": software_inventory,
    }
    for name, payload in files.items():
        (release_dir / name).write_bytes(payload)
    manifest = {
        "channel": "nightly",
        "packages": [],
        "profiles": {
            "code": {
                "id": "code",
                "revision": "r1",
                "architectures": [
                    {
                        "architecture": "x86_64",
                        "config": [_record(f"{base}/x86_64-profile.toml", config)],
                        "images": [_record(f"{base}/x86_64-vmlinuz", kernel, name="vmlinuz")],
                        "evidence": [
                            {
                                **_record(f"{base}/x86_64-obom.cdx.json", obom),
                                "kind": "obom",
                            },
                            {
                                **_record(
                                    f"{base}/x86_64-software-inventory.json",
                                    software_inventory,
                                ),
                                "kind": "software_inventory",
                            },
                        ],
                        "software": [
                            {
                                "name": "python",
                                "version": "3.12.11",
                                "source": "apt",
                                "architecture": "x86_64",
                                "evidence": (f"{base}/x86_64-software-inventory.json"),
                                "digest": {
                                    "sha256": "a" * 64,
                                    "blake3": "b" * 64,
                                },
                            }
                        ],
                    }
                ],
            }
        },
    }
    source = release_dir / "channel-source-nightly.json"
    source.write_text(json.dumps(manifest), encoding="utf-8")
    return source, release_dir, base


def test_profile_publication_exactly_matches_manifest(tmp_path: Path) -> None:
    source, release_dir, base = _publication(tmp_path)

    verified = VERIFY.verify_profile_publication(source, "code", base, release_dir)

    assert {path.name for path in verified} == {
        "x86_64-profile.toml",
        "x86_64-vmlinuz",
        "x86_64-obom.cdx.json",
        "x86_64-software-inventory.json",
    }


def test_profile_publication_rejects_unresolvable_software_evidence(
    tmp_path: Path,
) -> None:
    source, release_dir, base = _publication(tmp_path)
    manifest = json.loads(source.read_text(encoding="utf-8"))
    software = manifest["profiles"]["code"]["architectures"][0]["software"]
    software[0]["evidence"] = f"{base}/asset-revision/x86_64-software-inventory.json"
    source.write_text(json.dumps(manifest), encoding="utf-8")

    with pytest.raises(ValueError, match="software evidence"):
        VERIFY.verify_profile_publication(source, "code", base, release_dir)


def test_profile_publication_rejects_tamper_and_extra_files(tmp_path: Path) -> None:
    source, release_dir, base = _publication(tmp_path)
    (release_dir / "x86_64-vmlinuz").write_bytes(b"tampered")

    with pytest.raises(ValueError, match=r"metadata mismatch|SHA-256 mismatch"):
        VERIFY.verify_profile_publication(source, "code", base, release_dir)

    (release_dir / "x86_64-vmlinuz").write_bytes(b"kernel")
    (release_dir / "unexpected.txt").write_text("extra", encoding="utf-8")
    with pytest.raises(ValueError, match="file set mismatch"):
        VERIFY.verify_profile_publication(source, "code", base, release_dir)


def test_profile_publication_allows_identical_logical_paths_to_share_one_blob(
    tmp_path: Path,
) -> None:
    source, release_dir, base = _publication(tmp_path)
    manifest = json.loads(source.read_text(encoding="utf-8"))
    config = manifest["profiles"]["code"]["architectures"][0]["config"]
    duplicate = dict(config[0])
    duplicate["path"] = "profiles/code/root/root/.profile"
    config.append(duplicate)
    source.write_text(json.dumps(manifest), encoding="utf-8")

    VERIFY.verify_profile_publication(source, "code", base, release_dir)

    duplicate["bytes"] += 1
    source.write_text(json.dumps(manifest), encoding="utf-8")
    with pytest.raises(ValueError, match="conflicting metadata"):
        VERIFY.verify_profile_publication(source, "code", base, release_dir)


def test_profile_publication_stages_only_manifest_described_inputs(
    tmp_path: Path,
) -> None:
    base = "https://github.com/google/capsem/releases/download/profile-nightly-code-r1"
    assets = tmp_path / "assets" / "x86_64"
    config = tmp_path / "config" / "profiles" / "code"
    assets.mkdir(parents=True)
    config.mkdir(parents=True)
    kernel = b"kernel"
    profile = b"id = 'code'\n"
    root_manifest = (
        b'{"format":"capsem.profile-root.v1","files":'
        b'[{"path":"root/.profile","hash":"blake3:test","size":8}]}\n'
    )
    root_payload = b"profile\n"
    (assets / "vmlinuz").write_bytes(kernel)
    (assets / "build-ledger.log").write_text("must not publish", encoding="utf-8")
    (config / "profile.toml").write_bytes(profile)
    (config / "root/root").mkdir(parents=True)
    (config / "root.manifest.json").write_bytes(root_manifest)
    (config / "root/root/.profile").write_bytes(root_payload)
    manifest = {
        "channel": "nightly",
        "packages": [],
        "profiles": {
            "code": {
                "id": "code",
                "revision": "r1",
                "architectures": [
                    {
                        "architecture": "x86_64",
                        "config": [
                            {
                                **_record(f"{base}/x86_64-profile.toml", profile),
                                "path": "profiles/code/profile.toml",
                            },
                            {
                                **_record(
                                    f"{base}/x86_64-root.manifest.json",
                                    root_manifest,
                                ),
                                "kind": "root_manifest",
                                "path": "profiles/code/root.manifest.json",
                            },
                            {
                                **_record(
                                    f"{base}/x86_64-root-payload-test",
                                    root_payload,
                                ),
                                "kind": "root_payload",
                                "path": "profiles/code/root/root/.profile",
                            },
                        ],
                        "images": [_record(f"{base}/x86_64-vmlinuz", kernel, name="vmlinuz")],
                        "evidence": [],
                        "software": [],
                    }
                ],
            }
        },
    }
    source = tmp_path / "source.json"
    source.write_text(json.dumps(manifest), encoding="utf-8")
    release_dir = tmp_path / "publication"

    staged = STAGE.stage_profile_publication(
        source,
        "code",
        tmp_path / "assets",
        tmp_path / "config",
        release_dir,
    )

    assert {path.name for path in staged} == {
        "x86_64-profile.toml",
        "x86_64-root.manifest.json",
        "x86_64-root-payload-test",
        "x86_64-vmlinuz",
        "channel-source-nightly.json",
    }
    VERIFY.verify_profile_publication(
        release_dir / "channel-source-nightly.json",
        "code",
        base,
        release_dir,
    )
