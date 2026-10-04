"""Release-site generation gates proving HTML values come from owner JSON."""

from __future__ import annotations

import copy
import hashlib
import importlib
import json
from pathlib import Path
from typing import Any

from blake3 import blake3
from capsem_builder.release.tools import check_remote_release_readiness as READINESS
from helpers.release_site import build_release_site
from test_release_site_html_contract import (
    FIXTURE_GRAPH,
    PROJECT_ROOT,
    RELEASE_SITE_DIST,
    build_release_site_from_fixture,
    fixture_graph,
)


def test_no_invented_data() -> None:
    build_release_site_from_fixture()
    graph = fixture_graph()

    index = (RELEASE_SITE_DIST / "index.html").read_text(encoding="utf-8")
    stable = (RELEASE_SITE_DIST / "channels" / "stable" / "index.html").read_text(
        encoding="utf-8"
    )
    runtime_page = (RELEASE_SITE_DIST / "channels" / "stable" / "runtime" / "index.html").read_text(
        encoding="utf-8"
    )

    stable_manifest = graph["manifests"]["stable"]["1.0.2"]
    stable_package = stable_manifest["packages"][0]
    runtime_urls = [
        item["url"]
        for architecture in stable_manifest["runtime"]["architectures"]
        for group in ("images", "evidence")
        for item in architecture[group]
    ]

    assert stable_package["name"] not in index
    assert stable_package["url"] not in index
    assert "Capsem Packages" not in index
    assert "Runtime Evidence" not in stable
    assert "Software Inventory" not in stable
    for url in runtime_urls:
        assert url not in stable
        assert url in runtime_page

    assert "Capsem Packages" not in runtime_page
    assert "Manifest History" not in runtime_page
    assert stable_package["name"] not in runtime_page
    assert stable_package["evidence"][0]["url"] not in runtime_page


def test_no_catalog_side_channel() -> None:
    build_release_site_from_fixture()
    graph = fixture_graph()
    forbidden = ("catalog.json",)

    serialized_graph = json.dumps(graph, sort_keys=True)
    for token in forbidden:
        assert token not in serialized_graph

    generated_pages = [
        RELEASE_SITE_DIST / "index.html",
        RELEASE_SITE_DIST / "channels" / "stable" / "index.html",
        RELEASE_SITE_DIST / "channels" / "nightly" / "index.html",
        RELEASE_SITE_DIST / "channels" / "stable" / "runtime" / "index.html",
        RELEASE_SITE_DIST / "channels" / "nightly" / "runtime" / "index.html",
    ]
    pages = PROJECT_ROOT / "build_system" / "release_site" / "src" / "pages"
    source_files = [
        PROJECT_ROOT / "build_system" / "release_site" / "src" / "lib" / "release-data.ts",
        pages / "index.astro",
        pages / "channels" / "[id].astro",
        pages / "channels" / "[channel]" / "runtime.astro",
    ]

    for path in generated_pages + source_files:
        text = path.read_text(encoding="utf-8")
        for token in forbidden:
            assert token not in text, f"{path}: {token}"


def test_root_channel_metadata() -> None:
    build_release_site_from_fixture()
    graph = fixture_graph()

    index = (RELEASE_SITE_DIST / "index.html").read_text(encoding="utf-8")
    stable = graph["channels"]["stable"]
    nightly = graph["channels"]["nightly"]

    assert stable["description"] in index
    assert nightly["description"] in index
    assert stable["manifests"][0]["version"] in index
    assert nightly["manifests"][0]["version"] in index
    assert stable["manifests"][0]["url"] in index
    assert nightly["manifests"][0]["url"] in index
    assert "Selected manifest" not in index
    assert ">Status<" not in index
    assert ">Records<" not in index
    assert "<code>stable</code>" not in index
    assert "<code>nightly</code>" not in index


def test_astro_renders_json_graph(tmp_path: Path) -> None:
    graph = fixture_graph()
    stable_manifest = graph["manifests"]["stable"]["1.0.2"]
    package = stable_manifest["packages"][0]
    runtime = stable_manifest["runtime"]
    binary = package["binaries"][0]
    architecture = runtime["architectures"][0]

    graph["channels"]["stable"]["label"] = "Stable Graph Mutation"
    graph["channels"]["stable"]["description"] = "Description rendered from mutated channels JSON."
    package["name"] = "Capsem-json-mutated.pkg"
    binary["description"] = "Binary description rendered from mutated package JSON."
    runtime["revision"] = "runtime-graph-mutation"
    architecture["software"][0]["version"] = "99.99.99-json-mutation"
    architecture["images"][0]["name"] = "rootfs-json-mutated.erofs"

    graph_path = tmp_path / "release-graph-mutated.json"
    graph_path.write_text(json.dumps(graph, indent=2, sort_keys=True), encoding="utf-8")
    dist = build_release_site(graph_path)

    index = (dist / "index.html").read_text(encoding="utf-8")
    stable = (dist / "channels" / "stable" / "index.html").read_text(encoding="utf-8")
    package_page = (
        dist / "channels" / "stable" / "packages" / package["id"] / "index.html"
    ).read_text(encoding="utf-8")
    runtime_page = (dist / "channels" / "stable" / "runtime" / "index.html").read_text(
        encoding="utf-8"
    )

    assert "Stable Graph Mutation" in index
    assert "Description rendered from mutated channels JSON." in index
    assert "Capsem-json-mutated.pkg" in stable
    assert "Capsem-json-mutated.pkg" in package_page
    assert "Binary description rendered from mutated package JSON." in package_page
    assert "runtime-graph-mutation" in stable
    assert "runtime-graph-mutation" in runtime_page
    assert "99.99.99-json-mutation" in runtime_page
    assert "rootfs-json-mutated.erofs" in runtime_page


def test_rendered_values_map_to_owning_json_paths(tmp_path: Path) -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    mutated = copy.deepcopy(graph)
    mutated["generated_at"] = "2031-02-03T04:05:06Z"

    channel = mutated["channels"]["stable"]
    current_record = next(item for item in channel["manifests"] if item["status"] == "current")
    current_record["revision"] = "manifest-json-owned-1"
    current_record["updated_at"] = "2030-01-02T03:04:05Z"
    current_record["digest"] = _digest("stable-current-manifest-record")

    manifest = mutated["manifests"]["stable"][current_record["version"]]
    package = manifest["packages"][0]
    package["name"] = "Capsem JSON-owned Package"
    package["version"] = "9.8.7-json-package"
    package["url"] = "https://release.example.invalid/json-owned-package.pkg"
    package["bytes"] = 9876543
    package["digest"] = _digest("json-owned-package")
    package["evidence"][0]["url"] = (
        "https://release.example.invalid/json-owned-package-sbom.spdx.json"
    )
    package["evidence"][0]["bytes"] = 7654321
    package["evidence"][0]["digest"] = _digest("json-owned-package-sbom")

    binary = package["binaries"][0]
    binary["name"] = "capsem-json-owned-binary"
    binary["version"] = "9.8.7-json-binary"
    binary["description"] = "JSON-owned binary description"
    binary["installed_path"] = "/usr/local/bin/capsem-json-owned-binary"
    binary["bytes"] = 1234567
    binary["digest"] = _digest("json-owned-binary")
    binary["sbom_component_ref"] = "SPDXRef-File-json-owned-binary"

    runtime = manifest["runtime"]
    runtime["revision"] = "2030.01.02-json"
    runtime["min_capsem_version"] = "9.8.7"

    architecture = next(
        item for item in runtime["architectures"] if item["architecture"] == "arm64"
    )
    software = architecture["software"][0]
    software["name"] = "@json/owned-tool"
    software["version"] = "7.6.5"
    software["source"] = "npm-json-owned"
    software["digest"] = _digest("json-owned-software")

    image = architecture["images"][0]
    image["kind"] = "json-owned-rootfs"
    image["name"] = "json-owned-rootfs.erofs"
    image["url"] = "https://release.example.invalid/json-owned-rootfs.erofs"
    image["bytes"] = 6789
    image["digest"] = _digest("json-owned-image")

    evidence = next(
        item for item in architecture["evidence"] if item["kind"] == "software_inventory"
    )
    evidence["url"] = "https://release.example.invalid/json-owned-software-inventory.json"
    evidence["bytes"] = 2345
    evidence["digest"] = _digest("json-owned-runtime-evidence")

    image_evidence = next(item for item in architecture["evidence"] if item["kind"] == "abom")
    image_evidence["url"] = "https://release.example.invalid/json-owned-abom.cdx.json"
    image_evidence["bytes"] = 3456
    image_evidence["digest"] = _digest("json-owned-image-evidence")

    graph_path = tmp_path / "release-graph-json-owned.json"
    graph_path.write_text(json.dumps(mutated), encoding="utf-8")
    dist = build_release_site(graph_path)

    index = (dist / "index.html").read_text(encoding="utf-8")
    stable_page = (dist / "channels" / "stable" / "index.html").read_text(
        encoding="utf-8"
    )
    package_page = (
        dist / "channels" / "stable" / "packages" / package["id"] / "index.html"
    ).read_text(encoding="utf-8")
    runtime_page = (dist / "channels" / "stable" / "runtime" / "index.html").read_text(
        encoding="utf-8"
    )

    _assert_values(
        index,
        "root channel table",
        [
            "Stable",
            "manifest-json-owned-1",
            "2030-01-02T03:04:05Z",
            "/assets/stable/manifest.json",
        ],
    )

    _assert_values(
        _section(stable_page, "Current Manifest", "Manifest History"),
        "current manifest section",
        [
            current_record["version"],
            "2031-02-03T04:05:06Z",
            _hash_label(current_record["digest"]["sha256"]),
            _hash_label(current_record["digest"]["blake3"]),
        ],
    )
    _assert_values(
        _section(stable_page, "Manifest History", "Capsem Packages"),
        "manifest history section",
        [
            _hash_label(current_record["digest"]["sha256"]),
            _hash_label(current_record["digest"]["blake3"]),
        ],
    )
    _assert_values(
        _section(stable_page, "Capsem Packages", ">Runtime</h2>"),
        "channel packages section",
        [
            package["name"],
            package["version"],
            package["url"],
            "9,876,543",
            _hash_label(package["digest"]["sha256"]),
            _hash_label(package["evidence"][0]["digest"]["blake3"]),
        ],
    )
    _assert_values(
        _section(stable_page, ">Runtime</h2>", "</section>"),
        "channel runtime section",
        [runtime["revision"], runtime["min_capsem_version"], "arm64"],
    )

    _assert_values(
        _section(package_page, "Package", "Contained Binaries"),
        "package detail section",
        [
            package["id"],
            package["name"],
            package["version"],
            package["url"],
            _hash_label(package["digest"]["sha256"]),
        ],
    )
    _assert_values(
        _section(package_page, "Contained Binaries", "Package Evidence"),
        "contained binaries section",
        [
            binary["name"],
            binary["version"],
            binary["description"],
            binary["installed_path"],
            "1,234,567",
            _hash_label(binary["digest"]["sha256"]),
            binary["sbom_component_ref"],
        ],
    )
    _assert_values(
        _section(package_page, "Package Evidence", "</section>"),
        "package evidence section",
        [
            "json-owned-package-sbom.spdx.json",
            "7,654,321",
            _hash_label(package["evidence"][0]["digest"]["sha256"]),
            _hash_label(package["evidence"][0]["digest"]["blake3"]),
        ],
    )

    _assert_values(
        _section(runtime_page, ">Runtime</h2>", "Architecture arm64"),
        "runtime summary section",
        [runtime["revision"], runtime["min_capsem_version"]],
    )
    _assert_values(
        _section(runtime_page, "Runtime Images", "Runtime Evidence"),
        "runtime images section",
        [
            image["kind"],
            image["name"],
            image["url"],
            "6,789",
            _hash_label(image["digest"]["sha256"]),
        ],
    )
    _assert_values(
        _section(runtime_page, "Runtime Evidence", "Installed Software"),
        "runtime evidence section",
        [
            "json-owned-software-inventory.json",
            "2,345",
            _hash_label(evidence["digest"]["sha256"]),
            "json-owned-abom.cdx.json",
            "3,456",
            _hash_label(image_evidence["digest"]["sha256"]),
            _hash_label(image_evidence["digest"]["blake3"]),
        ],
    )
    _assert_values(
        _section(runtime_page, "Installed Software", "</section>"),
        "installed software section",
        [
            software["name"],
            software["version"],
            software["source"],
            _hash_label(software["digest"]["blake3"]),
        ],
    )


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


def test_release_site_validator_checks_content_not_file_existence(
    monkeypatch: Any,
) -> None:
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

    package = manifest["packages"][0]
    binary = package["binaries"][0]
    package_pages = dict(pages)
    package_pages[f"{site}/channels/{channel}/packages/{package['id']}/"] = (
        package_pages[f"{site}/channels/{channel}/packages/{package['id']}/"]
        .replace(binary["installed_path"], "/Applications/Capsem.app/stale")
        .replace(
            checker.hash_label(binary["digest"]["sha256"]),
            "stale-bin-sha...",
        )
    )
    patch_release_fetches(
        monkeypatch,
        checker,
        site=site,
        channels=channels,
        manifest_payload=manifest_payload,
        artifact_bytes=artifact_bytes,
        pages=package_pages,
    )
    stale_package = checker.check_release_site_contract(site, channel)
    assert not stale_package.ok
    assert (
        f"package page {channel}/{package['id']} missing binary installed path "
        f"{binary['installed_path']}"
    ) in stale_package.detail
    assert (
        f"package page {channel}/{package['id']} missing binary SHA-256 "
        f"{checker.hash_label(binary['digest']['sha256'])}"
    ) in stale_package.detail

    runtime_pages = dict(pages)
    runtime_pages[f"{site}/channels/{channel}/"] = runtime_pages[
        f"{site}/channels/{channel}/"
    ].replace(manifest["runtime"]["revision"], "stale-runtime-revision")
    patch_release_fetches(
        monkeypatch,
        checker,
        site=site,
        channels=channels,
        manifest_payload=manifest_payload,
        artifact_bytes=artifact_bytes,
        pages=runtime_pages,
    )
    stale_runtime = checker.check_release_site_contract(site, channel)
    assert not stale_runtime.ok
    assert (
        f"channel page {channel} missing runtime revision {manifest['runtime']['revision']}"
    ) in stale_runtime.detail


def load_remote_readiness_checker() -> Any:
    return importlib.reload(READINESS)


def minimal_release_graph(
    checker: Any,
) -> tuple[dict[str, Any], dict[str, Any], bytes, dict[str, bytes]]:
    runtime_base = "/runtime/releases/stable/0.7.0-0123456789ab/arm64"
    images = (("kernel", "vmlinuz"), ("initrd", "initrd.img"), ("rootfs", "rootfs.erofs"))
    artifact_bytes = {
        f"{runtime_base}/{name}": f"{kind} image bytes\n".encode() for kind, name in images
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
        "runtime": {
            "revision": "0.7.0-0123456789ab",
            "status": "current",
            "min_capsem_version": "1.4.0",
            "architectures": [
                {
                    "architecture": "arm64",
                    "package_inventory_revision": "0.7.0-0123456789ab",
                    "image_revision": "0.7.0-0123456789ab",
                    "software": [
                        {
                            "name": "@openai/codex",
                            "version": "0.142.5",
                            "source": "npm",
                            "architecture": "arm64",
                            "evidence": f"{runtime_base}/software-inventory.json",
                            "digest": digest(checker, b"codex software row"),
                        }
                    ],
                    "images": [
                        {
                            "kind": kind,
                            "name": name,
                            "url": f"{runtime_base}/{name}",
                            "status": "current",
                            "bytes": len(artifact_bytes[f"{runtime_base}/{name}"]),
                            "digest": digest(checker, artifact_bytes[f"{runtime_base}/{name}"]),
                        }
                        for kind, name in images
                    ],
                    "evidence": [],
                }
            ],
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
    runtime = manifest["runtime"]
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
                package["version"],
                runtime["revision"],
                runtime["min_capsem_version"],
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
        if path.startswith(("/assets/releases/", "/runtime/releases/")):
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


def _digest(seed: str) -> dict[str, str]:
    payload = seed.encode("utf-8")
    return {
        "sha256": hashlib.sha256(payload).hexdigest(),
        "blake3": blake3(payload).hexdigest(),
    }


def _hash_label(value: str) -> str:
    return f"{value[:8]}..."


def _assert_values(page: str, section: str, values: list[str]) -> None:
    for value in values:
        assert value in page, f"{section} did not render JSON-owned value {value!r}"


def _section(page: str, start: str, end: str) -> str:
    assert start in page, f"missing section start {start!r}"
    body = page.split(start, maxsplit=1)[1]
    if end != "</section>":
        assert end in body, f"missing section end {end!r}"
        return body.split(end, maxsplit=1)[0]
    return body.split(end, maxsplit=1)[0]
