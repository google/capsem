"""Named release profile contract gates used by Sprinty."""

from __future__ import annotations

import json

from test_release_site_html_contract import (
    FIXTURE_GRAPH,
    RELEASE_SITE_DIST,
    build_release_site_from_fixture,
)


def _graph() -> dict:
    return json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))


def _current_manifest(graph: dict, channel: str) -> dict:
    record = next(
        item for item in graph["channels"][channel]["manifests"] if item["status"] == "current"
    )
    return graph["manifests"][channel][record["version"]]


def test_manifest_profile_payloads_per_architecture() -> None:
    graph = _graph()

    for channel in graph["channels"]:
        manifest = _current_manifest(graph, channel)
        for profile_id, profile in manifest["profiles"].items():
            assert "software" not in profile, profile_id
            assert "config" not in profile, profile_id
            assert "images" not in profile, profile_id
            architectures = profile["architectures"]
            assert architectures, profile_id
            for architecture in architectures:
                assert architecture["architecture"], profile_id
                assert isinstance(architecture["software"], list)
                assert isinstance(architecture["config"], list)
                assert isinstance(architecture["images"], list)
                assert isinstance(architecture["evidence"], list)


def test_profile_architecture_blocks() -> None:
    build_release_site_from_fixture()
    graph = _graph()

    for channel in graph["channels"]:
        manifest = _current_manifest(graph, channel)
        for profile_id, profile in manifest["profiles"].items():
            page = (
                RELEASE_SITE_DIST
                / "channels"
                / channel
                / "profiles"
                / profile_id
                / "index.html"
            ).read_text(encoding="utf-8")
            for architecture in profile["architectures"]:
                heading = f"Architecture {architecture['architecture']}"
                assert heading in page
                section = page.split(heading, maxsplit=1)[1].split("</section>", maxsplit=1)[0]
                assert "Installed Software" in section
                assert "Config Files" in section
                assert "Profile Images" in section
                assert "Profile Evidence" in section


def test_profile_evidence_at_architecture_top() -> None:
    build_release_site_from_fixture()
    graph = _graph()

    for channel in graph["channels"]:
        manifest = _current_manifest(graph, channel)
        for profile_id, profile in manifest["profiles"].items():
            page = (
                RELEASE_SITE_DIST
                / "channels"
                / channel
                / "profiles"
                / profile_id
                / "index.html"
            ).read_text(encoding="utf-8")
            for architecture in profile["architectures"]:
                section = page.split(
                    f"Architecture {architecture['architecture']}",
                    maxsplit=1,
                )[1].split("</section>", maxsplit=1)[0]

                assert section.index("Profile Evidence") < section.index(
                    "Installed Software"
                )
                evidence_block = section.split("Profile Evidence", maxsplit=1)[1].split(
                    "Installed Software",
                    maxsplit=1,
                )[0]
                image_block = section.split("Profile Images", maxsplit=1)[1]
                for evidence in architecture["evidence"]:
                    if evidence["kind"] == "software_inventory":
                        assert evidence["url"] in evidence_block
                    elif evidence["kind"] in {"abom", "obom"}:
                        assert evidence["url"] in image_block


def test_all_profile_config_artifacts_listed() -> None:
    build_release_site_from_fixture()
    graph = _graph()

    for channel in graph["channels"]:
        manifest = _current_manifest(graph, channel)
        for profile_id, profile in manifest["profiles"].items():
            page = (
                RELEASE_SITE_DIST
                / "channels"
                / channel
                / "profiles"
                / profile_id
                / "index.html"
            ).read_text(encoding="utf-8")
            config_urls = [
                item["url"]
                for architecture in profile["architectures"]
                for item in architecture["config"]
            ]
            assert len(config_urls) >= 8, profile_id
            for url in config_urls:
                assert url in page


def test_complete_profile_image_artifact_set() -> None:
    build_release_site_from_fixture()
    graph = _graph()

    for channel in graph["channels"]:
        manifest = _current_manifest(graph, channel)
        for profile_id, profile in manifest["profiles"].items():
            page = (
                RELEASE_SITE_DIST
                / "channels"
                / channel
                / "profiles"
                / profile_id
                / "index.html"
            ).read_text(encoding="utf-8")
            image_kinds = {
                image["kind"]
                for architecture in profile["architectures"]
                for image in architecture["images"]
            }
            assert {"kernel", "initrd", "rootfs"}.issubset(image_kinds), profile_id
            for architecture in profile["architectures"]:
                for image in architecture["images"]:
                    assert image["url"] in page


def test_software_inventory_is_architecture_scoped() -> None:
    graph = _graph()

    for channel in graph["channels"]:
        manifest = _current_manifest(graph, channel)
        for profile_id, profile in manifest["profiles"].items():
            for architecture in profile["architectures"]:
                arch = architecture["architecture"]
                assert architecture["software"], f"{channel}:{profile_id}:{arch}"
                for software in architecture["software"]:
                    assert software["architecture"] == arch
                    assert software["architecture"] != "all"
                    assert software["version"] != "unversioned"
                    assert software["digest"]["sha256"]
                    assert software["digest"]["blake3"]
                    assert software["evidence"]


def test_profile_has_min_capsem_version_not_current_binary() -> None:
    graph = _graph()

    for channel in graph["channels"]:
        manifest = _current_manifest(graph, channel)
        for profile_id, profile in manifest["profiles"].items():
            assert "current_binary" not in profile, profile_id
            assert "current_assets" not in profile, profile_id
            assert profile["min_capsem_version"] == "1.4.0"


def test_profile_package_inventory_per_architecture() -> None:
    graph = _graph()

    for channel in graph["channels"]:
        manifest = _current_manifest(graph, channel)
        assert manifest["packages"], channel
        assert {
            package["architecture"] for package in manifest["packages"]
        } == {"arm64", "amd64"}
        for profile_id, profile in manifest["profiles"].items():
            assert "packages" not in profile, profile_id
            for architecture in profile["architectures"]:
                arch = architecture["architecture"]
                assert "packages" not in architecture, f"{channel}:{profile_id}:{arch}"
                assert arch in {"arm64", "x86_64"}
