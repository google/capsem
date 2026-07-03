"""Release-site generated-page ownership contract gates."""

from __future__ import annotations

import hashlib
import fcntl
import importlib.util
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

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


def test_stale_html_rejected(monkeypatch: Any) -> None:
    checker = load_remote_readiness_checker()
    site = "https://release.test"
    channel = "stable"
    channels, manifest, manifest_payload, artifact_bytes = minimal_release_graph(checker)
    pages = minimal_release_pages(checker, site, channel, channels, manifest)

    patch_release_fetches(
        monkeypatch,
        checker,
        site=site,
        channels=channels,
        manifest_payload=manifest_payload,
        artifact_bytes=artifact_bytes,
        pages=pages,
    )
    good = checker.check_release_site_contract(site, channel)
    assert good.ok, good.detail

    package_name = manifest["packages"][0]["name"]
    stale_pages = dict(pages)
    stale_pages[f"{site}/channels/{channel}/"] = stale_pages[
        f"{site}/channels/{channel}/"
    ].replace(package_name, "Capsem-stale.pkg")
    patch_release_fetches(
        monkeypatch,
        checker,
        site=site,
        channels=channels,
        manifest_payload=manifest_payload,
        artifact_bytes=artifact_bytes,
        pages=stale_pages,
    )

    stale = checker.check_release_site_contract(site, channel)
    assert not stale.ok
    assert f"channel page {channel} missing package name {package_name}" in stale.detail


def load_remote_readiness_checker() -> Any:
    module_path = PROJECT_ROOT / "scripts" / "check-remote-release-readiness.py"
    spec = importlib.util.spec_from_file_location(
        "check_remote_release_readiness_for_generated_site_tests",
        module_path,
    )
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def minimal_release_graph(
    checker: Any,
) -> tuple[dict[str, Any], dict[str, Any], bytes, dict[str, bytes]]:
    artifact_bytes = {
        "/profiles/releases/2026.0703.1/co-work/arm64/mcp.json": b'{"mcpServers":{}}\n',
        "/assets/releases/2026.0703.1/arm64-rootfs.erofs": b"rootfs image bytes\n",
    }
    manifest: dict[str, Any] = {
        "version": "1.0.2+assets.2026.0703.1",
        "packages": [
            {
                "id": "capsem-1-4-0-pkg",
                "kind": "macos_pkg",
                "platform": "macos",
                "architecture": "arm64",
                "name": "Capsem-1.4.0.pkg",
                "version": "1.4.0",
                "url": (
                    "https://github.com/google/capsem/releases/download/"
                    "v1.4.0/Capsem-1.4.0.pkg"
                ),
                "bytes": 12,
                "digest": digest(checker, b"package bytes"),
                "binaries": [
                    {
                        "name": "capsem-app",
                        "version": "1.4.0",
                        "description": "Capsem desktop application executable",
                        "installed_path": "/Applications/Capsem.app/Contents/MacOS/capsem-app",
                        "architecture": "arm64",
                        "platform": "macos",
                        "bytes": 12,
                        "digest": digest(checker, b"binary bytes"),
                        "sbom_component_ref": "SPDXRef-File-capsem-app",
                    }
                ],
            }
        ],
        "profiles": {
            "co-work": {
                "id": "co-work",
                "name": "Co-work",
                "description": "Collaborative agent profile.",
                "revision": "2026.07.03.1",
                "min_capsem_version": "1.4.0",
                "architectures": [
                    {
                        "architecture": "arm64",
                        "software": [
                            {
                                "name": "@openai/codex",
                                "version": "0.142.5",
                                "source": "npm",
                                "architecture": "arm64",
                                "evidence": (
                                    "/profiles/releases/2026.0703.1/co-work/arm64/"
                                    "npm-packages.txt"
                                ),
                                "digest": digest(checker, b"codex software row"),
                            }
                        ],
                        "config": [
                            {
                                "kind": "mcp",
                                "path": "profiles/co-work/mcp.json",
                                "url": "/profiles/releases/2026.0703.1/co-work/arm64/mcp.json",
                                "bytes": len(
                                    artifact_bytes[
                                        "/profiles/releases/2026.0703.1/co-work/arm64/mcp.json"
                                    ]
                                ),
                                "digest": digest(
                                    checker,
                                    artifact_bytes[
                                        "/profiles/releases/2026.0703.1/co-work/arm64/mcp.json"
                                    ],
                                ),
                            }
                        ],
                        "images": [
                            {
                                "kind": "rootfs",
                                "name": "rootfs.erofs",
                                "url": "/assets/releases/2026.0703.1/arm64-rootfs.erofs",
                                "bytes": len(
                                    artifact_bytes[
                                        "/assets/releases/2026.0703.1/arm64-rootfs.erofs"
                                    ]
                                ),
                                "digest": digest(
                                    checker,
                                    artifact_bytes[
                                        "/assets/releases/2026.0703.1/arm64-rootfs.erofs"
                                    ],
                                ),
                            }
                        ],
                        "evidence": [],
                    }
                ],
            }
        },
    }
    manifest_payload = json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()
    channels = {
        "version": 1,
        "generated_at": "2026-07-03T05:45:26Z",
        "channels": {
            "stable": {
                "label": "Stable",
                "description": "Recommended release channel.",
                "manifests": [
                    {
                        "version": manifest["version"],
                        "revision": manifest["version"],
                        "status": "current",
                        "url": "/assets/stable/manifest.json",
                        "digest": digest(checker, manifest_payload),
                    }
                ],
            }
        },
    }
    return channels, manifest, manifest_payload, artifact_bytes


def minimal_release_pages(
    checker: Any,
    site: str,
    channel: str,
    channels: dict[str, Any],
    manifest: dict[str, Any],
) -> dict[str, str]:
    channel_record = channels["channels"][channel]
    manifest_record = channel_record["manifests"][0]
    package = manifest["packages"][0]
    binary = package["binaries"][0]
    profile = manifest["profiles"]["co-work"]
    architecture = profile["architectures"][0]
    config = architecture["config"][0]
    image = architecture["images"][0]
    return {
        f"{site}/": " ".join(
            [
                channel_record["label"],
                channel_record["description"],
                manifest_record["version"],
                manifest_record["url"],
            ]
        ),
        f"{site}/channels/{channel}/": " ".join(
            [
                channel_record["label"],
                manifest_record["version"],
                manifest_record["url"],
                package["name"],
                profile["name"],
                profile["id"],
                profile["revision"],
                profile["min_capsem_version"],
            ]
        ),
        f"{site}/channels/{channel}/packages/{package['id']}/": " ".join(
            [
                package["name"],
                package["version"],
                package["kind"],
                checker.hash_label(package["digest"]["sha256"]),
                checker.hash_label(package["digest"]["blake3"]),
                binary["name"],
                binary["version"],
                binary["description"],
                binary["installed_path"],
                checker.hash_label(binary["digest"]["sha256"]),
                checker.hash_label(binary["digest"]["blake3"]),
                binary["sbom_component_ref"],
            ]
        ),
        f"{site}/channels/{channel}/profiles/co-work/": " ".join(
            [
                profile["name"],
                profile["description"],
                profile["id"],
                profile["revision"],
                profile["min_capsem_version"],
                architecture["architecture"],
                checker.hash_label(config["digest"]["sha256"]),
                checker.hash_label(config["digest"]["blake3"]),
                checker.hash_label(image["digest"]["sha256"]),
                checker.hash_label(image["digest"]["blake3"]),
            ]
        ),
    }


def patch_release_fetches(
    monkeypatch: Any,
    checker: Any,
    *,
    site: str,
    channels: dict[str, Any],
    manifest_payload: bytes,
    artifact_bytes: dict[str, bytes],
    pages: dict[str, str],
) -> None:
    checker._FETCH_BYTES_CACHE.clear()

    def fake_fetch_text(url: str) -> Any:
        if url == f"{site}/channels.json":
            return checker.FetchText(json.dumps(channels))
        if url in pages:
            return checker.FetchText(pages[url])
        return checker.FetchText("", f"unexpected text URL {url}")

    def fake_fetch_bytes(url: str) -> Any:
        path = url.removeprefix(site)
        if path == "/assets/stable/manifest.json":
            return checker.FetchBytes(manifest_payload)
        if path in artifact_bytes:
            return checker.FetchBytes(artifact_bytes[path])
        return checker.FetchBytes(b"", f"unexpected bytes URL {url}")

    def fake_fetch_headers(url: str) -> Any:
        path = url.removeprefix(site)
        if path in {"/", "/channels.json", "/assets/stable/manifest.json"}:
            return checker.FetchHeaders({"cache-control": "no-cache, must-revalidate"})
        if path.startswith(("/assets/releases/", "/profiles/releases/")):
            return checker.FetchHeaders(
                {"cache-control": "public, max-age=31536000, immutable"}
            )
        return checker.FetchHeaders({}, f"unexpected headers URL {url}")

    monkeypatch.setattr(checker, "fetch_text", fake_fetch_text)
    monkeypatch.setattr(checker, "fetch_bytes", fake_fetch_bytes)
    monkeypatch.setattr(checker, "fetch_headers", fake_fetch_headers)


def digest(checker: Any, payload: bytes) -> dict[str, str]:
    assert checker.blake3 is not None
    return {
        "sha256": hashlib.sha256(payload).hexdigest(),
        "blake3": checker.blake3.blake3(payload).hexdigest(),
    }
