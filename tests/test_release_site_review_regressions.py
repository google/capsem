"""Review regression gates for release-site contract feedback."""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

from test_release_site_html_contract import (
    FIXTURE_GRAPH,
    PROJECT_ROOT,
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


def test_channel_descriptions() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    index = (RELEASE_SITE_DIST / "index.html").read_text(encoding="utf-8")

    for channel in graph["channels"].values():
        assert channel["description"] in index
    assert "<code>stable</code>" not in index
    assert "<code>nightly</code>" not in index

    stripped_graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    for channel in stripped_graph["channels"].values():
        channel.pop("description", None)
    graph_path = PROJECT_ROOT / "target" / "release-site-no-channel-descriptions.json"
    graph_path.parent.mkdir(parents=True, exist_ok=True)
    graph_path.write_text(json.dumps(stripped_graph), encoding="utf-8")

    build_release_site_from_graph(graph_path)
    stripped_index = (RELEASE_SITE_DIST / "index.html").read_text(encoding="utf-8")

    assert "Recommended release channel for everyday Capsem installs." not in stripped_index
    assert "Faster-moving release channel for daily fixes and early validation." not in stripped_index


def test_root_channel_table_semantics() -> None:
    build_release_site_from_fixture()

    index = (RELEASE_SITE_DIST / "index.html").read_text(encoding="utf-8")

    assert "Selected manifest" not in index
    assert ">Status<" not in index
    assert ">Records<" not in index
    assert "manifest records" not in index
    assert ">History<" not in index
    assert "Manifest revision" in index
    assert "Updated" in index
    assert "Coverage" in index
    assert "Manifest URL" in index
    assert "<code>1.0.2</code>" in index
    assert "2026-07-03T05:45:26Z" in index
    assert "3 packages" in index
    assert "2 profiles" in index
    assert "arm64, x86_64" in index
    assert "/assets/stable/manifest.json" in index
    assert "/assets/nightly/manifest.json" in index


def test_manifest_version_independence() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        page = (RELEASE_SITE_DIST / "channels" / channel / "index.html").read_text(
            encoding="utf-8"
        )
        current_block = page.split("Current Manifest", maxsplit=1)[1].split(
            "Manifest History",
            maxsplit=1,
        )[0]

        assert "Manifest version" in current_block
        assert ">Version<" not in current_block
        assert f"<code>{current['version']}</code>" in current_block

        for package in manifest["packages"]:
            if package["version"] != current["version"]:
                assert f"<code>{package['version']}</code>" not in current_block
        for profile in manifest["profiles"].values():
            assert profile["revision"] not in current_block

        package_block = page.split("Capsem Packages", maxsplit=1)[1].split(
            "Profile References",
            maxsplit=1,
        )[0]
        assert "Manifest version" not in package_block
        for package in manifest["packages"]:
            assert f"<code>{package['version']}</code>" in package_block

        profile_block = page.split("Profile References", maxsplit=1)[1]
        assert "Manifest version" not in profile_block
        for profile in manifest["profiles"].values():
            assert f"<code>{profile['revision']}</code>" in profile_block


def test_canonical_manifest_url() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    index = (RELEASE_SITE_DIST / "index.html").read_text(encoding="utf-8")
    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        canonical = f"/assets/{channel}/manifest.json"
        assert current["url"] == canonical
        assert canonical in index

        page = (RELEASE_SITE_DIST / "channels" / channel / "index.html").read_text(
            encoding="utf-8"
        )
        assert canonical in page
        assert "Profile Catalog" not in page
        assert "catalog.json" not in page
        assert "profile_catalog" not in page

        for manifest_record in record["manifests"]:
            url = manifest_record["url"]
            if url == canonical:
                continue
            assert url not in index
            assert url not in page


def test_no_profile_catalog_side_channel() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    rendered_pages = [
        RELEASE_SITE_DIST / "index.html",
        *[
            RELEASE_SITE_DIST / "channels" / channel / "index.html"
            for channel in graph["channels"]
        ],
    ]
    forbidden_tokens = ("Profile Catalog", "profile_catalog", "catalog.json")
    for page_path in rendered_pages:
        page = page_path.read_text(encoding="utf-8")
        for token in forbidden_tokens:
            assert token not in page

    catalog_outputs = [
        path
        for path in RELEASE_SITE_DIST.rglob("catalog.json")
        if "node_modules" not in path.parts
    ]
    assert catalog_outputs == []

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        assert "profile_catalog" not in manifest
        assert "catalog" not in manifest


def test_software_evidence_once_per_architecture() -> None:
    from test_release_profile_architecture_contract import (
        test_software_inventory_evidence_once_per_architecture,
    )

    test_software_inventory_evidence_once_per_architecture()


def build_release_site_from_graph(graph_path: Path) -> None:
    env = {
        **os.environ,
        "ASTRO_TELEMETRY_DISABLED": "1",
        "CAPSEM_RELEASE_CHANNEL_DIST": str(graph_path),
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
