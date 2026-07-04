"""Release profile architecture and ownership contract gates."""

from __future__ import annotations

from test_release_site_html_contract import (
    FIXTURE_GRAPH,
    PROJECT_ROOT,
    RELEASE_SITE_DIST,
    build_release_site_from_fixture,
)

from copy import deepcopy
import hashlib
import importlib.util
import json
import re
import sys
import tomllib

from blake3 import blake3
from pytest import MonkeyPatch


PROFILE_PAGE = (
    PROJECT_ROOT / "release-site" / "src" / "pages" / "channels" / "[channel]" / "profiles" / "[id].astro"
)
SEMVER_RE = re.compile(
    r"^(0|[1-9]\d*)\."
    r"(0|[1-9]\d*)\."
    r"(0|[1-9]\d*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)
CONFIG_KIND_LABELS = {
    "profile": "Profile metadata",
    "mcp": "MCP configuration",
    "enforcement": "Enforcement rules",
    "detection": "Detection rules",
    "apt": "APT package list",
    "python": "Python requirements",
    "npm": "NPM package list",
    "build": "Build script",
    "tips": "Usage tips",
    "root_manifest": "Root manifest",
}


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


def test_profile_architecture_packages_and_images_are_separate() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for profile_id, profile in manifest["profiles"].items():
            assert "packages" not in profile, f"{channel}:{profile_id}"
            for architecture in profile["architectures"]:
                label = f"{channel}:{profile_id}:{architecture['architecture']}"
                assert set(architecture) == {
                    "architecture",
                    "package_inventory_revision",
                    "image_revision",
                    "software",
                    "config",
                    "images",
                    "evidence",
                }, label
                assert "packages" not in architecture, label
                assert architecture["software"], label
                assert architecture["images"], label
                for software in architecture["software"]:
                    assert {"name", "version", "source", "architecture", "evidence", "digest"} <= set(
                        software
                    ), label
                    assert "url" not in software, label
                    assert software["evidence"].endswith("software-inventory.json"), label
                for image in architecture["images"]:
                    assert {"kind", "name", "url", "bytes", "digest", "status"} <= set(image), label
                    assert "source" not in image, label
                    assert "version" not in image, label


def test_profile_image_versions_are_semver_compatible() -> None:
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    seen_profile_versions: dict[tuple[str, str], str] = {}

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        package_versions = {package["version"] for package in manifest["packages"]}
        for profile_id, profile in manifest["profiles"].items():
            label = f"{channel}:{profile_id}"
            assert _is_semver(profile["version"]), label
            assert _is_semver(profile["revision"]), label
            assert profile["version"] == profile["revision"], label
            assert profile["revision"] != manifest["version"], label
            assert profile["revision"] not in package_versions, label
            seen_profile_versions[(channel, profile_id)] = profile["revision"]
            for architecture in profile["architectures"]:
                arch_label = f"{label}:{architecture['architecture']}"
                assert _is_semver(architecture["image_revision"]), arch_label
                assert architecture["image_revision"] == profile["revision"], arch_label
                assert architecture["image_revision"] not in package_versions, arch_label

    assert seen_profile_versions[("stable", "co-work")] != seen_profile_versions[
        ("nightly", "co-work")
    ]
    assert seen_profile_versions[("stable", "code")] != seen_profile_versions[
        ("nightly", "code")
    ]


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
                profile_evidence_block = section.split("Profile Evidence", maxsplit=1)[1].split(
                    "Installed Software",
                    maxsplit=1,
                )[0]
                image_block = section.split("Profile Images", maxsplit=1)[1]
                evidence = {
                    item["kind"]: item
                    for item in architecture["evidence"]
                    if item["kind"] in {"abom", "obom", "software_inventory"}
                }
                assert set(evidence) == {"abom", "obom", "software_inventory"}, (
                    f"{channel}:{profile_id}:{arch}"
                )
                for kind, item in evidence.items():
                    if kind == "software_inventory":
                        assert item["url"].endswith(
                            f"/{arch}-software-inventory.json"
                        ), f"{channel}:{profile_id}:{arch}:{kind}"
                        assert item["url"] in profile_evidence_block, (
                            f"{channel}:{profile_id}:{arch}:{kind}"
                        )
                    else:
                        assert f"/{arch}/" in item["url"], f"{channel}:{profile_id}:{arch}:{kind}"
                        assert item["url"] in image_block, f"{channel}:{profile_id}:{arch}:{kind}"
                        assert item["url"] not in profile_evidence_block, (
                            f"{channel}:{profile_id}:{arch}:{kind}"
                        )

            for other_profile_id, other_profile in manifest["profiles"].items():
                if other_profile_id == profile_id:
                    continue
                for architecture in other_profile["architectures"]:
                    for item in architecture["evidence"]:
                        if item["kind"] in {"abom", "obom"}:
                            assert item["url"] not in page, (
                                f"{channel}:{profile_id} leaked {other_profile_id}:{item['url']}"
                            )


def test_abom_obom_image_scoped_evidence(monkeypatch) -> None:
    checker = _readiness_checker_module()
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    monkeypatch.setattr(
        checker,
        "fetch_text",
        lambda _url: checker.FetchText(text="profile page"),
    )
    monkeypatch.setattr(
        checker,
        "check_release_graph_artifact",
        lambda *_args, **_kwargs: [],
    )

    for channel, record in graph["channels"].items():
        channel_page = (RELEASE_SITE_DIST / "channels" / channel / "index.html").read_text(
            encoding="utf-8"
        )
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for profile_id, profile in manifest["profiles"].items():
            profile_page = (
                RELEASE_SITE_DIST
                / "channels"
                / channel
                / "profiles"
                / profile_id
                / "index.html"
            ).read_text(encoding="utf-8")
            for architecture in profile["architectures"]:
                arch = architecture["architecture"]
                label = f"{channel}:{profile_id}:{arch}"
                section = profile_page.split(f"Architecture {arch}", maxsplit=1)[1].split(
                    "</section>",
                    maxsplit=1,
                )[0]
                profile_evidence_block = section.split("Profile Evidence", maxsplit=1)[1].split(
                    "Installed Software",
                    maxsplit=1,
                )[0]
                image_evidence_block = section.split("Profile Image Evidence", maxsplit=1)[1]
                image_evidence = [
                    item for item in architecture["evidence"] if item["kind"] in {"abom", "obom"}
                ]

                assert {item["kind"] for item in image_evidence} == {"abom", "obom"}, label
                for evidence in image_evidence:
                    assert f"/{arch}/" in evidence["url"], label
                    assert evidence["url"] in image_evidence_block, label
                    assert evidence["url"] not in profile_evidence_block, label
                    assert evidence["url"] not in channel_page, label

                invalid_profile = deepcopy(profile)
                invalid_architecture = next(
                    item
                    for item in invalid_profile["architectures"]
                    if item["architecture"] == arch
                )
                invalid_evidence = next(
                    item
                    for item in invalid_architecture["evidence"]
                    if item["kind"] in {"abom", "obom"}
                )
                wrong_arch = "x86_64" if arch != "x86_64" else "arm64"
                invalid_evidence["url"] = invalid_evidence["url"].replace(
                    f"/{arch}/",
                    f"/{wrong_arch}/",
                )

                failures = checker.check_release_graph_profile(
                    "https://release.capsem.test",
                    channel,
                    profile_id,
                    invalid_profile,
                )
                assert any(
                    f"profile {profile_id} architecture {arch} evidence "
                    f"{invalid_evidence['kind']} url must include /{arch}/" in failure
                    for failure in failures
                ), label


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


def test_profile_evidence_scoped_to_image_artifacts() -> None:
    test_abom_obom_architecture_scoped()
    test_abom_obom_image_scoped_evidence(MonkeyPatch())
    test_profile_image_evidence()


def test_image_removal_is_absence_not_status(monkeypatch) -> None:
    checker = _readiness_checker_module()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    channel = "stable"
    profile_id = "co-work"
    current = next(
        item for item in graph["channels"][channel]["manifests"] if item["status"] == "current"
    )
    manifest = graph["manifests"][channel][current["version"]]
    profile = deepcopy(manifest["profiles"][profile_id])
    architecture = next(
        item for item in profile["architectures"] if item["architecture"] == "arm64"
    )
    removed_image = next(item for item in architecture["images"] if item["kind"] == "initrd")
    removed_key = (removed_image["kind"], removed_image["name"], removed_image["url"])

    architecture["images"] = [
        image
        for image in architecture["images"]
        if (image["kind"], image["name"], image["url"]) != removed_key
    ]

    assert removed_key not in {
        (image["kind"], image["name"], image["url"]) for image in architecture["images"]
    }
    assert not list(_walk_values(profile, "removed"))
    assert {item["status"] for item in graph["channels"][channel]["manifests"]} == {
        "current",
        "supported",
        "deprecated",
        "revoked",
    }
    assert any(
        item["status"] != "current" and item["url"].startswith(f"/manifests/{channel}/")
        for item in graph["channels"][channel]["manifests"]
    )

    monkeypatch.setattr(
        checker,
        "fetch_text",
        lambda _url: checker.FetchText(
            text=f"{profile_id} {profile['name']} {profile['revision']} arm64"
        ),
    )
    monkeypatch.setattr(
        checker,
        "check_release_graph_artifact",
        lambda *_args, **_kwargs: [],
    )

    valid_failures = checker.check_release_graph_profile(
        "https://release.capsem.test",
        channel,
        profile_id,
        profile,
    )
    assert not [failure for failure in valid_failures if "removed" in failure.lower()]

    invalid_profile = deepcopy(profile)
    invalid_architecture = next(
        item for item in invalid_profile["architectures"] if item["architecture"] == "arm64"
    )
    invalid_architecture["images"].append({**removed_image, "status": "removed"})

    invalid_failures = checker.check_release_graph_profile(
        "https://release.capsem.test",
        channel,
        profile_id,
        invalid_profile,
    )
    assert any("status removed is not allowed" in failure for failure in invalid_failures)


def test_image_entries_own_abom_obom() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        channel_page = (RELEASE_SITE_DIST / "channels" / channel / "index.html").read_text(
            encoding="utf-8"
        )
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for profile_id, profile in manifest["profiles"].items():
            profile_page = (
                RELEASE_SITE_DIST
                / "channels"
                / channel
                / "profiles"
                / profile_id
                / "index.html"
            ).read_text(encoding="utf-8")
            for architecture in profile["architectures"]:
                label = f"{channel}:{profile_id}:{architecture['architecture']}"
                section = profile_page.split(
                    f"Architecture {architecture['architecture']}",
                    maxsplit=1,
                )[1].split("</section>", maxsplit=1)[0]
                profile_evidence_block = section.split("Profile Evidence", maxsplit=1)[1].split(
                    "Installed Software",
                    maxsplit=1,
                )[0]
                image_block = section.split("Profile Images", maxsplit=1)[1]
                image_evidence = [
                    item
                    for item in architecture["evidence"]
                    if item["kind"] in {"abom", "obom"}
                ]

                assert {item["kind"] for item in image_evidence} == {"abom", "obom"}, label
                for item in image_evidence:
                    assert item["url"] in image_block, label
                    assert item["url"] not in profile_evidence_block, label
                    assert item["url"] not in channel_page, label


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


def test_software_inventory_evidence_once_per_architecture() -> None:
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
                evidence_block = section.split("Profile Evidence", maxsplit=1)[1].split(
                    "Installed Software",
                    maxsplit=1,
                )[0]
                software_block = section.split("Installed Software", maxsplit=1)[1].split(
                    "Config Files",
                    maxsplit=1,
                )[0]
                software_inventory = [
                    item
                    for item in architecture["evidence"]
                    if item["kind"] == "software_inventory"
                ]

                assert len(software_inventory) == 1, label
                evidence_url = software_inventory[0]["url"]
                assert evidence_block.count(f'href="{evidence_url}"') == 1, label
                assert evidence_url not in software_block, label


def test_software_inventory_evidence_link_once_per_architecture_block() -> None:
    test_software_inventory_evidence_once_per_architecture()


def test_profile_architecture_sections() -> None:
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
                heading = f"Architecture {architecture['architecture']}"
                assert heading in page, label
                section = page.split(heading, maxsplit=1)[1].split("</section>", maxsplit=1)[0]
                assert "Profile Evidence" in section, label
                assert "Installed Software" in section, label
                assert "Config Files" in section, label
                assert "Profile Images" in section, label

                for software in architecture["software"]:
                    assert software["name"] in section, label
                    assert software["evidence"] in section, label
                for config in architecture["config"]:
                    assert config["path"] in section, label
                    assert config["url"] in section, label
                for image in architecture["images"]:
                    assert image["name"] in section, label
                    assert image["url"] in section, label
                for evidence in architecture["evidence"]:
                    assert evidence["url"] in section, label


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
    manifest = graph["manifests"]["stable"]["1.0.2"]
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


def test_profile_arch_packages_and_images_blocks() -> None:
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
                evidence_block = section.split("Profile Evidence", maxsplit=1)[1].split(
                    "Installed Software",
                    maxsplit=1,
                )[0]
                software_block = section.split("Installed Software", maxsplit=1)[1].split(
                    "Config Files",
                    maxsplit=1,
                )[0]
                config_block = section.split("Config Files", maxsplit=1)[1].split(
                    "Profile Images",
                    maxsplit=1,
                )[0]
                image_block = section.split("Profile Images", maxsplit=1)[1]

                for software in architecture["software"]:
                    assert software["name"] in software_block, label
                    assert software["version"] in software_block, label
                    assert software["name"] not in image_block, label
                for config in architecture["config"]:
                    assert config["url"] in config_block, label
                    assert config["url"] not in software_block, label
                    assert config["url"] not in image_block, label
                for image in architecture["images"]:
                    assert image["url"] in image_block, label
                    assert image["url"] not in software_block, label
                    assert image["url"] not in config_block, label
                for evidence in architecture["evidence"]:
                    owner_block = image_block if evidence["kind"] in {"abom", "obom"} else evidence_block
                    assert evidence["url"] in owner_block, label


def test_profile_page_has_architecture_package_and_image_blocks() -> None:
    test_profile_arch_packages_and_images_blocks()


def _walk_values(value: object, needle: str) -> list[str]:
    matches: list[str] = []

    def visit(item: object, path: str) -> None:
        if isinstance(item, dict):
            for item_key, item_value in item.items():
                next_path = f"{path}.{item_key}" if path else str(item_key)
                visit(item_value, next_path)
        elif isinstance(item, list):
            for index, item_value in enumerate(item):
                visit(item_value, f"{path}[{index}]")
        elif item == needle:
            matches.append(path)

    visit(value, "")
    return matches


def _is_semver(value: str) -> bool:
    return bool(SEMVER_RE.fullmatch(value))


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


def test_all_config_classes_render() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    required_kinds = {
        "profile",
        "mcp",
        "enforcement",
        "detection",
        "apt",
        "python",
        "npm",
        "build",
        "tips",
        "root_manifest",
    }

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
                kinds = {config["kind"] for config in architecture["config"]}
                assert required_kinds.issubset(kinds), label

                section = page.split(f"Architecture {architecture['architecture']}", maxsplit=1)[
                    1
                ].split("</section>", maxsplit=1)[0]
                config_block = section.split("Config Files", maxsplit=1)[1].split(
                    "Profile Images",
                    maxsplit=1,
                )[0]
                for config in architecture["config"]:
                    assert CONFIG_KIND_LABELS[config["kind"]] in config_block, label
                    assert config["path"] in config_block, label
                    assert config["url"] in config_block, label


def test_profile_config_inventory_includes_security_and_detection() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    file_kind_map = {
        "apt_packages": "apt",
        "python_requirements": "python",
        "npm_packages": "npm",
    }

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for profile_id, profile in manifest["profiles"].items():
            profile_config = tomllib.loads(
                (PROJECT_ROOT / "config" / "profiles" / profile_id / "profile.toml").read_text(
                    encoding="utf-8"
                )
            )
            expected_paths = {
                file_kind_map.get(kind, kind): file_record["path"]
                for kind, file_record in profile_config["files"].items()
            }
            assert {"enforcement", "detection", "mcp"}.issubset(expected_paths), profile_id
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
                config_by_kind = {config["kind"]: config for config in architecture["config"]}
                assert expected_paths.keys() <= config_by_kind.keys(), label
                section = page.split(f"Architecture {architecture['architecture']}", maxsplit=1)[
                    1
                ].split("</section>", maxsplit=1)[0]
                config_block = section.split("Config Files", maxsplit=1)[1].split(
                    "Profile Images",
                    maxsplit=1,
                )[0]

                for kind, path in expected_paths.items():
                    assert config_by_kind[kind]["path"] == path, label
                    assert path in config_block, label
                    assert config_by_kind[kind]["url"] in config_block, label


def test_config_inventory_security_rules_detection() -> None:
    test_profile_config_inventory_includes_security_and_detection()


def test_all_config_classes() -> None:
    test_all_config_classes_render()
    test_profile_config_inventory_includes_security_and_detection()


def test_config_inventory_all_classes() -> None:
    test_all_config_classes_render()
    test_profile_config_inventory_includes_security_and_detection()

    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    expected_kinds = {
        "profile",
        "mcp",
        "enforcement",
        "detection",
        "apt",
        "python",
        "npm",
        "build",
        "tips",
        "root_manifest",
    }

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
                config_kinds = {config["kind"] for config in architecture["config"]}
                assert config_kinds == expected_kinds, label

                section = page.split(f"Architecture {architecture['architecture']}", maxsplit=1)[
                    1
                ].split("</section>", maxsplit=1)[0]
                config_block = section.split("Config Files", maxsplit=1)[1].split(
                    "Profile Images",
                    maxsplit=1,
                )[0]
                for kind in expected_kinds:
                    assert CONFIG_KIND_LABELS[kind] in config_block, label
                assert config_block.count("<tr class=\"border-b border-zinc-100 align-top\">") >= len(
                    expected_kinds
                ), label


def test_config_kind_enum_contract(monkeypatch) -> None:
    checker = _readiness_checker_module()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    profile = json.loads(json.dumps(graph["manifests"]["stable"]["1.0.2"]["profiles"]["co-work"]))
    generated_kind_profile = json.loads(json.dumps(profile))
    for architecture in generated_kind_profile["architectures"]:
        for config in architecture["config"]:
            config["kind"] = {
                "apt": "apt_packages",
                "python": "python_requirements",
                "npm": "npm_packages",
            }.get(config["kind"], config["kind"])
    profile["architectures"][0]["config"][0]["kind"] = "misc"

    monkeypatch.setattr(
        checker,
        "fetch_text",
        lambda _url: checker.FetchText(
            text="co-work Co-work 1.0.0-stable.20260702 arm64"
        ),
    )
    monkeypatch.setattr(
        checker,
        "check_release_graph_artifact",
        lambda *_args, **_kwargs: [],
    )

    generated_kind_failures = checker.check_release_graph_profile(
        "https://release.capsem.test",
        "stable",
        "co-work",
        generated_kind_profile,
    )
    assert not [
        failure
        for failure in generated_kind_failures
        if "config kind" in failure and "is not allowed" in failure
    ]

    failures = checker.check_release_graph_profile(
        "https://release.capsem.test",
        "stable",
        "co-work",
        profile,
    )

    assert (
        "profile co-work architecture arm64 config kind misc is not allowed"
        in failures
    )


def test_config_kinds_are_typed_enums() -> None:
    build_release_site_from_fixture()
    source = PROFILE_PAGE.read_text(encoding="utf-8")
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    assert "configKindLabel" in source
    assert "file.kind ?? file.label" not in source

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
                config_block = section.split("Config Files", maxsplit=1)[1].split(
                    "Profile Images",
                    maxsplit=1,
                )[0]

                for config in architecture["config"]:
                    expected_label = CONFIG_KIND_LABELS[config["kind"]]
                    assert expected_label in config_block, label
                    assert f">{config['kind']}<" not in config_block, label
                    assert config["path"] in config_block, label


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
                label = f"{channel}:{profile_id}:{architecture['architecture']}"
                kinds = {image["kind"] for image in architecture["images"]}
                assert {"kernel", "initrd", "rootfs"}.issubset(kinds), label
                section = page.split(f"Architecture {architecture['architecture']}", maxsplit=1)[
                    1
                ].split("</section>", maxsplit=1)[0]
                image_block = section.split("Profile Images", maxsplit=1)[1]
                for image in architecture["images"]:
                    assert image["kind"] in image_block, label
                    assert image["name"] in image_block, label
                    assert image["url"] in image_block, label
                    assert image["digest"]["sha256"][:8] + "..." in image_block, label
                    assert image["digest"]["blake3"][:8] + "..." in image_block, label


def test_profile_images_grouped_by_architecture_complete_set(monkeypatch) -> None:
    checker = _readiness_checker_module()
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    monkeypatch.setattr(
        checker,
        "fetch_text",
        lambda _url: checker.FetchText(text="profile page"),
    )
    monkeypatch.setattr(
        checker,
        "check_release_graph_artifact",
        lambda *_args, **_kwargs: [],
    )

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for profile_id, profile in manifest["profiles"].items():
            profile_page = (
                RELEASE_SITE_DIST
                / "channels"
                / channel
                / "profiles"
                / profile_id
                / "index.html"
            ).read_text(encoding="utf-8")
            for architecture in profile["architectures"]:
                arch = architecture["architecture"]
                label = f"{channel}:{profile_id}:{arch}"
                section = profile_page.split(f"Architecture {arch}", maxsplit=1)[1].split(
                    "</section>",
                    maxsplit=1,
                )[0]
                image_block = section.split("Profile Images", maxsplit=1)[1].split(
                    "Profile Image Evidence",
                    maxsplit=1,
                )[0]
                image_kinds = {image["kind"] for image in architecture["images"]}
                assert image_kinds == {"kernel", "initrd", "rootfs"}, label
                for image in architecture["images"]:
                    assert f"/{arch}/" in image["url"], label
                    assert image["url"] in image_block, label

                invalid_profile = deepcopy(profile)
                invalid_architecture = next(
                    item
                    for item in invalid_profile["architectures"]
                    if item["architecture"] == arch
                )
                removed_kind = next(iter(image_kinds))
                invalid_architecture["images"] = [
                    image
                    for image in invalid_architecture["images"]
                    if image["kind"] != removed_kind
                ]

                failures = checker.check_release_graph_profile(
                    "https://release.capsem.test",
                    channel,
                    profile_id,
                    invalid_profile,
                )
                assert any(
                    f"profile {profile_id} architecture {arch} images missing {removed_kind}"
                    in failure
                    for failure in failures
                ), label


def test_profile_image_complete_architecture_bundle() -> None:
    test_all_profile_image_artifacts()
    test_profile_images_grouped_by_architecture_complete_set(MonkeyPatch())


def test_software_inventory_not_all_arch() -> None:
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
                    "</section>",
                    maxsplit=1,
                )[0]
                software_block = section.split("Installed Software", maxsplit=1)[1].split(
                    "Config Files",
                    maxsplit=1,
                )[0]
                assert ">all<" not in software_block
                assert "<code>all</code>" not in software_block
                for software in architecture["software"]:
                    assert software["architecture"] == arch
                    assert software["architecture"] != "all"
                    assert software["name"] in software_block


def test_software_inventory_grouped_by_architecture_blocks() -> None:
    build_release_site_from_fixture()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    for channel, record in graph["channels"].items():
        current = next(item for item in record["manifests"] if item["status"] == "current")
        manifest = graph["manifests"][channel][current["version"]]
        for profile_id, profile in manifest["profiles"].items():
            assert "software" not in profile, f"{channel}:{profile_id}"
            page = (
                RELEASE_SITE_DIST
                / "channels"
                / channel
                / "profiles"
                / profile_id
                / "index.html"
            ).read_text(encoding="utf-8")

            sections = {}
            for architecture in profile["architectures"]:
                arch = architecture["architecture"]
                label = f"{channel}:{profile_id}:{arch}"
                assert architecture["software"], label
                assert all(item["architecture"] == arch for item in architecture["software"]), label

                section = page.split(f"Architecture {arch}", maxsplit=1)[1].split(
                    "</section>",
                    maxsplit=1,
                )[0]
                software_block = section.split("Installed Software", maxsplit=1)[1].split(
                    "Config Files",
                    maxsplit=1,
                )[0]
                sections[arch] = software_block

                assert ">all<" not in software_block, label
                assert "<code>all</code>" not in software_block, label
                for software in architecture["software"]:
                    assert software["name"] in software_block, label
                    assert software["version"] in software_block, label
                    assert software["source"] in software_block, label
                    assert software["digest"]["sha256"][:8] + "..." in software_block, label
                    assert software["digest"]["blake3"][:8] + "..." in software_block, label

            for architecture in profile["architectures"]:
                arch = architecture["architecture"]
                other_blocks = [
                    block for other_arch, block in sections.items() if other_arch != arch
                ]
                for software in architecture["software"]:
                    for block in other_blocks:
                        assert software["digest"]["sha256"][:8] + "..." not in block, (
                            f"{channel}:{profile_id}:{arch}:{software['name']}"
                        )


def test_profile_software_inventory_stored_under_architecture_nodes() -> None:
    test_software_inventory_not_all_arch()
    test_software_inventory_grouped_by_architecture_blocks()


def test_software_versions_are_real(monkeypatch) -> None:
    checker = _readiness_checker_module()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))

    monkeypatch.setattr(
        checker,
        "fetch_text",
        lambda _url: checker.FetchText(
            text="co-work Co-work 1.0.0-stable.20260702 arm64"
        ),
    )
    monkeypatch.setattr(
        checker,
        "check_release_graph_artifact",
        lambda *_args, **_kwargs: [],
    )

    forbidden = {
        "": "version missing",
        "unversioned": "version is unversioned",
        "unknown": "version is unknown",
        "latest": "version is latest",
    }
    for version, expected in forbidden.items():
        profile = json.loads(json.dumps(graph["manifests"]["stable"]["1.0.2"]["profiles"]["co-work"]))
        profile["architectures"][0]["software"][0]["version"] = version

        failures = checker.check_release_graph_profile(
            "https://release.capsem.test",
            "stable",
            "co-work",
            profile,
        )

        assert f"profile co-work architecture arm64 software python {expected}" in failures


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


def test_software_rows_do_not_reuse_inventory_digest(monkeypatch) -> None:
    checker = _readiness_checker_module()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    profile = json.loads(json.dumps(graph["manifests"]["stable"]["1.0.2"]["profiles"]["co-work"]))
    architecture = profile["architectures"][0]
    inventory_digest = next(
        item["digest"]
        for item in architecture["evidence"]
        if item["kind"] == "software_inventory"
    )
    architecture["software"][0]["digest"] = inventory_digest

    monkeypatch.setattr(
        checker,
        "fetch_text",
        lambda _url: checker.FetchText(
            text="co-work Co-work 1.0.0-stable.20260702 arm64"
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

    assert (
        "profile co-work architecture arm64 software python digest reuses software_inventory evidence digest"
        in failures
    )


def test_real_software_versions(monkeypatch) -> None:
    test_software_versions_are_real(monkeypatch)
    test_software_versions_and_hashes()
    test_software_rows_do_not_reuse_inventory_digest(monkeypatch)


def test_software_inventory_real_versions(monkeypatch) -> None:
    test_real_software_versions(monkeypatch)


def test_software_inventory_rows_require_real_versions(monkeypatch) -> None:
    test_software_inventory_real_versions(monkeypatch)


def test_software_inventory_rows_require_row_owned_hashes(monkeypatch) -> None:
    test_software_versions_and_hashes()
    test_software_rows_do_not_reuse_inventory_digest(monkeypatch)


def test_software_inventory_rejects_repeated_hashes_for_distinct_rows(monkeypatch) -> None:
    checker = _readiness_checker_module()
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    profile = deepcopy(graph["manifests"]["stable"]["1.0.2"]["profiles"]["co-work"])
    architecture = profile["architectures"][0]
    first, second = architecture["software"][:2]
    second["digest"] = first["digest"]

    monkeypatch.setattr(
        checker,
        "fetch_text",
        lambda _url: checker.FetchText(
            text="co-work Co-work 1.0.0-stable.20260702 arm64"
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

    assert (
        "profile co-work architecture arm64 software "
        f"digest {first['digest']['sha256']} is reused by {first['name']} and {second['name']}"
        in failures
    )


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
