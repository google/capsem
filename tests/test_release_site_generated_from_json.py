"""Release-site generated-page ownership contract gates."""

from __future__ import annotations

from test_release_site_html_contract import (
    RELEASE_SITE_DIST,
    build_release_site_from_fixture,
    fixture_graph,
)


def test_no_invented_data() -> None:
    build_release_site_from_fixture()
    graph = fixture_graph()

    index = (RELEASE_SITE_DIST / "index.html").read_text(encoding="utf-8")
    stable = (
        RELEASE_SITE_DIST / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")
    profile = (
        RELEASE_SITE_DIST
        / "channels"
        / "stable"
        / "profiles"
        / "co-work"
        / "index.html"
    ).read_text(encoding="utf-8")

    stable_manifest = graph["manifests"]["stable"]["1.0.2"]
    stable_package = stable_manifest["packages"][0]
    stable_profile = stable_manifest["profiles"]["co-work"]
    profile_image_urls = [
        item["url"]
        for architecture in stable_profile["architectures"]
        for group in ("images", "evidence")
        for item in architecture[group]
    ]

    assert stable_package["name"] not in index
    assert stable_package["url"] not in index
    assert "Capsem Packages" not in index
    assert "Profile Evidence" not in stable
    assert "Software Inventory" not in stable
    for url in profile_image_urls:
        assert url not in stable

    assert "Capsem Packages" not in profile
    assert "Manifest History" not in profile
    assert stable_package["name"] not in profile
    assert stable_package["evidence"][0]["url"] not in profile
