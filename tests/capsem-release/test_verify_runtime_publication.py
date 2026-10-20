from __future__ import annotations

import hashlib
import json
from pathlib import Path

import blake3
import pytest
from capsem_builder.release.tools import stage_runtime_publication as STAGE
from capsem_builder.release.tools import verify_runtime_publication as VERIFY

RELEASES = "https://github.com/google/capsem/releases/download"
BASE = f"{RELEASES}/runtime-nightly-9.9.0-0123456789ab"


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


def _manifest(base: str, *, channel: str = "nightly", **payloads: bytes) -> dict[str, object]:
    kernel = payloads["kernel"]
    obom = payloads["obom"]
    inventory = payloads["inventory"]
    return {
        "channel": channel,
        "packages": [],
        "runtime": {
            "revision": "9.9.0-0123456789ab",
            "status": "current",
            "architectures": [
                {
                    "architecture": "x86_64",
                    "images": [
                        {
                            **_record(f"{base}/x86_64-vmlinuz", kernel, name="vmlinuz"),
                            "kind": "kernel",
                        }
                    ],
                    "evidence": [
                        {
                            **_record(f"{base}/x86_64-obom.cdx.json", obom),
                            "kind": "obom",
                        },
                        {
                            **_record(f"{base}/x86_64-software-inventory.json", inventory),
                            "kind": "software_inventory",
                        },
                    ],
                    "software": [
                        {
                            "name": "python",
                            "version": "3.12.11",
                            "source": "apt",
                            "architecture": "x86_64",
                            "evidence": f"{base}/x86_64-software-inventory.json",
                            "digest": {"sha256": "a" * 64, "blake3": "b" * 64},
                        }
                    ],
                }
            ],
        },
    }


PAYLOADS = {
    "kernel": b"kernel",
    "obom": b'{"bomFormat":"CycloneDX"}',
    "inventory": (
        b'{"schema":"capsem.runtime_software_inventory.v1","architecture":"x86_64",'
        b'"packages":[{"name":"python","version":"3.12.11","source":"apt"}]}'
    ),
}


def _publication(tmp_path: Path, *, channel: str = "nightly") -> tuple[Path, Path]:
    release_dir = tmp_path / "publication"
    release_dir.mkdir()
    files = {
        "x86_64-vmlinuz": PAYLOADS["kernel"],
        "x86_64-obom.cdx.json": PAYLOADS["obom"],
        "x86_64-software-inventory.json": PAYLOADS["inventory"],
    }
    for name, payload in files.items():
        (release_dir / name).write_bytes(payload)
    source = release_dir / f"channel-source-{channel}.json"
    source.write_text(json.dumps(_manifest(BASE, channel=channel, **PAYLOADS)), encoding="utf-8")
    return source, release_dir


def test_runtime_publication_exactly_matches_manifest(tmp_path: Path) -> None:
    source, release_dir = _publication(tmp_path)

    verified = VERIFY.verify_runtime_publication(source, BASE, release_dir)

    assert {path.name for path in verified} == {
        "x86_64-vmlinuz",
        "x86_64-obom.cdx.json",
        "x86_64-software-inventory.json",
    }


def test_runtime_publication_identity_is_channel_qualified(tmp_path: Path) -> None:
    source, release_dir = _publication(tmp_path, channel="stable")

    with pytest.raises(ValueError, match="channel/revision identity runtime-stable-"):
        VERIFY.verify_runtime_publication(source, BASE, release_dir)


def test_runtime_publication_rejects_unresolvable_software_evidence(
    tmp_path: Path,
) -> None:
    source, release_dir = _publication(tmp_path)
    manifest = json.loads(source.read_text(encoding="utf-8"))
    software = manifest["runtime"]["architectures"][0]["software"]
    software[0]["evidence"] = f"{BASE}/asset-revision/x86_64-software-inventory.json"
    source.write_text(json.dumps(manifest), encoding="utf-8")

    with pytest.raises(ValueError, match="software evidence"):
        VERIFY.verify_runtime_publication(source, BASE, release_dir)


def test_runtime_publication_rejects_tamper_and_extra_files(tmp_path: Path) -> None:
    source, release_dir = _publication(tmp_path)
    (release_dir / "x86_64-vmlinuz").write_bytes(b"tampered")

    with pytest.raises(ValueError, match=r"metadata mismatch|SHA-256 mismatch"):
        VERIFY.verify_runtime_publication(source, BASE, release_dir)

    (release_dir / "x86_64-vmlinuz").write_bytes(PAYLOADS["kernel"])
    (release_dir / "unexpected.txt").write_text("extra", encoding="utf-8")
    with pytest.raises(ValueError, match="file set mismatch"):
        VERIFY.verify_runtime_publication(source, BASE, release_dir)


def test_runtime_publication_allows_identical_rows_to_share_one_blob(
    tmp_path: Path,
) -> None:
    source, release_dir = _publication(tmp_path)
    manifest = json.loads(source.read_text(encoding="utf-8"))
    images = manifest["runtime"]["architectures"][0]["images"]
    duplicate = dict(images[0])
    images.append(duplicate)
    source.write_text(json.dumps(manifest), encoding="utf-8")

    VERIFY.verify_runtime_publication(source, BASE, release_dir)

    duplicate["bytes"] += 1
    source.write_text(json.dumps(manifest), encoding="utf-8")
    with pytest.raises(ValueError, match="conflicting metadata"):
        VERIFY.verify_runtime_publication(source, BASE, release_dir)


def test_runtime_publication_stages_only_manifest_described_inputs(
    tmp_path: Path,
) -> None:
    assets = tmp_path / "assets" / "x86_64"
    assets.mkdir(parents=True)
    (assets / "vmlinuz").write_bytes(PAYLOADS["kernel"])
    (assets / "obom.cdx.json").write_bytes(PAYLOADS["obom"])
    (assets / "software-inventory.json").write_bytes(PAYLOADS["inventory"])
    (assets / "build-ledger.log").write_text("must not publish", encoding="utf-8")
    source = tmp_path / "source.json"
    source.write_text(json.dumps(_manifest(BASE, **PAYLOADS)), encoding="utf-8")
    release_dir = tmp_path / "publication"

    staged = STAGE.stage_runtime_publication(source, tmp_path / "assets", release_dir)

    assert {path.name for path in staged} == {
        "x86_64-vmlinuz",
        "x86_64-obom.cdx.json",
        "x86_64-software-inventory.json",
        "channel-source-nightly.json",
    }
    VERIFY.verify_runtime_publication(
        release_dir / "channel-source-nightly.json", BASE, release_dir
    )


def test_runtime_publication_requires_a_runtime(tmp_path: Path) -> None:
    source = tmp_path / "source.json"
    source.write_text(json.dumps({"channel": "nightly", "packages": []}), encoding="utf-8")

    with pytest.raises(ValueError, match="does not contain a runtime"):
        STAGE.stage_runtime_publication(source, tmp_path / "assets", tmp_path / "out")
    with pytest.raises(ValueError, match="does not contain a runtime"):
        VERIFY.verify_runtime_publication(source, BASE, tmp_path)
