"""Top-level release-site HTML gates used by Sprinty release hardening."""

from __future__ import annotations

import json
import os
import subprocess
import fcntl
from functools import cache
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]
FIXTURE_GRAPH = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)
RELEASE_SITE_DIST = PROJECT_ROOT / "release-site" / "dist"


def test_channel_name_not_repeated() -> None:
    build_release_site_from_fixture()

    index = (RELEASE_SITE_DIST / "index.html").read_text(encoding="utf-8")

    assert "Channels" in index
    assert "Stable" in index
    assert "Nightly" in index
    assert "<code>stable</code>" not in index
    assert "<code>nightly</code>" not in index
    assert "Recommended release channel" in index
    assert "Faster-moving release channel" in index


def test_channel_descriptions() -> None:
    build_release_site_from_fixture()

    index = (RELEASE_SITE_DIST / "index.html").read_text(encoding="utf-8")

    assert "Recommended release channel for everyday Capsem installs." in index
    assert "Faster-moving release channel for daily fixes and early validation." in index


def test_channel_manifest_revision_not_selected_manifest() -> None:
    build_release_site_from_fixture()

    index = (RELEASE_SITE_DIST / "index.html").read_text(encoding="utf-8")

    assert "Manifest revision" in index
    assert "Current manifest" not in index
    assert "Selected manifest" not in index
    assert "<code>1.0.2</code>" in index
    assert "1.5.0-nightly.20260702" not in index


def test_channel_list_has_no_status_or_records_theater() -> None:
    build_release_site_from_fixture()

    index = (RELEASE_SITE_DIST / "index.html").read_text(encoding="utf-8")

    assert ">Status<" not in index
    assert ">Records<" not in index
    assert "manifest records" not in index
    assert ">History<" not in index
    assert "Updated" in index
    assert "Coverage" in index
    assert "2026-07-03T05:45:26Z" in index
    assert "3 packages" in index
    assert "2 profiles" in index
    assert "arm64, x86_64" in index


def test_root_channel_manifest_metadata() -> None:
    build_release_site_from_fixture()

    index = (RELEASE_SITE_DIST / "index.html").read_text(encoding="utf-8")

    assert "Manifest revision" in index
    assert "Updated" in index
    assert "Coverage" in index
    assert "Manifest URL" in index
    assert "Selected manifest" not in index
    assert ">Status<" not in index
    assert ">Records<" not in index
    assert "/assets/stable/manifest.json" in index
    assert "/assets/nightly/manifest.json" in index
    assert "<code>1.0.2</code>" in index
    assert "2026-07-03T05:45:26Z" in index
    assert "3 packages" in index
    assert "2 profiles" in index
    assert "arm64, x86_64" in index


def test_one_manifest_url() -> None:
    build_release_site_from_fixture()

    for channel in ("stable", "nightly"):
        page = (
            RELEASE_SITE_DIST / "channels" / channel / "index.html"
        ).read_text(encoding="utf-8")

        assert f"/assets/{channel}/manifest.json" in page
        assert f"/manifests/{channel}/" not in page
        assert "/profiles/releases/" not in page
        assert "catalog.json" not in page
        assert "profile_catalog" not in page


def test_no_catalog_url_on_channel_page() -> None:
    build_release_site_from_fixture()

    for channel in ("stable", "nightly"):
        page = (
            RELEASE_SITE_DIST / "channels" / channel / "index.html"
        ).read_text(encoding="utf-8")

        assert f"/assets/{channel}/manifest.json" in page
        assert "Profile Catalog" not in page
        assert "catalog.json" not in page
        assert "profile_catalog" not in page
        assert "capsem.profile_catalog" not in page


def test_digest_display_truncates_human_hashes_and_preserves_machine_json() -> None:
    build_release_site_from_fixture()

    graph = fixture_graph()
    full_hashes = [
        graph["channels"]["stable"]["manifests"][0]["digest"]["sha256"],
        graph["manifests"]["stable"]["1.0.2"]["packages"][0]["digest"]["sha256"],
        graph["manifests"]["stable"]["1.0.2"]["packages"][0]["binaries"][0][
            "digest"
        ]["sha256"],
        graph["manifests"]["stable"]["1.0.2"]["profiles"]["co-work"]["architectures"][
            0
        ]["config"][0]["digest"]["sha256"],
    ]
    rendered = "\n".join(
        path.read_text(encoding="utf-8")
        for path in [
            RELEASE_SITE_DIST / "index.html",
            RELEASE_SITE_DIST / "channels" / "stable" / "index.html",
            RELEASE_SITE_DIST
            / "channels"
            / "stable"
            / "packages"
            / graph["manifests"]["stable"]["1.0.2"]["packages"][0]["id"]
            / "index.html",
            RELEASE_SITE_DIST
            / "channels"
            / "stable"
            / "profiles"
            / "co-work"
            / "index.html",
        ]
    )

    for digest in full_hashes:
        assert len(digest) == 64
        assert digest in FIXTURE_GRAPH.read_text(encoding="utf-8")
        assert f"{digest[:8]}..." in rendered
        assert digest not in rendered


def test_package_target_sbom() -> None:
    build_release_site_from_fixture()

    graph = fixture_graph()
    stable = (RELEASE_SITE_DIST / "channels" / "stable" / "index.html").read_text(
        encoding="utf-8"
    )
    packages_section = stable.split("Capsem Packages", maxsplit=1)[1].split(
        "Profile References",
        maxsplit=1,
    )[0]

    for package in graph["manifests"]["stable"]["1.0.2"]["packages"]:
        platform = "macOS" if package["platform"] == "macos" else package["platform"].title()
        heading = f"Package target {platform} {package['architecture']}"
        target_section = packages_section.split(heading, maxsplit=1)[1]
        next_target = target_section.find("Package target ")
        if next_target >= 0:
            target_section = target_section[:next_target]
        sbom = next(item for item in package["evidence"] if item["kind"] == "sbom")

        assert package["name"] in target_section
        assert sbom["url"].split("/")[-1] in target_section
        assert f"{sbom['bytes']:,}" in target_section
        assert sbom["digest"]["sha256"][:8] + "..." in target_section
        assert sbom["digest"]["blake3"][:8] + "..." in target_section


def test_package_detail_navigation() -> None:
    build_release_site_from_fixture()

    graph = fixture_graph()
    stable = (RELEASE_SITE_DIST / "channels" / "stable" / "index.html").read_text(
        encoding="utf-8"
    )

    assert "Capsem Packages" in stable
    assert "Capsem Binaries" not in stable
    for package in graph["manifests"]["stable"]["1.0.2"]["packages"]:
        detail_href = f"/channels/stable/packages/{package['id']}/"
        assert detail_href in stable

        detail = (
            RELEASE_SITE_DIST
            / "channels"
            / "stable"
            / "packages"
            / package["id"]
            / "index.html"
        ).read_text(encoding="utf-8")
        assert "Contained Binaries" in detail
        assert "Package Evidence" in detail
        for binary in package["binaries"]:
            assert binary["installed_path"] in detail
            assert binary["installed_path"] not in stable


@cache
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


@cache
def fixture_graph() -> dict:
    return json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
