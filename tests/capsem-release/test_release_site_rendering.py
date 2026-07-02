"""Release-site rendering contract guards."""

from __future__ import annotations

import os
import subprocess
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
    assert "6666666666666666666666666666666666666666666666666666666666666666" in stable
    assert "stable-capsem-bin-hmac" in stable

    assert "1.5.0-nightly.20260702" in nightly
    assert "Capsem-1.5.0-nightly.20260702.pkg" in nightly
    assert "nightly-capsem-bin-hmac" in nightly


def test_channel_page_has_no_detached_profile_image_evidence() -> None:
    build_release_site_from_fixture()

    stable = (
        PROJECT_ROOT / "release-site" / "dist" / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")

    assert "Current VM Assets" not in stable
    assert "VM OBOM" not in stable
    assert "rootfs.erofs" not in stable
    assert "stable-co-work-rootfs-hmac" not in stable
    assert "stable-co-work-abom-hmac" not in stable


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
