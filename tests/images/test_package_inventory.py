"""Inventory exact exported image bytes, never a stale base package snapshot."""
from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import tarfile
from pathlib import Path

import pytest
from capsem_builder.release import package_inventory as inventory

ROOT = Path(__file__).resolve().parents[2]

SUBJECT = "ghcr.io/google/capsem/dev@sha256:" + "1" * 64


def image_tar(path: Path, files: dict[str, bytes]) -> None:
    with tarfile.open(path, "w") as output:
        for name, body in files.items():
            member = tarfile.TarInfo(name)
            member.size = len(body)
            output.addfile(member, io.BytesIO(body))


def test_inventory_uses_final_installed_packages_and_checks_tool_binary(tmp_path: Path) -> None:
    toolkit = {"architecture": "arm64", "tools": [{"name": "starship", "version": "starship 1.27.0",
        "binary_path": "/usr/local/bin/starship", "binary_sha256": hashlib.sha256(b"actual").hexdigest()}],
        "packages": [{"name": "stale-base-only", "version": "0"}]}
    source = tmp_path / "rootfs.tar"
    image_tar(source, {"./var/lib/dpkg/status": b"Package: git\nStatus: install ok installed\nVersion: 2.39.5\nArchitecture: arm64\n\nPackage: removed\nStatus: deinstall ok config-files\nVersion: 1\n",
        "usr/local/share/capsem/toolkit-inventory.json": json.dumps(toolkit).encode(),
        "usr/local/bin/starship": b"actual"})
    result = inventory.from_rootfs(source, SUBJECT, "arm64")
    assert result["subject"] == SUBJECT
    assert result["architecture"] == "arm64"
    assert result["packages"] == [{"name": "git", "version": "2.39.5", "architecture": "arm64"}]
    assert result["tools"] == toolkit["tools"]
    toolkit["tools"][0]["binary_sha256"] = "0" * 64
    image_tar(source, {"var/lib/dpkg/status": b"Package: git\nStatus: install ok installed\nVersion: 2\nArchitecture: arm64\n",
        "usr/local/share/capsem/toolkit-inventory.json": json.dumps(toolkit).encode(),
        "usr/local/bin/starship": b"actual"})
    with pytest.raises(ValueError, match="binary changed"):
        inventory.from_rootfs(source, SUBJECT, "arm64")


def test_missing_or_incomplete_inventory_fails_instead_of_fabricating_empty(tmp_path: Path) -> None:
    source = tmp_path / "rootfs.tar"
    image_tar(source, {})
    with pytest.raises(ValueError, match="missing"):
        inventory.from_rootfs(source, SUBJECT, "arm64")
    image_tar(source, {"var/lib/dpkg/status": b"Package: git\nStatus: install ok installed\n"})
    with pytest.raises(ValueError, match="incomplete"):
        inventory.from_rootfs(source, SUBJECT, "arm64")


def test_inventory_does_not_follow_archive_links_or_accept_a_mutable_subject(tmp_path: Path) -> None:
    source = tmp_path / "rootfs.tar"
    with tarfile.open(source, "w") as output:
        member = tarfile.TarInfo("var/lib/dpkg/status")
        member.type = tarfile.SYMTYPE
        member.linkname = "/etc/passwd"
        output.addfile(member)
    with pytest.raises(ValueError, match="regular inventory file"):
        inventory.from_rootfs(source, SUBJECT, "arm64")
    with pytest.raises(ValueError, match="exact image digest"):
        inventory.from_rootfs(source, "registry.example/dev:latest", "arm64")


def test_package_inventory_artifact_is_bound_to_each_platform() -> None:
    spec = importlib.util.spec_from_file_location("image_catalog", ROOT / "images/ci/catalog.py")
    assert spec and spec.loader
    catalog = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(catalog)
    digest = "sha256:" + "2" * 64
    records = [{"kind": "index", "image": "dev", "key": "a", "built": True, "digest": digest},
        {"kind": "platform", "image": "dev", "key": "a", "built": True, "arch": "arm64",
         "digest": digest, "obom": digest, "erofs": digest, "inventory": digest}]
    version = catalog.versions(records, "ghcr.io/google/capsem")["dev"]
    assert version["evidence"] == {"arm64": [{"kind": "package_inventory",
        "reference": "ghcr.io/google/capsem/dev@" + digest,
        "media_type": "application/vnd.capsem.package-inventory.v1+json"}]}


def test_publication_attaches_inventory_to_platform_subject_and_keys_its_producer() -> None:
    import yaml
    workflow = yaml.safe_load((ROOT / ".github/workflows/images.yaml").read_text())
    steps = workflow["jobs"]["image-arch"]["steps"]
    attach = next(step for step in steps if step.get("id") == "attach")
    assert attach["env"]["SUBJECT"].endswith("@${{ steps.push.outputs.digest }}")
    assert '"$SUBJECT" packages.json:application/vnd.capsem.package-inventory.v1+json' in attach["run"]
    key = next(step for step in steps if step.get("id") == "published")
    from images.ci.inputkey import DERIVED_FILES

    assert '--derived "$IMAGE" --with "$BASE_DIGEST"' in key["run"]
    assert "build_system/builder/release/package_inventory.py" in DERIVED_FILES


def test_standalone_fixture_inventory_requires_real_packages_but_not_shared_toolkit(tmp_path: Path) -> None:
    source = tmp_path / "rootfs.tar"
    image_tar(source, {"var/lib/dpkg/status": b"Package: git\nStatus: install ok installed\nVersion: 2.39.5\nArchitecture: arm64\n"})
    with pytest.raises(ValueError, match="toolkit-inventory"):
        inventory.from_rootfs(source, SUBJECT, "arm64")
    result = inventory.from_rootfs(source, SUBJECT, "arm64", shared_toolkit=False)
    assert result["packages"] == [{"name": "git", "version": "2.39.5", "architecture": "arm64"}]
    assert result["tools"] == []
    assert result["subject"] == SUBJECT
    image_tar(source, {})
    with pytest.raises(ValueError, match="regular inventory file"):
        inventory.from_rootfs(source, SUBJECT, "arm64", shared_toolkit=False)
