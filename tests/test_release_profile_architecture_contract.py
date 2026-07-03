"""Release profile architecture and ownership contract gates."""

from __future__ import annotations

from test_release_site_html_contract import (
    FIXTURE_GRAPH,
    PROJECT_ROOT,
    RELEASE_SITE_DIST,
    build_release_site_from_fixture,
)

import hashlib
import importlib.util
import json
import sys

from blake3 import blake3


PROFILE_PAGE = (
    PROJECT_ROOT / "release-site" / "src" / "pages" / "channels" / "[channel]" / "profiles" / "[id].astro"
)


def _readiness_checker_module():
    module_path = PROJECT_ROOT / "scripts" / "check-remote-release-readiness.py"
    spec = importlib.util.spec_from_file_location("check_remote_release_readiness", module_path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_profile_no_current_binary() -> None:
    build_release_site_from_fixture()

    source = PROFILE_PAGE.read_text(encoding="utf-8")
    stable = (
        RELEASE_SITE_DIST
        / "channels"
        / "stable"
        / "profiles"
        / "co-work"
        / "index.html"
    ).read_text(encoding="utf-8")

    assert "current_binary" not in source
    assert "current_assets" not in source
    assert "compatibility?.min_binary" not in source
    assert "Current binary" not in stable
    assert "current_binary" not in stable
    assert "Current assets" not in stable
    assert "current_assets" not in stable
    assert "Minimum Capsem" in stable


def test_manifest_profile_architecture_shape() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for profile in manifest["profiles"].values():
            assert "software" not in profile, profile["id"]
            assert "config" not in profile, profile["id"]
            assert "images" not in profile, profile["id"]
            architectures = profile["architectures"]
            assert architectures, profile["id"]
            for architecture in architectures:
                assert architecture["architecture"], profile["id"]
                assert architecture["software"], architecture
                assert architecture["config"], architecture
                assert architecture["images"], architecture
                evidence_kinds = {item["kind"] for item in architecture["evidence"]}
                assert {"abom", "obom", "software_inventory"}.issubset(evidence_kinds)


def test_abom_obom_architecture_scoped() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
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
                arch = architecture["architecture"]
                section = page.split(f"Architecture {arch}", maxsplit=1)[1].split(
                    "</section>", maxsplit=1
                )[0]
                evidence = {
                    item["kind"]: item
                    for item in architecture["evidence"]
                    if item["kind"] in {"abom", "obom", "software_inventory"}
                }
                assert set(evidence) == {"abom", "obom", "software_inventory"}, (
                    f"{channel}:{profile_id}:{arch}"
                )
                for kind, item in evidence.items():
                    assert f"/{arch}/" in item["url"], f"{channel}:{profile_id}:{arch}:{kind}"
                    assert item["url"] in section, f"{channel}:{profile_id}:{arch}:{kind}"

            for other_profile_id, other_profile in manifest["profiles"].items():
                if other_profile_id == profile_id:
                    continue
                for architecture in other_profile["architectures"]:
                    for item in architecture["evidence"]:
                        if item["kind"] in {"abom", "obom"}:
                            assert item["url"] not in page, (
                                f"{channel}:{profile_id} leaked {other_profile_id}:{item['url']}"
                            )


def test_profile_image_evidence() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
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
                label = f"{channel}:{profile_id}:{architecture['architecture']}"
                section = page.split(f"Architecture {architecture['architecture']}", maxsplit=1)[
                    1
                ].split("</section>", maxsplit=1)[0]
                image_block = section.split("Profile Images", maxsplit=1)[1]
                evidence_by_kind = {
                    item["kind"]: item
                    for item in architecture["evidence"]
                    if item["kind"] in {"abom", "obom"}
                }
                assert set(evidence_by_kind) == {"abom", "obom"}, label
                for kind, evidence in evidence_by_kind.items():
                    assert f"/{architecture['architecture']}/" in evidence["url"], label
                    assert evidence["url"] in image_block, f"{label}:{kind}"


def test_installed_inventory_not_channel_packages() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        channel_package_names = {package["name"] for package in manifest["packages"]}
        channel_package_urls = {package["url"] for package in manifest["packages"]}

        for profile_id, profile in manifest["profiles"].items():
            assert "packages" not in profile
            page = (
                RELEASE_SITE_DIST
                / "channels"
                / channel
                / "profiles"
                / profile_id
                / "index.html"
            ).read_text(encoding="utf-8")
            for architecture in profile["architectures"]:
                assert "packages" not in architecture
                label = f"{channel}:{profile_id}:{architecture['architecture']}"
                section = page.split(f"Architecture {architecture['architecture']}", maxsplit=1)[
                    1
                ].split("</section>", maxsplit=1)[0]
                installed_block = section.split("Installed Software", maxsplit=1)[1].split(
                    "Config Files",
                    maxsplit=1,
                )[0]

                for software in architecture["software"]:
                    assert software["name"] in installed_block, label
                    assert software["version"] in installed_block, label
                for package_name in channel_package_names:
                    assert package_name not in installed_block, label
                for package_url in channel_package_urls:
                    assert package_url not in installed_block, label


def test_software_evidence_scope() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
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
                section = page.split(f"Architecture {architecture['architecture']}", maxsplit=1)[
                    1
                ].split("</section>", maxsplit=1)[0]
                evidence_block = section.split("Profile Evidence", maxsplit=1)[1].split(
                    "Installed Software",
                    maxsplit=1,
                )[0]
                software_block = section.split("Installed Software", maxsplit=1)[1].split(
                    "Config Files",
                    maxsplit=1,
                )[0]
                software_evidence = {
                    item["evidence"]
                    for item in architecture["software"]
                    if item.get("evidence")
                }

                assert software_evidence
                for evidence_url in software_evidence:
                    assert evidence_url in evidence_block
                    assert evidence_url not in software_block


def test_profile_architecture_sections() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    profile = graph["manifests"]["stable"]["1.4.0"]["profiles"]["co-work"]
    page = (
        RELEASE_SITE_DIST
        / "channels"
        / "stable"
        / "profiles"
        / "co-work"
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
        for software in architecture["software"]:
            assert software["name"] in section
            assert software["evidence"] in section
        for config in architecture["config"]:
            assert config["path"] in section
            assert config["url"] in section
        for image in architecture["images"]:
            assert image["name"] in section
            assert image["url"] in section
        for evidence in architecture["evidence"]:
            assert evidence["url"] in section


def test_all_profiles() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for profile_id in manifest["profiles"]:
            assert (
                RELEASE_SITE_DIST
                / "channels"
                / channel
                / "profiles"
                / profile_id
                / "index.html"
            ).is_file()


def test_config_and_images_are_separate() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    manifest = graph["manifests"]["stable"]["1.4.0"]
    page = (
        RELEASE_SITE_DIST
        / "channels"
        / "stable"
        / "profiles"
        / "co-work"
        / "index.html"
    ).read_text(encoding="utf-8")

    for architecture in manifest["profiles"]["co-work"]["architectures"]:
        section = page.split(f"Architecture {architecture['architecture']}", maxsplit=1)[1].split(
            "</section>", maxsplit=1
        )[0]
        config_section = section.split("Config Files", maxsplit=1)[1].split(
            "Profile Images", maxsplit=1
        )[0]
        image_section = section.split("Profile Images", maxsplit=1)[1]
        for config in architecture["config"]:
            assert config["url"] in config_section
            assert config["url"] not in image_section
        for image in architecture["images"]:
            assert image["url"] in image_section
            assert image["url"] not in config_section


def test_all_profile_config_files() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
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
                assert len(architecture["config"]) >= 8, f"{channel}:{profile_id}"
                for config in architecture["config"]:
                    assert config["url"] in page


def test_all_profile_image_artifacts() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
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
                kinds = {image["kind"] for image in architecture["images"]}
                assert {"kernel", "initrd", "rootfs"}.issubset(kinds), f"{channel}:{profile_id}"
                for image in architecture["images"]:
                    assert image["url"] in page


def test_software_inventory_not_all_arch() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for profile in manifest["profiles"].values():
            for architecture in profile["architectures"]:
                arch = architecture["architecture"]
                for software in architecture["software"]:
                    assert software["architecture"] == arch
                    assert software["architecture"] != "all"


def test_real_software_versions(monkeypatch) -> None:
    checker = _readiness_checker_module()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    profile = json.loads(json.dumps(graph["manifests"]["stable"]["1.4.0"]["profiles"]["co-work"]))
    profile["architectures"][0]["software"][0]["version"] = "unversioned"

    monkeypatch.setattr(
        checker,
        "fetch_text",
        lambda _url: checker.FetchText(
            text="co-work Co-work 2026.07.02.1-stable arm64"
        ),
    )
    monkeypatch.setattr(
        checker,
        "check_release_graph_artifact",
        lambda *_args, **_kwargs: [],
    )

    failures = checker.check_release_graph_profile(
        "https://release.capsem.test",
        "stable",
        "co-work",
        profile,
    )

    assert "profile co-work architecture arm64 software python version is unversioned" in failures


def test_software_versions_and_hashes() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    forbidden_versions = {"", "latest", "unknown", "unversioned"}

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for profile_id, profile in manifest["profiles"].items():
            for architecture in profile["architectures"]:
                evidence_digests = {
                    item["digest"]["sha256"]
                    for item in architecture["evidence"]
                    if item.get("digest")
                }
                for software in architecture["software"]:
                    label = f"{channel}:{profile_id}:{architecture['architecture']}:{software['name']}"
                    assert software["version"].strip().lower() not in forbidden_versions, label
                    assert software["digest"] == _software_row_digest(software), label
                    assert software["digest"]["sha256"] not in evidence_digests, label


def _software_row_digest(software: dict) -> dict[str, str]:
    row_core = {
        "name": software["name"],
        "version": software["version"],
        "source": software["source"],
        "architecture": software["architecture"],
        "evidence": software["evidence"],
    }
    payload = json.dumps(row_core, separators=(",", ":")).encode("utf-8")
    return {
        "sha256": hashlib.sha256(payload).hexdigest(),
        "blake3": blake3(payload).hexdigest(),
    }
