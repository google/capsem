"""Release profile architecture and ownership contract gates."""

from __future__ import annotations

from test_release_site_html_contract import (
    PROJECT_ROOT,
    RELEASE_SITE_DIST,
    build_release_site_from_fixture,
)


PROFILE_PAGE = (
    PROJECT_ROOT / "release-site" / "src" / "pages" / "channels" / "[channel]" / "profiles" / "[id].astro"
)


def test_profile_no_current_binary() -> None:
    build_release_site_from_fixture()

    source = PROFILE_PAGE.read_text(encoding="utf-8")
    stable = (
        RELEASE_SITE_DIST
        / "channels"
        / "stable"
        / "profiles"
        / "co-work"
        / "index.html"
    ).read_text(encoding="utf-8")

    assert "current_binary" not in source
    assert "current_assets" not in source
    assert "compatibility?.min_binary" not in source
    assert "Current binary" not in stable
    assert "current_binary" not in stable
    assert "Current assets" not in stable
    assert "current_assets" not in stable
    assert "Minimum Capsem" in stable
