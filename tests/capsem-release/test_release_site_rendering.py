"""Release-site rendering contract guards."""

from __future__ import annotations

import os
import subprocess
import json
import fcntl
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
RELEASE_SITE_SRC = PROJECT_ROOT / "release-site" / "src"
FIXTURE_GRAPH = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)


def test_site_loader_reads_channels_not_health() -> None:
    loader = (RELEASE_SITE_SRC / "lib" / "release-data.ts").read_text(
        encoding="utf-8"
    )
    index = (RELEASE_SITE_SRC / "pages" / "index.astro").read_text(encoding="utf-8")
    profile = (RELEASE_SITE_SRC / "pages" / "profiles" / "[id].astro").read_text(
        encoding="utf-8"
    )

    assert "channels.json" in loader
    assert "loadGraphData" in loader
    assert "selectManifestRecord" in loader
    assert "health.json" not in loader
    assert "data.health" not in index
    assert "data.health" not in profile


def test_root_lists_stable_nightly_and_manifest_statuses() -> None:
    build_release_site_from_fixture()

    index = (PROJECT_ROOT / "release-site" / "dist" / "index.html").read_text(
        encoding="utf-8"
    )

    assert "Channels" in index
    assert "Stable" in index
    assert "Nightly" in index
    assert "1.4.0" in index
    assert "1.5.0-nightly.20260702" in index
    assert "Recommended release channel" in index
    assert "Faster-moving release channel" in index


def test_root_channel_table_uses_descriptions_not_theater_labels() -> None:
    build_release_site_from_fixture()

    index = (PROJECT_ROOT / "release-site" / "dist" / "index.html").read_text(
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
    stable_package_digest = graph["manifests"]["stable"]["1.4.0"]["packages"][0][
        "digest"
    ]["blake3"]
    profile_config_digest = graph["manifests"]["stable"]["1.4.0"]["profiles"][
        "co-work"
    ]["config"][0]["digest"]["sha256"]
    pages = [
        PROJECT_ROOT / "release-site" / "dist" / "index.html",
        PROJECT_ROOT / "release-site" / "dist" / "channels" / "stable" / "index.html",
        PROJECT_ROOT
        / "release-site"
        / "dist"
        / "channels"
        / "stable"
        / "profiles"
        / "co-work"
        / "index.html",
    ]

    for full_hash in (
        stable_manifest_digest,
        stable_package_digest,
        profile_config_digest,
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
        PROJECT_ROOT / "release-site" / "dist" / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")
    nightly = (
        PROJECT_ROOT / "release-site" / "dist" / "channels" / "nightly" / "index.html"
    ).read_text(encoding="utf-8")

    assert "Current Manifest" in stable
    assert "Manifest History" in stable
    assert "Packages" in stable
    assert "Capsem Binaries" in stable
    assert "Profile References" in stable
    assert "Profile Catalog" not in stable
    assert "/profiles/releases/" not in stable
    assert "/assets/stable/manifest.json" in stable
    assert "/manifests/stable/" not in stable
    assert "Capsem-1.4.0.pkg" in stable
    assert "Capsem_1.4.0_arm64.deb" in stable
    assert "macos_pkg" in stable
    assert "debian_package" in stable
    assert "capsem-app" in stable
    assert "capsem-tray" in stable
    assert "SPDXRef-File-capsem" in stable
    stable_package_section = stable.split("Capsem Packages", maxsplit=1)[1].split(
        "Capsem Binaries",
        maxsplit=1,
    )[0]
    stable_binary_section = stable.split("Capsem Binaries", maxsplit=1)[1].split(
        "Profile References",
        maxsplit=1,
    )[0]
    stable_sbom = _fixture()["manifests"]["stable"]["1.4.0"]["packages"][0][
        "evidence"
    ][0]
    assert stable_sbom["url"] in stable_package_section
    assert _hash_label(stable_sbom["digest"]["sha256"]) in stable_package_section
    assert stable_sbom["digest"]["sha256"] not in stable_package_section
    assert stable_sbom["url"] not in stable_binary_section
    assert stable_sbom["digest"]["sha256"] not in stable_binary_section
    assert stable_sbom["digest"]["blake3"] not in stable_binary_section
    assert _hash_label(
        _fixture()["manifests"]["stable"]["1.4.0"]["packages"][0]["binaries"][0][
            "digest"
        ]["sha256"]
    ) in stable
    assert "HMAC" not in stable
    assert "hmac" not in stable
    assert "co-work" in stable
    assert "code" in stable

    assert "1.5.0-nightly.20260702" in nightly
    assert "Capsem-1.5.0-nightly.20260702.pkg" in nightly
    assert "Capsem_1.5.0-nightly.20260702_arm64.deb" in nightly
    assert "/assets/nightly/manifest.json" in nightly
    assert "/manifests/nightly/" not in nightly
    assert _hash_label(
        _fixture()["manifests"]["nightly"]["1.5.0-nightly.20260702"]["packages"][0][
            "binaries"
        ][0]["digest"]["sha256"]
    ) in nightly
    assert "HMAC" not in nightly
    assert "hmac" not in nightly
    assert "co-work" in nightly
    assert "code" in nightly


def test_channel_page_has_one_manifest_url() -> None:
    build_release_site_from_fixture()

    for channel in ("stable", "nightly"):
        page = (
            PROJECT_ROOT / "release-site" / "dist" / "channels" / channel / "index.html"
        ).read_text(encoding="utf-8")
        canonical_url = f"/assets/{channel}/manifest.json"

        assert canonical_url in page
        assert f"/manifests/{channel}/" not in page
        assert "/profiles/releases/" not in page
        assert "catalog.json" not in page
        assert "profile_catalog" not in page


def test_package_pages_show_package_owned_binaries() -> None:
    build_release_site_from_fixture()

    graph = _fixture()
    package = graph["manifests"]["stable"]["1.4.0"]["packages"][0]
    package_page_path = (
        PROJECT_ROOT
        / "release-site"
        / "dist"
        / "channels"
        / "stable"
        / "packages"
        / package["id"]
        / "index.html"
    )
    assert package_page_path.is_file()
    page = package_page_path.read_text(encoding="utf-8")

    assert "Capsem Package" in page
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


def test_channel_page_has_no_detached_profile_image_evidence() -> None:
    build_release_site_from_fixture()

    stable = (
        PROJECT_ROOT / "release-site" / "dist" / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")

    assert "Current VM Assets" not in stable
    assert "VM OBOM" not in stable
    assert "rootfs.erofs" not in stable


def test_profile_page_renders_profile_owned_images_and_configs() -> None:
    build_release_site_from_fixture()

    stable_co_work = (
        PROJECT_ROOT
        / "release-site"
        / "dist"
        / "channels"
        / "stable"
        / "profiles"
        / "co-work"
        / "index.html"
    ).read_text(encoding="utf-8")
    stable_code = (
        PROJECT_ROOT
        / "release-site"
        / "dist"
        / "channels"
        / "stable"
        / "profiles"
        / "code"
        / "index.html"
    ).read_text(encoding="utf-8")
    nightly_co_work = (
        PROJECT_ROOT
        / "release-site"
        / "dist"
        / "channels"
        / "nightly"
        / "profiles"
        / "co-work"
        / "index.html"
    ).read_text(encoding="utf-8")
    nightly_code = (
        PROJECT_ROOT
        / "release-site"
        / "dist"
        / "channels"
        / "nightly"
        / "profiles"
        / "code"
        / "index.html"
    ).read_text(encoding="utf-8")

    graph = _fixture()
    pages = {
        ("stable", "co-work"): stable_co_work,
        ("stable", "code"): stable_code,
        ("nightly", "co-work"): nightly_co_work,
        ("nightly", "code"): nightly_code,
    }
    versions = {
        "stable": "1.4.0",
        "nightly": "1.5.0-nightly.20260702",
    }
    for (channel, profile_id), page in pages.items():
        profile = graph["manifests"][channel][versions[channel]]["profiles"][profile_id]
        assert "Software Inventory" in page
        assert "Config Files" in page
        assert "Profile Images" in page
        assert "Profile Evidence" in page
        assert "HMAC" not in page
        assert "hmac" not in page
        assert profile["revision"] in page
        for item in profile["config"]:
            assert item["path"] in page
            assert item["url"] in page
            assert _hash_label(item["digest"]["sha256"]) in page
            assert _hash_label(item["digest"]["blake3"]) in page
        for image in profile["images"]:
            assert image["architecture"] in page
            for artifact in image["artifacts"]:
                assert artifact["name"] in page
                assert artifact["url"] in page
                assert _hash_label(artifact["digest"]["sha256"]) in page
                assert _hash_label(artifact["digest"]["blake3"]) in page
            for evidence in image["evidence"]:
                assert evidence["kind"].upper() in page
                assert evidence["url"] in page
                assert _hash_label(evidence["digest"]["sha256"]) in page
                assert _hash_label(evidence["digest"]["blake3"]) in page
        for software in profile["software"]:
            assert software["name"] in page
            assert software["version"] in page
            assert _hash_label(software["digest"]["sha256"]) in page
            assert _hash_label(software["digest"]["blake3"]) in page


def test_profile_page_forbids_current_binary_and_current_assets() -> None:
    build_release_site_from_fixture()

    stable = (
        PROJECT_ROOT
        / "release-site"
        / "dist"
        / "channels"
        / "stable"
        / "profiles"
        / "co-work"
        / "index.html"
    ).read_text(encoding="utf-8")

    assert "Current binary" not in stable
    assert "current_binary" not in stable
    assert "Current assets" not in stable
    assert "current_assets" not in stable
    assert "VM asset revision" not in stable
    assert "Capsem Binaries" not in stable
    assert "Capsem-1.4.0.pkg" not in stable


def build_release_site_from_fixture() -> None:
    lock_path = Path(os.environ.get("TMPDIR", "/tmp")) / "capsem-release-site-build.lock"
    with lock_path.open("w", encoding="utf-8") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        env = {
            **os.environ,
            "ASTRO_TELEMETRY_DISABLED": "1",
            "CAPSEM_RELEASE_CHANNEL_DIST": str(FIXTURE_GRAPH),
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


def _fixture() -> dict:
    return json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))


def _hash_label(value: str) -> str:
    return f"{value[:8]}..." if len(value) > 12 else value
