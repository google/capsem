"""Marketing-site download links must follow the split binary/asset release model."""

from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]


def test_marketing_download_cta_uses_release_channel_not_github_latest() -> None:
    data = (PROJECT_ROOT / "site" / "src" / "lib" / "data.ts").read_text(
        encoding="utf-8"
    )
    cta = (PROJECT_ROOT / "site" / "src" / "components" / "CTA.svelte").read_text(
        encoding="utf-8"
    )

    assert "https://release.capsem.org/channels/stable/" in data
    assert "releases/latest" not in data
    assert "Download Package" in cta
    assert "Download DMG" not in cta
    assert "download the DMG directly" not in cta


def test_getting_started_manual_download_uses_release_channel_package() -> None:
    guide = (
        PROJECT_ROOT / "docs" / "src" / "content" / "docs" / "getting-started.md"
    ).read_text(encoding="utf-8")

    assert "https://release.capsem.org/channels/stable/" in guide
    assert "releases/latest" not in guide
    assert ".pkg" in guide
    assert "DMG" not in guide
