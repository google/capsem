"""Release-site rendering contract guards."""

from __future__ import annotations

import os
import subprocess
import json
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
RELEASE_SITE_SRC = PROJECT_ROOT / "release-site" / "src"
FIXTURE_GRAPH = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)


def test_site_loader_reads_channels_not_health() -> None:
    loader = (RELEASE_SITE_SRC / "lib" / "release-data.ts").read_text(
        encoding="utf-8"
    )
    index = (RELEASE_SITE_SRC / "pages" / "index.astro").read_text(encoding="utf-8")
    profile = (RELEASE_SITE_SRC / "pages" / "profiles" / "[id].astro").read_text(
        encoding="utf-8"
    )

    assert "channels.json" in loader
    assert "loadGraphData" in loader
    assert "selectManifestRecord" in loader
    assert "health.json" not in loader
    assert "data.health" not in index
    assert "data.health" not in profile


def test_root_lists_stable_nightly_and_manifest_statuses() -> None:
    env = {
        **os.environ,
        "ASTRO_TELEMETRY_DISABLED": "1",
        "CAPSEM_RELEASE_CHANNEL_DIST": str(FIXTURE_GRAPH),
    }
    result = subprocess.run(
        ["pnpm", "--dir", "release-site", "run", "build"],
        cwd=PROJECT_ROOT,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr

    index = (PROJECT_ROOT / "release-site" / "dist" / "index.html").read_text(
        encoding="utf-8"
    )

    assert "Channels" in index
    assert "Stable" in index
    assert "Nightly" in index
    assert "1.4.0" in index
    assert "1.5.0-nightly.20260702" in index
    for status in ("current", "supported", "deprecated", "revoked"):
        assert status in index


def test_channel_page_lists_packages_and_binaries() -> None:
    build_release_site_from_fixture()

    stable = (
        PROJECT_ROOT / "release-site" / "dist" / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")
    nightly = (
        PROJECT_ROOT / "release-site" / "dist" / "channels" / "nightly" / "index.html"
    ).read_text(encoding="utf-8")

    assert "Selected Manifest" in stable
    assert "Manifest History" in stable
    assert "Packages" in stable
    assert "Capsem Binaries" in stable
    assert "Profile References" in stable
    assert "Capsem-1.4.0.pkg" in stable
    assert "macos_pkg" in stable
    assert "SPDXRef-File-capsem" in stable
    assert _fixture()["manifests"]["stable"]["1.4.0"]["packages"][0]["binaries"][0][
        "digest"
    ]["sha256"] in stable
    assert "HMAC" not in stable
    assert "hmac" not in stable
    assert "co-work" in stable
    assert "code" in stable

    assert "1.5.0-nightly.20260702" in nightly
    assert "Capsem-1.5.0-nightly.20260702.pkg" in nightly
    assert _fixture()["manifests"]["nightly"]["1.5.0-nightly.20260702"]["packages"][0][
        "binaries"
    ][0]["digest"]["sha256"] in nightly
    assert "HMAC" not in nightly
    assert "hmac" not in nightly
    assert "co-work" in nightly
    assert "code" in nightly


def test_channel_page_has_no_detached_profile_image_evidence() -> None:
    build_release_site_from_fixture()

    stable = (
        PROJECT_ROOT / "release-site" / "dist" / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")

    assert "Current VM Assets" not in stable
    assert "VM OBOM" not in stable
    assert "rootfs.erofs" not in stable


def test_profile_page_renders_profile_owned_images_and_configs() -> None:
    build_release_site_from_fixture()

    stable_co_work = (
        PROJECT_ROOT
        / "release-site"
        / "dist"
        / "channels"
        / "stable"
        / "profiles"
        / "co-work"
        / "index.html"
    ).read_text(encoding="utf-8")
    stable_code = (
        PROJECT_ROOT
        / "release-site"
        / "dist"
        / "channels"
        / "stable"
        / "profiles"
        / "code"
        / "index.html"
    ).read_text(encoding="utf-8")
    nightly_co_work = (
        PROJECT_ROOT
        / "release-site"
        / "dist"
        / "channels"
        / "nightly"
        / "profiles"
        / "co-work"
        / "index.html"
    ).read_text(encoding="utf-8")
    nightly_code = (
        PROJECT_ROOT
        / "release-site"
        / "dist"
        / "channels"
        / "nightly"
        / "profiles"
        / "code"
        / "index.html"
    ).read_text(encoding="utf-8")

    graph = _fixture()
    pages = {
        ("stable", "co-work"): stable_co_work,
        ("stable", "code"): stable_code,
        ("nightly", "co-work"): nightly_co_work,
        ("nightly", "code"): nightly_code,
    }
    versions = {
        "stable": "1.4.0",
        "nightly": "1.5.0-nightly.20260702",
    }
    for (channel, profile_id), page in pages.items():
        profile = graph["manifests"][channel][versions[channel]]["profiles"][profile_id]
        assert "Software Inventory" in page
        assert "Config Files" in page
        assert "Profile Images" in page
        assert "Profile Evidence" in page
        assert "HMAC" not in page
        assert "hmac" not in page
        assert profile["revision"] in page
        for item in profile["config"]:
            assert item["path"] in page
            assert item["url"] in page
            assert item["digest"]["sha256"] in page
            assert item["digest"]["blake3"] in page
        for image in profile["images"]:
            assert image["architecture"] in page
            for artifact in image["artifacts"]:
                assert artifact["name"] in page
                assert artifact["url"] in page
                assert artifact["digest"]["sha256"] in page
                assert artifact["digest"]["blake3"] in page
            for evidence in image["evidence"]:
                assert evidence["kind"].upper() in page
                assert evidence["url"] in page
                assert evidence["digest"]["sha256"] in page
                assert evidence["digest"]["blake3"] in page
        for software in profile["software"]:
            assert software["name"] in page
            assert software["version"] in page
            assert software["digest"]["sha256"] in page
            assert software["digest"]["blake3"] in page


def test_profile_page_forbids_current_binary_and_current_assets() -> None:
    build_release_site_from_fixture()

    stable = (
        PROJECT_ROOT
        / "release-site"
        / "dist"
        / "channels"
        / "stable"
        / "profiles"
        / "co-work"
        / "index.html"
    ).read_text(encoding="utf-8")

    assert "Current binary" not in stable
    assert "current_binary" not in stable
    assert "Current assets" not in stable
    assert "current_assets" not in stable
    assert "VM asset revision" not in stable
    assert "Capsem Binaries" not in stable
    assert "Capsem-1.4.0.pkg" not in stable


def build_release_site_from_fixture() -> None:
    env = {
        **os.environ,
        "ASTRO_TELEMETRY_DISABLED": "1",
        "CAPSEM_RELEASE_CHANNEL_DIST": str(FIXTURE_GRAPH),
    }
    result = subprocess.run(
        ["pnpm", "--dir", "release-site", "run", "build"],
        cwd=PROJECT_ROOT,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def _fixture() -> dict:
    return json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
