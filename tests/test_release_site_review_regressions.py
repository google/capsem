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
