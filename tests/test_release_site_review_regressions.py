"""Review regression gates for release-site contract feedback."""

from __future__ import annotations

import json

from test_release_site_html_contract import (
    FIXTURE_GRAPH,
    RELEASE_SITE_DIST,
    build_release_site_from_fixture,
)


def test_packages_grouped_by_os_architecture() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        page = (RELEASE_SITE_DIST / "channels" / channel / "index.html").read_text(
            encoding="utf-8"
        )
        package_block = page.split("Capsem Packages", maxsplit=1)[1].split(
            "Profile References",
            maxsplit=1,
        )[0]

        assert "Capsem Binaries" not in package_block
        for package in manifest["packages"]:
            platform = "macOS" if package["platform"] == "macos" else package["platform"].title()
            heading = f"Package target {platform} {package['architecture']}"
            detail_href = f"/channels/{channel}/packages/{package['id']}/"

            assert heading in package_block
            assert detail_href in package_block
            assert package["name"] in package_block
            assert package["url"] in package_block
