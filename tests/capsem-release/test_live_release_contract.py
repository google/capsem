"""Hermetic multi-channel release artifact contract tests."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
from pathlib import Path
from typing import Any

import blake3
from helpers.release_site import FIXTURE_GRAPH, PROJECT_ROOT, release_site_build_lock


def test_local_multichannel_dist_contract(tmp_path: Path) -> None:
    dist = tmp_path / "release-dist"
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    _materialize_graph_dist(graph, dist)

    # build:channel renders through the shared build_system/release_site/dist before
    # overlaying into `dist`, so it has to serialize with every other build.
    with release_site_build_lock():
        result = subprocess.run(
            ["pnpm", "--dir", "build_system/release_site", "run", "build:channel"],
            cwd=PROJECT_ROOT,
            env={
                **os.environ,
                "ASTRO_TELEMETRY_DISABLED": "1",
                # Both: `astro build` renders from the graph and
                # `overlay-dist.mjs` writes the channel dist. Here they are
                # one directory, which is why one name looked sufficient.
                "CAPSEM_RELEASE_GRAPH": str(dist),
                "CAPSEM_RELEASE_CHANNEL_DIST": str(dist),
            },
            text=True,
            capture_output=True,
            check=False,
        )
    assert result.returncode == 0, result.stdout + result.stderr

    channels = json.loads((dist / "channels.json").read_text(encoding="utf-8"))
    assert sorted(channels["channels"]) == ["nightly", "stable"]
    versions: dict[str, str] = {}
    for channel in ("stable", "nightly"):
        records = channels["channels"][channel]["manifests"]
        assert [record["status"] for record in records] == [
            "current",
            "supported",
            "deprecated",
            "revoked",
        ]
        current = records[0]
        versions[channel] = current["version"]
        assert current["url"] == f"/assets/{channel}/manifest.json"
        assert (dist / current["url"].lstrip("/")).is_file()
        assert (dist / "channels" / channel / "index.html").is_file()
        assert (dist / "channels" / channel / "runtime" / "index.html").is_file()
        manifest = json.loads((dist / current["url"].lstrip("/")).read_text(encoding="utf-8"))
        assert "profiles" not in manifest
        assert manifest["runtime"]["revision"]

    index = (dist / "index.html").read_text(encoding="utf-8")
    stable = (dist / "channels" / "stable" / "index.html").read_text(encoding="utf-8")
    nightly = (dist / "channels" / "nightly" / "index.html").read_text(encoding="utf-8")

    assert "Stable" in index
    assert "Nightly" in index
    assert "Capsem-1.4.0.pkg" in stable
    assert (
        _hash_label(
            graph["manifests"]["stable"]["1.0.2"]["packages"][0]["binaries"][0]["digest"]["sha256"]
        )
        in stable
    )
    assert "Capsem-1.5.0-nightly.20260702.pkg" in nightly
    assert (
        _hash_label(
            graph["manifests"]["nightly"]["1.0.2"]["packages"][0]["binaries"][0]["digest"]["sha256"]
        )
        in nightly
    )
    for channel, page in (("stable", stable), ("nightly", nightly)):
        runtime = graph["manifests"][channel][versions[channel]]["runtime"]
        assert "HMAC" not in page
        assert "hmac" not in page
        # The channel page names the runtime and links to it; the runtime
        # page owns the per-architecture facts.
        assert runtime["revision"] in page
        assert f"/channels/{channel}/runtime/" in page

        runtime_page = (dist / "channels" / channel / "runtime" / "index.html").read_text(
            encoding="utf-8"
        )
        assert "HMAC" not in runtime_page
        assert "hmac" not in runtime_page
        for architecture in runtime["architectures"]:
            assert _hash_label(architecture["images"][0]["digest"]["sha256"]) in runtime_page
            assert _hash_label(architecture["evidence"][0]["digest"]["sha256"]) in runtime_page


def _materialize_graph_dist(graph: dict[str, Any], dist: Path) -> None:
    dist.mkdir(parents=True, exist_ok=True)
    channels = json.loads(json.dumps({"version": graph["version"], "channels": graph["channels"]}))

    for channel, channel_record in channels["channels"].items():
        current = next(
            record for record in channel_record["manifests"] if record["status"] == "current"
        )
        manifest = graph["manifests"][channel][current["version"]]
        _normalize_runtime_file_digests(manifest["runtime"])
        current["digest"]["sha256"] = _json_sha256(manifest)
        current["digest"]["blake3"] = _json_blake3(manifest)
        _write_json(dist / current["url"].lstrip("/"), manifest)
        _materialize_runtime_files(dist, manifest["runtime"])

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
                "/runtime/releases/*",
                "  Cache-Control: public, max-age=31536000, immutable",
                "",
            ]
        ),
        encoding="utf-8",
    )


def _materialize_runtime_files(dist: Path, runtime: dict[str, Any]) -> None:
    for architecture in runtime["architectures"]:
        for artifact in architecture["images"]:
            _write_bytes(dist / artifact["url"].lstrip("/"), _image_bytes())
        for evidence in architecture["evidence"]:
            _write_bytes(dist / evidence["url"].lstrip("/"), _evidence_bytes())


def _normalize_runtime_file_digests(runtime: dict[str, Any]) -> None:
    for architecture in runtime["architectures"]:
        for artifact in architecture["images"]:
            _set_file_digest(artifact, _image_bytes())
        for evidence in architecture["evidence"]:
            _set_file_digest(evidence, _evidence_bytes())


def _set_file_digest(item: dict[str, Any], payload: bytes) -> None:
    digest = item.setdefault("digest", {})
    digest["sha256"] = hashlib.sha256(payload).hexdigest()
    digest["blake3"] = blake3.blake3(payload).hexdigest()
    item["bytes"] = len(payload)


def _image_bytes() -> bytes:
    return b"runtime-image-artifact"


def _evidence_bytes() -> bytes:
    return _json_bytes({"bomFormat": "CycloneDX", "specVersion": "1.6", "components": []})


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
