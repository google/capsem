"""Marketing-site download links must follow the split binary/asset release model."""

from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.release.tools.marketing_install_surface import (
    validate_rendered_marketing_install_surface,
)

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def test_marketing_download_cta_uses_release_channel_not_github_latest() -> None:
    data = (PROJECT_ROOT / "web" / "marketing" / "src" / "lib" / "data.ts").read_text(
        encoding="utf-8"
    )
    cta = (
        PROJECT_ROOT / "web" / "marketing" / "src" / "components" / "CTA.svelte"
    ).read_text(
        encoding="utf-8"
    )

    assert "https://release.capsem.org/channels/stable/" in data
    assert "releases/latest" not in data
    assert "Download Package" in cta
    assert "Download DMG" not in cta
    assert "download the DMG directly" not in cta


def test_marketing_homepage_exposes_the_supported_install_surface() -> None:
    index = (
        PROJECT_ROOT / "web" / "marketing" / "src" / "pages" / "index.astro"
    ).read_text(encoding="utf-8")

    assert 'import Hero from "../components/Hero.svelte"' in index
    assert 'import CTA from "../components/CTA.svelte"' in index
    assert "<Hero />" in index
    assert "<CTA" in index
    assert "Available Summer 2026" not in index


def test_publish_site_smoke_uses_the_rendered_install_surface_contract() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "site.yaml").read_text(
        encoding="utf-8"
    )

    assert "python3 build_system/scripts/web/marketing_install_surface.py /tmp/site-index.html" in workflow
    assert "Available Summer 2026" not in workflow


def test_rendered_install_surface_rejects_the_retired_holding_page() -> None:
    rendered = """
    <main id="main">
      curl -fsSL https://capsem.org/install.sh | sh
      <a href="https://release.capsem.org/channels/stable/">Download Package</a>
      Available Summer 2026
    </main>
    """

    with pytest.raises(SystemExit, match="does not expose"):
        validate_rendered_marketing_install_surface(rendered)


def test_getting_started_manual_download_uses_release_channel_package() -> None:
    guide = (
        PROJECT_ROOT / "web" / "docs" / "src" / "content" / "docs" / "getting-started.md"
    ).read_text(encoding="utf-8")

    assert "https://release.capsem.org/channels/stable/" in guide
    assert "releases/latest" not in guide
    assert ".pkg" in guide
    assert "DMG" not in guide
