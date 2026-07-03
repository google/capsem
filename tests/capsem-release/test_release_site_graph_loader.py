"""Release-site graph loader gates."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
FIXTURE_GRAPH = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)


def test_release_site_builds_from_release_graph_fixture() -> None:
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
    stable = (
        PROJECT_ROOT / "release-site" / "dist" / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")
    package_detail = (
        PROJECT_ROOT
        / "release-site"
        / "dist"
        / "channels"
        / "stable"
        / "packages"
        / "capsem-1-4-0-pkg"
        / "index.html"
    ).read_text(encoding="utf-8")
    profile = (
        PROJECT_ROOT
        / "release-site"
        / "dist"
        / "channels"
        / "stable"
        / "profiles"
        / "co-work"
        / "index.html"
    ).read_text(encoding="utf-8")

    assert "/assets/stable/manifest.json" in index
    assert "/manifests/stable/" not in index
    assert "Stable" in index
    assert "Nightly" in index
    assert "Manifest revision" in index
    assert "1.0.2" in index
    assert "1.5.0-nightly.20260702" not in index
    assert "Capsem-1.4.0.pkg" not in index
    assert "rootfs.erofs" not in index
    assert "Capsem-1.4.0.pkg" in stable
    assert "SPDXRef-File-capsem" not in stable
    assert "SPDXRef-File-capsem" in package_detail
    assert "rootfs.erofs" not in stable
    assert "2026.07.02.1-stable" in profile
    assert "Minimum Capsem" in profile
    assert "ABOM" in profile
