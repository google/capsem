"""Release-site generated-page ownership contract gates."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import fcntl
from pathlib import Path

from test_release_site_html_contract import (
    PROJECT_ROOT,
    RELEASE_SITE_DIST,
    build_release_site_from_fixture,
    fixture_graph,
)


def build_release_site_from_graph(graph_path: Path) -> None:
    if RELEASE_SITE_DIST.exists():
        shutil.rmtree(RELEASE_SITE_DIST)

    lock_path = Path(os.environ.get("TMPDIR", "/tmp")) / "capsem-release-site-build.lock"
    with lock_path.open("w", encoding="utf-8") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        result = subprocess.run(
            ["pnpm", "--dir", "release-site", "run", "build"],
            cwd=PROJECT_ROOT,
            env={
                **os.environ,
                "ASTRO_TELEMETRY_DISABLED": "1",
                "CAPSEM_RELEASE_CHANNEL_DIST": str(graph_path),
            },
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
    assert result.returncode == 0, result.stdout + result.stderr


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


def test_no_profile_catalog_side_channel() -> None:
    build_release_site_from_fixture()
    graph = fixture_graph()
    forbidden = ("profile_catalog", "catalog.json", "capsem.profile_catalog")

    serialized_graph = json.dumps(graph, sort_keys=True)
    for token in forbidden:
        assert token not in serialized_graph

    generated_pages = [
        RELEASE_SITE_DIST / "index.html",
        RELEASE_SITE_DIST / "channels" / "stable" / "index.html",
        RELEASE_SITE_DIST / "channels" / "nightly" / "index.html",
        RELEASE_SITE_DIST / "channels" / "stable" / "profiles" / "co-work" / "index.html",
        RELEASE_SITE_DIST / "channels" / "nightly" / "profiles" / "co-work" / "index.html",
    ]
    source_files = [
        PROJECT_ROOT / "release-site" / "src" / "lib" / "release-data.ts",
        PROJECT_ROOT / "release-site" / "src" / "pages" / "index.astro",
        PROJECT_ROOT / "release-site" / "src" / "pages" / "channels" / "[id].astro",
        PROJECT_ROOT / "release-site" / "src" / "pages" / "profiles" / "[id].astro",
    ]

    for path in generated_pages + source_files:
        text = path.read_text(encoding="utf-8")
        for token in forbidden:
            assert token not in text, f"{path}: {token}"


def test_astro_renders_json_graph(tmp_path: Path) -> None:
    graph = fixture_graph()
    stable_manifest = graph["manifests"]["stable"]["1.0.2"]
    package = stable_manifest["packages"][0]
    profile = stable_manifest["profiles"]["co-work"]
    binary = package["binaries"][0]
    architecture = profile["architectures"][0]

    graph["channels"]["stable"]["label"] = "Stable Graph Mutation"
    graph["channels"]["stable"]["description"] = "Description rendered from mutated channels JSON."
    package["name"] = "Capsem-json-mutated.pkg"
    binary["description"] = "Binary description rendered from mutated package JSON."
    profile["name"] = "Co-work Graph Mutation"
    profile["description"] = "Profile description rendered from mutated manifest JSON."
    architecture["software"][0]["version"] = "99.99.99-json-mutation"
    architecture["images"][0]["name"] = "rootfs-json-mutated.erofs"

    graph_path = tmp_path / "release-graph-mutated.json"
    graph_path.write_text(json.dumps(graph, indent=2, sort_keys=True), encoding="utf-8")
    build_release_site_from_graph(graph_path)

    index = (RELEASE_SITE_DIST / "index.html").read_text(encoding="utf-8")
    stable = (
        RELEASE_SITE_DIST / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")
    package_page = (
        RELEASE_SITE_DIST
        / "channels"
        / "stable"
        / "packages"
        / package["id"]
        / "index.html"
    ).read_text(encoding="utf-8")
    profile_page = (
        RELEASE_SITE_DIST
        / "channels"
        / "stable"
        / "profiles"
        / "co-work"
        / "index.html"
    ).read_text(encoding="utf-8")

    assert "Stable Graph Mutation" in index
    assert "Description rendered from mutated channels JSON." in index
    assert "Capsem-json-mutated.pkg" in stable
    assert "Capsem-json-mutated.pkg" in package_page
    assert "Binary description rendered from mutated package JSON." in package_page
    assert "Co-work Graph Mutation" in stable
    assert "Co-work Graph Mutation" in profile_page
    assert "Profile description rendered from mutated manifest JSON." in profile_page
    assert "99.99.99-json-mutation" in profile_page
    assert "rootfs-json-mutated.erofs" in profile_page
