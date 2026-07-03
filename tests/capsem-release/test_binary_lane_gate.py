"""Binary-lane release graph guards."""

from __future__ import annotations

import json
import hashlib
import subprocess
import sys
from copy import deepcopy
from pathlib import Path

import blake3

PROJECT_ROOT = Path(__file__).resolve().parents[2]
RELEASE_GRAPH = PROJECT_ROOT / "crates" / "capsem-admin" / "src" / "release_graph.rs"
ADMIN_MAIN = PROJECT_ROOT / "crates" / "capsem-admin" / "src" / "main.rs"
DIFF_POLICY = PROJECT_ROOT / "scripts" / "check-release-graph-diff.py"
DIFF_POLICY_TESTS = PROJECT_ROOT / "tests" / "capsem-release" / "test_release_lane_diff_policy.py"


def test_package_rows_are_not_binary_rows() -> None:
    source = RELEASE_GRAPH.read_text(encoding="utf-8")

    assert "pub struct PackageInventoryRow" in source
    assert "pub struct BinaryInventoryRow" in source
    assert "pub packages: Vec<PackageInventoryRow>" in source
    assert "pub binaries: Vec<BinaryInventoryRow>" in source
    assert "pub package: String" not in source
    assert "pub installed_path: String" in source
    assert "pub sbom_component_ref: String" in source
    assert "package_inventory_rows_are_separate_from_binary_rows" in source


def test_every_packaged_executable_has_hashes_and_sbom_ref() -> None:
    source = RELEASE_GRAPH.read_text(encoding="utf-8")

    assert "pub struct PackagedExecutableFile" in source
    assert "pub fn executable_inventory_from_package_files" in source
    assert "pub fn verify_package_contents_match_binary_inventory" in source
    assert "format!(\"{:x}\", Sha256::digest(&file.bytes))" in source
    assert "blake3::hash(&file.bytes)" in source
    assert "sbom_component_refs" in source
    assert "missing SBOM component reference" in source
    assert (
        "executable_inventory_records_every_packaged_binary_with_hashes_and_sbom_refs"
        in source
    )
    assert "executable_inventory_rejects_missing_sbom_component_ref" in source
    assert "executable_inventory_matches_macos_and_deb_package_contents" in source
    assert "executable_inventory_rejects_package_content_hash_drift" in source


def test_sha1_only_spdx_is_rejected() -> None:
    source = ADMIN_MAIN.read_text(encoding="utf-8")

    assert "fn validate_host_spdx_sbom_bytes" in source
    assert "let blake3 = blake3::hash(&bytes).to_hex().to_string();" in source
    assert "blake3: file.blake3.clone()" in source
    assert "channel manifest host binary {} has malformed blake3" in source
    assert 'algorithm.eq_ignore_ascii_case("SHA256")' in source
    assert "missing SHA256 checksum" in source
    assert "host_spdx_requires_sha256_file_checksums" in source


def test_binary_lane_allowed_diff_gate_is_channel_scoped() -> None:
    policy = DIFF_POLICY.read_text(encoding="utf-8")
    tests = DIFF_POLICY_TESTS.read_text(encoding="utf-8")

    assert 'choices=["binary", "profile", "channel"]' in policy
    assert 'manifest_field in {"version", "status", "packages", "binaries"}' in policy
    assert 'path[:2] != ("manifests", channel)' in policy
    assert "test_binary_allowed_diff" in tests
    assert "test_binary_lane_rejects_profile_changes" in tests
    assert "test_binary_lane_rejects_other_channel_binary_changes" in tests


def test_binary_lane_rejects_profile_ref_change(tmp_path: Path) -> None:
    old = _graph()
    new = deepcopy(old)
    new["manifests"]["stable"]["1.4.0"]["profiles"]["co-work"]["revision"] = "bad-profile-drift"

    result = _run_policy(tmp_path, old, new, "--lane", "binary", "--channel", "stable")

    assert result.returncode == 1
    assert "manifests.stable.1.4.0.profiles.co-work.revision" in result.stderr


def test_nightly_binary_update_does_not_change_stable(tmp_path: Path) -> None:
    old = _graph()
    new = deepcopy(old)
    new["channels"]["nightly"]["manifests"][0]["digest"]["sha256"] = "c" * 64
    new["manifests"]["nightly"]["1.5.0-nightly.1"]["packages"][0]["digest"][
        "sha256"
    ] = "d" * 64
    new["manifests"]["nightly"]["1.5.0-nightly.1"]["packages"][0]["binaries"][0][
        "digest"
    ]["blake3"] = "e" * 64

    result = _run_policy(tmp_path, old, new, "--lane", "binary", "--channel", "nightly")

    assert result.returncode == 0, result.stderr
    assert new["channels"]["stable"] == old["channels"]["stable"]
    assert new["manifests"]["stable"] == old["manifests"]["stable"]


def _run_policy(
    tmp_path: Path, old: dict, new: dict, *args: str
) -> subprocess.CompletedProcess[str]:
    old_path = tmp_path / "old.json"
    new_path = tmp_path / "new.json"
    old_path.write_text(json.dumps(old), encoding="utf-8")
    new_path.write_text(json.dumps(new), encoding="utf-8")
    return subprocess.run(
        [sys.executable, str(DIFF_POLICY), "--old", str(old_path), "--new", str(new_path), *args],
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def _graph() -> dict:
    return {
        "channels": {
            "stable": {
                "manifests": [
                    {
                        "version": "1.4.0",
                        "status": "current",
                        "url": "/manifests/stable/1.4.0/manifest.json",
                        "digest": _digest("a"),
                    }
                ]
            },
            "nightly": {
                "manifests": [
                    {
                        "version": "1.5.0-nightly.1",
                        "status": "current",
                        "url": "/manifests/nightly/1.5.0-nightly.1/manifest.json",
                        "digest": _digest("b"),
                    }
                ]
            },
        },
        "manifests": {
            "stable": {"1.4.0": _manifest("stable", "1.4.0")},
            "nightly": {"1.5.0-nightly.1": _manifest("nightly", "1.5.0-nightly.1")},
        },
    }


def _manifest(channel: str, version: str) -> dict:
    return {
        "version": version,
        "status": "current",
        "packages": [
            {
                "name": f"capsem-{channel}.pkg",
                "digest": _digest(f"{channel}-package"),
                "binaries": [
                    {
                        "name": "capsem",
                        "installed_path": "/usr/local/bin/capsem",
                        "digest": _digest(f"{channel}-binary"),
                    }
                ],
            }
        ],
        "profiles": {
            "co-work": {
                "id": "co-work",
                "revision": f"2026.07.02.1-{channel}",
                "images": [
                    {
                        "architecture": "arm64",
                        "artifacts": [{"kind": "rootfs", "digest": _digest("a")}],
                    }
                ],
            }
        },
    }


def _digest(seed: str) -> dict:
    payload = seed.encode("utf-8")
    return {
        "sha256": hashlib.sha256(payload).hexdigest(),
        "blake3": blake3.blake3(payload).hexdigest(),
    }
