"""Named release runtime contract gates."""

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


def _runtime_page(channel: str) -> str:
    return (RELEASE_SITE_DIST / "channels" / channel / "runtime" / "index.html").read_text(
        encoding="utf-8"
    )


def _architecture_section(page: str, architecture: str) -> str:
    heading = f"Architecture {architecture}"
    assert heading in page
    return page.split(heading, maxsplit=1)[1].split("</section>", maxsplit=1)[0]


def test_manifest_runtime_payloads_per_architecture() -> None:
    graph = _graph()

    for channel in graph["channels"]:
        runtime = _current_manifest(graph, channel)["runtime"]
        assert "software" not in runtime, channel
        assert "config" not in runtime, channel
        assert "images" not in runtime, channel
        architectures = runtime["architectures"]
        assert architectures, channel
        for architecture in architectures:
            assert architecture["architecture"], channel
            assert "config" not in architecture, channel
            assert isinstance(architecture["software"], list)
            assert isinstance(architecture["images"], list)
            assert isinstance(architecture["evidence"], list)


def test_runtime_architecture_blocks() -> None:
    build_release_site_from_fixture()
    graph = _graph()

    for channel in graph["channels"]:
        runtime = _current_manifest(graph, channel)["runtime"]
        page = _runtime_page(channel)
        for architecture in runtime["architectures"]:
            section = _architecture_section(page, architecture["architecture"])
            assert "Runtime Images" in section
            assert "Runtime Evidence" in section
            assert "Installed Software" in section
            assert "Config Files" not in section


def test_runtime_evidence_precedes_installed_software() -> None:
    build_release_site_from_fixture()
    graph = _graph()

    for channel in graph["channels"]:
        runtime = _current_manifest(graph, channel)["runtime"]
        page = _runtime_page(channel)
        for architecture in runtime["architectures"]:
            section = _architecture_section(page, architecture["architecture"])

            assert section.index("Runtime Evidence") < section.index("Installed Software")
            evidence_block = section.split("Runtime Evidence", maxsplit=1)[1].split(
                "Installed Software", maxsplit=1
            )[0]
            for evidence in architecture["evidence"]:
                assert evidence["url"] in evidence_block


def test_complete_runtime_image_artifact_set() -> None:
    build_release_site_from_fixture()
    graph = _graph()

    for channel in graph["channels"]:
        runtime = _current_manifest(graph, channel)["runtime"]
        page = _runtime_page(channel)
        for architecture in runtime["architectures"]:
            image_kinds = {image["kind"] for image in architecture["images"]}
            assert {"kernel", "initrd", "rootfs"} <= image_kinds, (
                f"{channel}:{architecture['architecture']}"
            )
            for image in architecture["images"]:
                assert image["url"] in page


def test_software_inventory_is_architecture_scoped() -> None:
    graph = _graph()

    for channel in graph["channels"]:
        runtime = _current_manifest(graph, channel)["runtime"]
        for architecture in runtime["architectures"]:
            arch = architecture["architecture"]
            assert architecture["software"], f"{channel}:{arch}"
            for software in architecture["software"]:
                assert software["architecture"] == arch
                assert software["architecture"] != "all"
                assert software["version"] != "unversioned"
                assert software["digest"]["sha256"]
                assert software["digest"]["blake3"]
                assert software["evidence"]


def test_runtime_has_min_capsem_version_not_current_binary() -> None:
    graph = _graph()

    for channel in graph["channels"]:
        runtime = _current_manifest(graph, channel)["runtime"]
        assert "current_binary" not in runtime, channel
        assert "current_assets" not in runtime, channel
        assert runtime["min_capsem_version"] == "1.4.0"


def test_runtime_has_no_package_inventory_of_its_own() -> None:
    graph = _graph()

    for channel in graph["channels"]:
        manifest = _current_manifest(graph, channel)
        assert manifest["packages"], channel
        assert {package["architecture"] for package in manifest["packages"]} == {"arm64", "amd64"}
        assert "packages" not in manifest["runtime"], channel
        for architecture in manifest["runtime"]["architectures"]:
            arch = architecture["architecture"]
            assert "packages" not in architecture, f"{channel}:{arch}"
            assert arch in {"arm64", "x86_64"}
