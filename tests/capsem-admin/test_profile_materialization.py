"""Black-box profile materialization checks for capsem-admin."""

from __future__ import annotations

import json
import re
import subprocess
import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
ADMIN = PROJECT_ROOT / "target" / "debug" / "capsem-admin"
SOURCE_PROFILE = PROJECT_ROOT / "config" / "profiles" / "code" / "profile.toml"
SOURCE_PROFILE_DIR = SOURCE_PROFILE.parent


def _ensure_admin_binary() -> None:
    if ADMIN.exists():
        return
    subprocess.run(
        ["cargo", "build", "-p", "capsem-admin"],
        cwd=PROJECT_ROOT,
        check=True,
        capture_output=True,
        text=True,
        timeout=120,
    )


def _load_toml(path: Path) -> dict:
    return tomllib.loads(path.read_text())


def test_profile_materialize_generates_pins_without_mutating_source(tmp_path: Path) -> None:
    _ensure_admin_binary()
    output_root = tmp_path / "target-config"
    result = subprocess.run(
        [
            str(ADMIN),
            "profile",
            "materialize",
            "--profile",
            str(SOURCE_PROFILE),
            "--config-root",
            "config",
            "--manifest",
            (PROJECT_ROOT / "assets/manifest.json").resolve().as_uri(),
            "--assets-dir",
            "assets",
            "--output-root",
            str(output_root),
            "--arch",
            "arm64",
            "--clean",
            "--json",
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=30,
    )

    assert result.returncode == 0, (
        f"capsem-admin profile materialize failed:\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    report = json.loads(result.stdout)
    assert report["schema"] == "capsem.admin.profile_materialize.v1"
    assert report["ok"] is True
    assert report["profile_id"] == "code"
    assert report["profile_path"] == str(output_root / "profiles" / "code" / "profile.toml")
    assert report["manifest"] == str(output_root / "assets" / "manifest.json")
    assert {asset["logical_name"] for asset in report["materialized_assets"]} == {
        "vmlinuz",
        "initrd.img",
        "rootfs.erofs",
    }
    assert {asset["arch"] for asset in report["materialized_assets"]} == {"arm64"}
    assert len(report["materialized_obom"]) == 1
    assert report["materialized_obom"][0]["scope"] == "base_image"

    source_text = SOURCE_PROFILE.read_text()
    assert not re.search(r"(?m)^\s*(hash|size)\s=", source_text)

    generated_profile = output_root / "profiles" / "code" / "profile.toml"
    generated = _load_toml(generated_profile)
    source = _load_toml(SOURCE_PROFILE)
    assert generated["id"] == source["id"]
    assert generated["name"] == source["name"]
    assert generated["description"] == source["description"]
    assert set(generated["assets"]["arch"]) == {"arm64"}

    arm64_assets = generated["assets"]["arch"]["arm64"]
    for key in ("kernel", "initrd", "rootfs"):
        descriptor = arm64_assets[key]
        assert descriptor["url"].startswith("file://")
        assert re.fullmatch(r"blake3:[0-9a-f]{64}", descriptor["hash"])
        assert descriptor["size"] > 0

    for file_key in (
        "enforcement",
        "detection",
        "mcp",
        "apt_packages",
        "python_requirements",
        "npm_packages",
        "build",
        "tips",
        "root_manifest",
    ):
        descriptor = generated["files"][file_key]
        assert re.fullmatch(r"blake3:[0-9a-f]{64}", descriptor["hash"])
        assert descriptor["size"] > 0
        source_file = PROJECT_ROOT / "config" / source["files"][file_key]["path"]
        generated_file = output_root / descriptor["path"]
        assert generated_file.read_bytes() == source_file.read_bytes()

    assert (output_root / "assets" / "manifest.json").read_bytes() == (
        PROJECT_ROOT / "assets" / "manifest.json"
    ).read_bytes()
    assert not (output_root / "admin").exists()
    assert not (output_root / "skills").exists()


def test_profile_materialize_rejects_bare_manifest_path(tmp_path: Path) -> None:
    _ensure_admin_binary()
    output_root = tmp_path / "target-config"
    manifest_path = (PROJECT_ROOT / "assets/manifest.json").resolve()

    result = subprocess.run(
        [
            str(ADMIN),
            "profile",
            "materialize",
            "--profile",
            str(SOURCE_PROFILE),
            "--config-root",
            "config",
            "--manifest",
            str(manifest_path),
            "--assets-dir",
            "assets",
            "--output-root",
            str(output_root),
            "--arch",
            "arm64",
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=30,
    )

    assert result.returncode != 0
    err = result.stdout + result.stderr
    assert "manifest must be a URL" in err, err
    assert "file:///absolute/path" in err, err


def test_checked_in_source_profiles_keep_generation_hashes_out_of_profile_toml() -> None:
    offenders = []
    for profile_path in sorted((PROJECT_ROOT / "config" / "profiles").glob("*/profile.toml")):
        if re.search(r"(?m)^\s*(hash|size)\s=", profile_path.read_text()):
            offenders.append(str(profile_path.relative_to(PROJECT_ROOT)))

    assert offenders == []
