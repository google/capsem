"""Local live-release artifact contract tests."""

from __future__ import annotations

import json
import os
import subprocess
import hashlib
from pathlib import Path
from typing import Any
from urllib.request import Request, urlopen

import blake3


PROJECT_ROOT = Path(__file__).resolve().parents[2]
FIXTURE_GRAPH = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)
PROFILE_IDS = ("co-work", "code")


def test_local_multichannel_dist_contract(tmp_path: Path) -> None:
    dist = tmp_path / "release-dist"
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    _materialize_graph_dist(graph, dist)

    result = subprocess.run(
        ["pnpm", "--dir", "release-site", "run", "build:channel"],
        cwd=PROJECT_ROOT,
        env={
            **os.environ,
            "ASTRO_TELEMETRY_DISABLED": "1",
            "CAPSEM_RELEASE_CHANNEL_DIST": str(dist),
        },
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr

    channels = json.loads((dist / "channels.json").read_text(encoding="utf-8"))
    assert sorted(channels["channels"]) == ["nightly", "stable"]
    for channel in ("stable", "nightly"):
        records = channels["channels"][channel]["manifests"]
        assert [record["status"] for record in records] == [
            "current",
            "supported",
            "deprecated",
            "revoked",
        ]
        current = records[0]
        assert current["url"] == f"/assets/{channel}/manifest.json"
        assert (dist / current["url"].lstrip("/")).is_file()
        assert "profile_catalog" not in channels["channels"][channel]
        assert (dist / "channels" / channel / "index.html").is_file()
        manifest = json.loads((dist / current["url"].lstrip("/")).read_text(encoding="utf-8"))
        assert sorted(manifest["profiles"]) == sorted(PROFILE_IDS)
        for profile_id in PROFILE_IDS:
            assert (
                dist / "channels" / channel / "profiles" / profile_id / "index.html"
            ).is_file()

    index = (dist / "index.html").read_text(encoding="utf-8")
    stable = (dist / "channels" / "stable" / "index.html").read_text(encoding="utf-8")
    nightly = (dist / "channels" / "nightly" / "index.html").read_text(encoding="utf-8")
    stable_co_work = (
        dist / "channels" / "stable" / "profiles" / "co-work" / "index.html"
    ).read_text(encoding="utf-8")
    nightly_co_work = (
        dist / "channels" / "nightly" / "profiles" / "co-work" / "index.html"
    ).read_text(encoding="utf-8")
    stable_code = (
        dist / "channels" / "stable" / "profiles" / "code" / "index.html"
    ).read_text(encoding="utf-8")
    nightly_code = (
        dist / "channels" / "nightly" / "profiles" / "code" / "index.html"
    ).read_text(encoding="utf-8")

    assert "Stable" in index
    assert "Nightly" in index
    assert "Co-work" not in index
    assert "Code" not in index
    assert "Capsem-1.4.0.pkg" in stable
    assert _hash_label(
        graph["manifests"]["stable"]["1.4.0"]["packages"][0]["binaries"][0]["digest"][
            "sha256"
        ]
    ) in stable
    assert "HMAC" not in stable
    assert "hmac" not in stable
    assert "code" in stable
    assert "Capsem-1.5.0-nightly.20260702.pkg" in nightly
    assert _hash_label(
        graph["manifests"]["nightly"]["1.5.0-nightly.20260702"]["packages"][0][
            "binaries"
        ][0]["digest"]["sha256"]
    ) in nightly
    assert "HMAC" not in nightly
    assert "hmac" not in nightly
    assert "code" in nightly
    pages = {
        ("stable", "co-work"): stable_co_work,
        ("stable", "code"): stable_code,
        ("nightly", "co-work"): nightly_co_work,
        ("nightly", "code"): nightly_code,
    }
    versions = {"stable": "1.4.0", "nightly": "1.5.0-nightly.20260702"}
    for (channel, profile_id), page in pages.items():
        profile = graph["manifests"][channel][versions[channel]]["profiles"][profile_id]
        assert "HMAC" not in page
        assert "hmac" not in page
        architecture = _profile_architectures(profile)[0]
        assert _hash_label(architecture["config"][0]["digest"]["sha256"]) in page
        assert _hash_label(architecture["images"][0]["digest"]["sha256"]) in page
        assert _hash_label(architecture["evidence"][0]["digest"]["sha256"]) in page


def test_live_channels_json_and_manifests_verify() -> None:
    base = "https://release.capsem.org"
    channels_body = _fetch_bytes(f"{base}/channels.json")
    channels = json.loads(channels_body)

    assert sorted(channels["channels"]) == ["nightly", "stable"]
    expected_package_names: dict[str, list[str]] = {}
    allowed_statuses = {"current", "supported", "deprecated", "revoked"}
    for channel in ("stable", "nightly"):
        records = channels["channels"][channel]["manifests"]
        assert records
        statuses = [record["status"] for record in records]
        assert statuses.count("current") == 1
        assert set(statuses) <= allowed_statuses
        assert "removed" not in statuses
        current = next(record for record in records if record["status"] == "current")
        manifest_body = _fetch_bytes(f"{base}{current['url']}")
        assert hashlib.sha256(manifest_body).hexdigest() == current["digest"]["sha256"]
        manifest = json.loads(manifest_body)
        assert manifest["version"] == current["version"]
        assert current["url"] == f"/assets/{channel}/manifest.json"
        assert "profile_catalog" not in channels["channels"][channel]
        assert manifest["packages"]
        assert "binaries" not in manifest
        assert manifest["packages"][0]["binaries"]
        assert sorted(manifest["profiles"]) == sorted(PROFILE_IDS)
        expected_package_names[channel] = [package["name"] for package in manifest["packages"]]

        for profile_id in PROFILE_IDS:
            profile_url = f"{base}/channels/{channel}/profiles/{profile_id}/"
            profile_page = _fetch_bytes(profile_url).decode("utf-8")
            profile = manifest["profiles"][profile_id]
            assert profile["revision"] in profile_page
            architecture = _profile_architectures(profile)[0]
            assert _hash_label(architecture["config"][0]["digest"]["sha256"]) in profile_page
            assert _hash_label(architecture["images"][0]["digest"]["sha256"]) in profile_page

    root = _fetch_bytes(f"{base}/").decode("utf-8")
    stable = _fetch_bytes(f"{base}/channels/stable/").decode("utf-8")
    nightly = _fetch_bytes(f"{base}/channels/nightly/").decode("utf-8")
    assert "Stable" in root
    assert "Nightly" in root
    assert "Co-work" not in root
    assert "Code" not in root
    for package_name in expected_package_names["stable"]:
        assert package_name in stable
    for package_name in expected_package_names["nightly"]:
        assert package_name in nightly


def _materialize_graph_dist(graph: dict[str, Any], dist: Path) -> None:
    dist.mkdir(parents=True, exist_ok=True)
    channels = json.loads(json.dumps({"version": graph["version"], "channels": graph["channels"]}))

    for channel, channel_record in channels["channels"].items():
        current = next(
            record for record in channel_record["manifests"] if record["status"] == "current"
        )
        manifest = graph["manifests"][channel][current["version"]]
        _normalize_profile_file_digests(manifest)
        current["digest"]["sha256"] = _json_sha256(manifest)
        current["digest"]["blake3"] = _json_blake3(manifest)
        _write_json(dist / current["url"].lstrip("/"), manifest)

        channel_record.pop("profile_catalog", None)
        _materialize_profile_files(dist, list(manifest["profiles"].values()))

    _write_json(dist / "channels.json", channels)
    (dist / "_headers").write_text(
        "\n".join(
            [
                "/",
                "  Cache-Control: no-cache, must-revalidate",
                "/channels.json",
                "  Cache-Control: no-cache, must-revalidate",
                "/assets/*/manifest.json",
                "  Cache-Control: no-cache, must-revalidate",
                "/profiles/releases/*",
                "  Cache-Control: public, max-age=31536000, immutable",
                "",
            ]
        ),
        encoding="utf-8",
    )


def _materialize_profile_files(dist: Path, profiles: list[dict[str, Any]]) -> None:
    for profile in profiles:
        for architecture in _profile_architectures(profile):
            for item in architecture.get("config", []):
                _write_bytes(dist / item["url"].lstrip("/"), _profile_config_bytes(item))
            for artifact in architecture.get("images", []):
                _write_bytes(dist / artifact["url"].lstrip("/"), _profile_artifact_bytes(artifact))
            for evidence in architecture.get("evidence", []):
                _write_bytes(dist / evidence["url"].lstrip("/"), _profile_evidence_bytes(evidence))


def _normalize_profile_file_digests(manifest: dict[str, Any]) -> None:
    for profile in manifest["profiles"].values():
        for architecture in _profile_architectures(profile):
            for item in architecture.get("config", []):
                _set_file_digest(item, _profile_config_bytes(item))
            for artifact in architecture.get("images", []):
                _set_file_digest(artifact, _profile_artifact_bytes(artifact))
            for evidence in architecture.get("evidence", []):
                _set_file_digest(evidence, _profile_evidence_bytes(evidence))


def _set_file_digest(item: dict[str, Any], payload: bytes) -> None:
    digest = item.setdefault("digest", {})
    digest["sha256"] = hashlib.sha256(payload).hexdigest()
    digest["blake3"] = blake3.blake3(payload).hexdigest()
    item["bytes"] = len(payload)


def _profile_config_bytes(item: dict[str, Any]) -> bytes:
    return _json_bytes({"kind": item["kind"]})


def _profile_artifact_bytes(_item: dict[str, Any]) -> bytes:
    return b"profile-image-artifact"


def _profile_evidence_bytes(_item: dict[str, Any]) -> bytes:
    return _json_bytes(
        {
            "bomFormat": "CycloneDX",
            "specVersion": "1.6",
            "components": [],
        }
    )


def _write_json(path: Path, payload: Any) -> None:
    _write_bytes(path, _json_bytes(payload))


def _write_bytes(path: Path, payload: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(payload)


def _json_bytes(payload: Any) -> bytes:
    return (json.dumps(payload, indent=2, sort_keys=True) + "\n").encode("utf-8")


def _json_sha256(payload: Any) -> str:
    return hashlib.sha256(_json_bytes(payload)).hexdigest()


def _json_blake3(payload: Any) -> str:
    return blake3.blake3(_json_bytes(payload)).hexdigest()


def _hash_label(value: str) -> str:
    return f"{value[:8]}..." if len(value) > 12 else value


def _profile_architectures(profile: dict[str, Any]) -> list[dict[str, Any]]:
    if "architectures" in profile:
        return profile["architectures"]
    return [
        {
            "architecture": image["architecture"],
            "config": profile.get("config", []),
            "images": image.get("artifacts", []),
            "evidence": image.get("evidence", []),
        }
        for image in profile.get("images", [])
    ]


def _fetch_bytes(url: str) -> bytes:
    request = Request(url, headers={"User-Agent": "CapsemReleaseValidator/1.0"})
    with urlopen(request, timeout=20) as response:
        assert response.status == 200, url
        return response.read()
