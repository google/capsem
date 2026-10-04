"""Release-site rendering contract guards."""

from __future__ import annotations

import json

from helpers.release_site import (
    FIXTURE_GRAPH,
    PROJECT_ROOT,
    RELEASE_SITE_DIST,
    build_release_site_from_fixture,
)

RELEASE_SITE_SRC = PROJECT_ROOT / "build_system" / "release_site" / "src"


def test_site_loader_reads_channels_not_health() -> None:
    loader = (RELEASE_SITE_SRC / "lib" / "release-data.ts").read_text(
        encoding="utf-8"
    )
    index = (RELEASE_SITE_SRC / "pages" / "index.astro").read_text(encoding="utf-8")
    runtime = (RELEASE_SITE_SRC / "pages" / "channels" / "[channel]" / "runtime.astro").read_text(
        encoding="utf-8"
    )

    assert "channels.json" in loader
    assert "loadGraphData" in loader
    assert "selectManifestRecord" in loader
    assert "health.json" not in loader
    assert "data.health" not in index
    assert "data.health" not in runtime


def test_root_lists_stable_nightly_and_manifest_statuses() -> None:
    build_release_site_from_fixture()

    index = (RELEASE_SITE_DIST / "index.html").read_text(
        encoding="utf-8"
    )

    assert "Channels" in index
    assert "Stable" in index
    assert "Nightly" in index
    assert "Manifest revision" in index
    assert "1.0.2" in index
    assert "1.5.0-nightly.20260702" not in index
    assert "Recommended release channel" in index
    assert "Faster-moving release channel" in index


def test_root_channel_table_uses_descriptions_not_theater_labels() -> None:
    build_release_site_from_fixture()

    index = (RELEASE_SITE_DIST / "index.html").read_text(
        encoding="utf-8"
    )

    assert "Selected manifest" not in index
    assert ">Status<" not in index
    assert ">Records<" not in index
    assert "<code>stable</code>" not in index
    assert "<code>nightly</code>" not in index
    assert "recommended" in index.lower()
    assert "faster-moving" in index.lower()


def test_human_pages_truncate_hashes_but_machine_graph_keeps_full_hashes() -> None:
    build_release_site_from_fixture()

    graph = _fixture()
    stable_manifest_digest = graph["channels"]["stable"]["manifests"][0]["digest"][
        "sha256"
    ]
    stable_package_digest = graph["manifests"]["stable"]["1.0.2"]["packages"][0][
        "digest"
    ]["blake3"]
    runtime_image_digest = graph["manifests"]["stable"]["1.0.2"]["runtime"]["architectures"][0][
        "images"
    ][0]["digest"]["sha256"]
    pages = [
        RELEASE_SITE_DIST / "index.html",
        RELEASE_SITE_DIST / "channels" / "stable" / "index.html",
        RELEASE_SITE_DIST / "channels" / "stable" / "runtime" / "index.html",
    ]

    for full_hash in (
        stable_manifest_digest,
        stable_package_digest,
        runtime_image_digest,
    ):
        assert len(full_hash) == 64
        assert full_hash in FIXTURE_GRAPH.read_text(encoding="utf-8")
        short_hash = f"{full_hash[:8]}..."
        rendered = "\n".join(path.read_text(encoding="utf-8") for path in pages)
        assert short_hash in rendered
        assert full_hash not in rendered


def test_channel_page_lists_packages_and_binaries() -> None:
    build_release_site_from_fixture()

    stable = (
        RELEASE_SITE_DIST / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")
    nightly = (
        RELEASE_SITE_DIST / "channels" / "nightly" / "index.html"
    ).read_text(encoding="utf-8")

    assert "Current Manifest" in stable
    assert "Manifest History" in stable
    assert "Packages" in stable
    assert "Capsem Binaries" not in stable
    assert ">Runtime</h2>" in stable
    assert "/runtime/releases/" not in stable
    assert "/assets/stable/manifest.json" in stable
    assert "/manifests/stable/" not in stable
    assert "Capsem-1.4.0.pkg" in stable
    assert "Capsem_1.4.0_arm64.deb" in stable
    assert "macos_pkg" in stable
    assert "debian_package" in stable
    stable_package_section = stable.split("Capsem Packages", maxsplit=1)[1].split(
        ">Runtime</h2>",
        maxsplit=1,
    )[0]
    stable_sbom = _fixture()["manifests"]["stable"]["1.0.2"]["packages"][0][
        "evidence"
    ][0]
    assert stable_sbom["url"] in stable_package_section
    assert _hash_label(stable_sbom["digest"]["sha256"]) in stable_package_section
    assert stable_sbom["digest"]["sha256"] not in stable_package_section
    for package in _fixture()["manifests"]["stable"]["1.0.2"]["packages"]:
        for binary in package["binaries"]:
            assert binary["installed_path"] not in stable
            assert binary["sbom_component_ref"] not in stable
    assert "HMAC" not in stable
    assert "hmac" not in stable
    assert "/channels/stable/runtime/" in stable
    assert "1.0.0-stable.20260702" in stable

    assert "1.5.0-nightly.20260702" in nightly
    assert "Capsem-1.5.0-nightly.20260702.pkg" in nightly
    assert "Capsem_1.5.0-nightly.20260702_arm64.deb" in nightly
    assert "/assets/nightly/manifest.json" in nightly
    assert "/manifests/nightly/" not in nightly
    for package in _fixture()["manifests"]["nightly"]["1.0.2"][
        "packages"
    ]:
        for binary in package["binaries"]:
            assert binary["installed_path"] not in nightly
            assert binary["sbom_component_ref"] not in nightly
    assert "HMAC" not in nightly
    assert "hmac" not in nightly
    assert "/channels/nightly/runtime/" in nightly
    assert "1.0.0-nightly.20260702" in nightly


def test_channel_page_has_one_manifest_url() -> None:
    build_release_site_from_fixture()

    for channel in ("stable", "nightly"):
        page = (
            RELEASE_SITE_DIST / "channels" / channel / "index.html"
        ).read_text(encoding="utf-8")
        canonical_url = f"/assets/{channel}/manifest.json"

        assert canonical_url in page
        assert f"/manifests/{channel}/" not in page
        assert "/runtime/releases/" not in page
        assert "catalog.json" not in page
        assert "profile_catalog" not in page


def test_package_pages_show_package_owned_binaries() -> None:
    build_release_site_from_fixture()

    graph = _fixture()
    package = graph["manifests"]["stable"]["1.0.2"]["packages"][0]
    package_page_path = (
        RELEASE_SITE_DIST
        / "channels"
        / "stable"
        / "packages"
        / package["id"]
        / "index.html"
    )
    assert package_page_path.is_file()
    page = package_page_path.read_text(encoding="utf-8")

    assert package["name"] in page
    assert "Capsem Package" not in page
    assert package["name"] in page
    assert package["kind"] in page
    assert _hash_label(package["digest"]["sha256"]) in page
    assert _hash_label(package["digest"]["blake3"]) in page
    assert "Contained Binaries" in page
    assert "HMAC" not in page
    assert "hmac" not in page
    for binary in package["binaries"]:
        assert binary["name"] in page
        assert binary["version"] in page
        assert binary["installed_path"] in page
        assert _hash_label(binary["digest"]["sha256"]) in page
        assert _hash_label(binary["digest"]["blake3"]) in page
        assert binary["sbom_component_ref"] in page
    sibling = graph["manifests"]["stable"]["1.0.2"]["packages"][1]
    assert sibling["name"] not in page
    assert sibling["url"] not in page
    for binary in sibling["binaries"]:
        assert binary["installed_path"] not in page


def test_package_detail_page() -> None:
    test_package_pages_show_package_owned_binaries()


def test_package_sbom_link_not_repeated_on_binaries() -> None:
    build_release_site_from_fixture()

    graph = _fixture()
    package = graph["manifests"]["stable"]["1.0.2"]["packages"][0]
    page = (
        RELEASE_SITE_DIST
        / "channels"
        / "stable"
        / "packages"
        / package["id"]
        / "index.html"
    ).read_text(encoding="utf-8")
    package_evidence_section = page.split("Package Evidence", maxsplit=1)[1]
    binary_section = page.split("Contained Binaries", maxsplit=1)[1].split(
        "Package Evidence",
        maxsplit=1,
    )[0]

    for evidence in package["evidence"]:
        assert evidence["url"] in package_evidence_section
        assert _hash_label(evidence["digest"]["sha256"]) in package_evidence_section
        assert evidence["url"] not in binary_section
        assert evidence["digest"]["sha256"] not in binary_section
        assert evidence["digest"]["blake3"] not in binary_section
    for binary in package["binaries"]:
        assert binary["sbom_component_ref"] in binary_section


def test_channel_page_has_no_detached_runtime_image_evidence() -> None:
    build_release_site_from_fixture()

    stable = (RELEASE_SITE_DIST / "channels" / "stable" / "index.html").read_text(
        encoding="utf-8"
    )

    assert "Current VM Assets" not in stable
    assert "VM OBOM" not in stable
    assert "rootfs.erofs" not in stable


def test_runtime_page_renders_runtime_images_evidence_and_software() -> None:
    build_release_site_from_fixture()

    graph = _fixture()
    for channel in ("stable", "nightly"):
        page = (RELEASE_SITE_DIST / "channels" / channel / "runtime" / "index.html").read_text(
            encoding="utf-8"
        )
        runtime = graph["manifests"][channel]["1.0.2"]["runtime"]
        assert "Installed Software" in page
        assert "Runtime Images" in page
        assert "Runtime Evidence" in page
        assert "Config Files" not in page
        assert "HMAC" not in page
        assert "hmac" not in page
        assert runtime["revision"] in page
        for architecture in runtime["architectures"]:
            assert f"Architecture {architecture['architecture']}" in page
            for artifact in architecture["images"]:
                assert artifact["name"] in page
                assert artifact["url"] in page
                assert _hash_label(artifact["digest"]["sha256"]) in page
                assert _hash_label(artifact["digest"]["blake3"]) in page
            for evidence in architecture["evidence"]:
                assert evidence["kind"].upper() in page
                assert evidence["url"] in page
                assert _hash_label(evidence["digest"]["sha256"]) in page
                assert _hash_label(evidence["digest"]["blake3"]) in page
            for software in architecture["software"]:
                assert software["name"] in page
                assert software["version"] in page
                assert _hash_label(software["digest"]["sha256"]) in page
                assert _hash_label(software["digest"]["blake3"]) in page


def test_runtime_evidence_not_repeated_per_row() -> None:
    build_release_site_from_fixture()
    graph = _fixture()
    page = (RELEASE_SITE_DIST / "channels" / "stable" / "runtime" / "index.html").read_text(
        encoding="utf-8"
    )
    runtime = graph["manifests"]["stable"]["1.0.2"]["runtime"]

    for architecture in runtime["architectures"]:
        section = page.split(f"Architecture {architecture['architecture']}", maxsplit=1)[
            1
        ].split("</section>", maxsplit=1)[0]
        image_block = section.split("Runtime Images", maxsplit=1)[1].split(
            "Runtime Evidence", maxsplit=1
        )[0]
        evidence_block = section.split("Runtime Evidence", maxsplit=1)[1].split(
            "Installed Software", maxsplit=1
        )[0]
        software_block = section.split("Installed Software", maxsplit=1)[1]

        for evidence in architecture["evidence"]:
            assert evidence["url"] in evidence_block
            assert evidence["url"] not in image_block
            assert evidence["url"] not in software_block


def test_runtime_page_forbids_current_binary_and_packages() -> None:
    build_release_site_from_fixture()

    page = (RELEASE_SITE_DIST / "channels" / "stable" / "runtime" / "index.html").read_text(
        encoding="utf-8"
    )

    assert "Current binary" not in page
    assert "current_binary" not in page
    assert "Current assets" not in page
    assert "current_assets" not in page
    assert "Capsem Binaries" not in page
    assert "Capsem-1.4.0.pkg" not in page


def _fixture() -> dict:
    return json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))


def _hash_label(value: str) -> str:
    return f"{value[:8]}..." if len(value) > 12 else value
