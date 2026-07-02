"""Local live-release artifact contract tests."""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path
from typing import Any


PROJECT_ROOT = Path(__file__).resolve().parents[2]
FIXTURE_GRAPH = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)


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
        assert (dist / current["url"].lstrip("/")).is_file()
        catalog_source = channels["channels"][channel]["profile_catalog"]["source"]
        assert (dist / catalog_source.lstrip("/")).is_file()
        assert (dist / "channels" / channel / "index.html").is_file()
        assert (dist / "channels" / channel / "profiles" / "co-work" / "index.html").is_file()

    index = (dist / "index.html").read_text(encoding="utf-8")
    stable = (dist / "channels" / "stable" / "index.html").read_text(encoding="utf-8")
    nightly = (dist / "channels" / "nightly" / "index.html").read_text(encoding="utf-8")
    stable_profile = (
        dist / "channels" / "stable" / "profiles" / "co-work" / "index.html"
    ).read_text(encoding="utf-8")
    nightly_profile = (
        dist / "channels" / "nightly" / "profiles" / "co-work" / "index.html"
    ).read_text(encoding="utf-8")

    assert "Stable" in index
    assert "Nightly" in index
    assert "Capsem-1.4.0.pkg" in stable
    assert "stable-capsem-bin-hmac" in stable
    assert "Capsem-1.5.0-nightly.20260702.pkg" in nightly
    assert "nightly-capsem-bin-hmac" in nightly
    assert "stable-co-work-config-hmac" in stable_profile
    assert "stable-co-work-rootfs-hmac" in stable_profile
    assert "stable-co-work-abom-hmac" in stable_profile
    assert "nightly-co-work-config-hmac" in nightly_profile
    assert "nightly-co-work-rootfs-hmac" in nightly_profile
    assert "nightly-co-work-abom-hmac" in nightly_profile


def _materialize_graph_dist(graph: dict[str, Any], dist: Path) -> None:
    dist.mkdir(parents=True, exist_ok=True)
    channels = json.loads(json.dumps({"version": graph["version"], "channels": graph["channels"]}))

    for channel, channel_record in channels["channels"].items():
        current = next(
            record for record in channel_record["manifests"] if record["status"] == "current"
        )
        manifest = graph["manifests"][channel][current["version"]]
        _write_json(dist / current["url"].lstrip("/"), manifest)

        catalog = {
            "schema": "capsem.profile_catalog.v1",
            "revision": f"profiles-{channel}-{current['version']}",
            "profiles": list(manifest["profiles"].values()),
        }
        catalog_source = (
            f"/profiles/releases/{catalog['revision']}/catalog.json"
        )
        channel_record["profile_catalog"] = {
            "source": catalog_source,
            "revision": catalog["revision"],
            "hash": current["digest"]["blake3"],
        }
        _write_json(dist / catalog_source.lstrip("/"), catalog)
        _materialize_profile_files(dist, catalog["profiles"])

    _write_json(dist / "channels.json", channels)
    (dist / "_headers").write_text(
        "\n".join(
            [
                "/",
                "  Cache-Control: no-cache, must-revalidate",
                "/channels.json",
                "  Cache-Control: no-cache, must-revalidate",
                "/manifests/*",
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
        for item in profile.get("config", []):
            _write_json(dist / item["url"].lstrip("/"), {"kind": item["kind"]})
        for image in profile.get("images", []):
            for artifact in image.get("artifacts", []):
                path = dist / artifact["url"].lstrip("/")
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"profile-image-artifact")
            for evidence in image.get("evidence", []):
                _write_json(
                    dist / evidence["url"].lstrip("/"),
                    {
                        "bomFormat": "CycloneDX",
                        "specVersion": "1.6",
                        "components": [],
                    },
                )


def _write_json(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
