"""Release-site rendering contract guards."""

from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
RELEASE_SITE_SRC = PROJECT_ROOT / "release-site" / "src"


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
