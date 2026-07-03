"""Release package and executable inventory contract gates."""

from __future__ import annotations

import json
from pathlib import Path

from test_release_site_html_contract import RELEASE_SITE_DIST, build_release_site_from_fixture


PROJECT_ROOT = Path(__file__).resolve().parents[1]
FIXTURE_GRAPH = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)


def test_package_owns_binaries() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        packages = manifest["packages"]

        assert packages, channel
        for package in packages:
            assert "name" in package, package
            assert "url" in package, package
            assert "digest" in package, package
            assert package["binaries"], package["name"]
            for binary in package["binaries"]:
                assert "package" not in binary, binary
                assert binary["name"], binary
                assert binary["version"], binary
                assert binary["installed_path"].startswith("/"), binary
                assert len(binary["digest"]["sha256"]) == 64, binary
                assert len(binary["digest"]["blake3"]) == 64, binary
                assert binary["sbom_component_ref"].startswith("SPDXRef-"), binary


def test_sbom_not_repeated_per_binary() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for package in manifest["packages"]:
            package_sboms = [
                item
                for item in package.get("evidence", [])
                if "sbom" in item["kind"].lower()
            ]
            assert package_sboms, package["name"]
            for binary in package["binaries"]:
                assert "evidence" not in binary, binary
                assert "package_evidence" not in binary, binary
                assert "sbom" not in binary, binary
                assert "sbom_component_ref" in binary, binary


def test_package_architecture_groups() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    stable = (
        RELEASE_SITE_DIST / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")
    packages_section = stable.split("Capsem Packages", maxsplit=1)[1].split(
        "Capsem Binaries",
        maxsplit=1,
    )[0]
    stable_packages = graph["manifests"]["stable"]["1.4.0"]["packages"]
    architectures = {package["architecture"] for package in stable_packages}

    for architecture in architectures:
        assert f"Architecture {architecture}" in packages_section
    for package in stable_packages:
        arch_position = packages_section.index(f"Architecture {package['architecture']}")
        package_position = packages_section.index(package["name"])
        assert arch_position < package_position


def test_macos_package_present() -> None:
    build_release_site_from_fixture()

    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    stable = (
        RELEASE_SITE_DIST / "channels" / "stable" / "index.html"
    ).read_text(encoding="utf-8")
    stable_packages = graph["manifests"]["stable"]["1.4.0"]["packages"]
    macos_packages = [
        package for package in stable_packages if package["kind"] == "macos_pkg"
    ]

    assert macos_packages
    for package in macos_packages:
        assert package["platform"] == "macos"
        assert package["name"].endswith(".pkg")
        assert package["name"] in stable
        assert package["url"] in stable
