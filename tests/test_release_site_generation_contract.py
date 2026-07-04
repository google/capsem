"""Compatibility release-site generation contract gates."""

from test_release_site_generated_from_json import (
    RELEASE_SITE_DIST,
    build_release_site_from_fixture,
    fixture_graph,
    test_no_profile_catalog_side_channel,
)


def test_release_site_forbids_profile_catalog_side_channel() -> None:
    test_no_profile_catalog_side_channel()


def test_human_pages_expose_one_canonical_manifest_fetch_url() -> None:
    build_release_site_from_fixture()
    graph = fixture_graph()
    pages = [path.read_text(encoding="utf-8") for path in RELEASE_SITE_DIST.rglob("*.html")]

    for channel, record in graph["channels"].items():
        canonical = f"/assets/{channel}/manifest.json"
        assert sum(page.count(f'href="{canonical}"') for page in pages) >= 1, channel

        alternate_urls = [
            manifest["url"]
            for manifest in record["manifests"]
            if manifest["url"] != canonical
        ]
        for alternate_url in alternate_urls:
            for page in pages:
                assert f'href="{alternate_url}"' not in page, alternate_url
                assert f">{alternate_url}<" not in page, alternate_url
