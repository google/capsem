"""Release output contract tests.

These tests intentionally assert the documented public graph shape. They are
expected to fail while the generator still emits legacy asset-channel output.
"""

from __future__ import annotations

import hashlib
import importlib.util
import json
import sys
import tomllib
from collections.abc import Iterator
from pathlib import Path
from typing import Any

import blake3
import pytest


PROJECT_ROOT = Path(__file__).resolve().parents[2]
CHANNEL = "stable"
FIXTURE_GRAPH = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)
PROFILE_CONFIG_ROOT = PROJECT_ROOT / "config" / "profiles"
FORBIDDEN_PROFILE_FIELDS = {
    "current_binary",
    "current_assets",
    "asset_version",
    "binary_version",
}
REQUIRED_PROFILE_CONFIG_FILES = {
    "apt-packages.txt",
    "build.sh",
    "detection.yaml",
    "enforcement.toml",
    "mcp.json",
    "npm-packages.txt",
    "profile.toml",
    "python-requirements.txt",
    "root.manifest.json",
    "tips.txt",
}
REQUIRED_IMAGE_ARTIFACT_KINDS = {"kernel", "initrd", "rootfs"}
REQUIRED_PACKAGE_KINDS = {"macos_pkg", "debian_package"}
REQUIRED_BINARY_NAMES = {"capsem-app", "capsem-tray"}
ALLOWED_RELEASE_STATUSES = {"current", "supported", "deprecated", "revoked"}

pytestmark = pytest.mark.build_chain


def _load_channel_helpers() -> Any:
    module_path = PROJECT_ROOT / "tests" / "capsem-release" / "test_release_channel_contract.py"
    spec = importlib.util.spec_from_file_location("release_channel_contract_helpers", module_path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


@pytest.fixture(scope="module")
def generated_release_dist(tmp_path_factory: pytest.TempPathFactory) -> Path:
    helpers = _load_channel_helpers()
    dist = tmp_path_factory.mktemp("release-output-contract") / "dist"
    helpers._build_release_channel(dist)
    return dist


def test_channel_manifest_records_are_versioned_graph_files(
    generated_release_dist: Path,
) -> None:
    channels = _read_json(generated_release_dist / "channels.json")
    channel = channels["channels"][CHANNEL]
    current = _current_manifest_record(channel)
    manifest_url = current["url"]

    assert manifest_url == f"/assets/{CHANNEL}/manifest.json"
    assert "profile_catalog" not in channel
    _assert_no_hmac(current, f"channels.{CHANNEL}.manifests.current")

    manifest_bytes = _read_bytes(generated_release_dist, manifest_url)
    digest = current["digest"]
    assert digest == {
        "sha256": hashlib.sha256(manifest_bytes).hexdigest(),
        "blake3": blake3.blake3(manifest_bytes).hexdigest(),
    }


def test_manifest_uses_package_owned_binary_graph(
    generated_release_dist: Path,
) -> None:
    manifest = _selected_manifest(generated_release_dist)

    assert "assets" not in manifest
    assert "binaries" not in manifest
    packages = manifest["packages"]
    assert isinstance(packages, list)
    assert packages
    assert REQUIRED_PACKAGE_KINDS <= {package.get("kind") for package in packages}

    binary_names = {
        binary.get("name")
        for package in packages
        for binary in package.get("binaries", [])
    }
    assert REQUIRED_BINARY_NAMES <= binary_names

    for index, package in enumerate(packages):
        context = f"packages[{index}]"
        assert isinstance(package["id"], str)
        assert isinstance(package["kind"], str)
        assert isinstance(package["name"], str)
        assert isinstance(package["url"], str)
        assert isinstance(package["bytes"], int)
        assert set(package["digest"]) == {"sha256", "blake3"}
        _assert_no_hmac(package, context)

        binaries = package["binaries"]
        assert isinstance(binaries, list)
        assert binaries
        for binary_index, binary in enumerate(binaries):
            binary_context = f"{context}.binaries[{binary_index}]"
            assert "package" not in binary
            assert isinstance(binary["name"], str)
            assert isinstance(binary["version"], str)
            assert binary["version"] != "unversioned"
            assert isinstance(binary["installed_path"], str)
            assert isinstance(binary["bytes"], int)
            assert set(binary["digest"]) == {"sha256", "blake3"}
            assert isinstance(binary["sbom_component_ref"], str)
            _assert_no_hmac(binary, binary_context)


def test_manifest_profiles_are_the_profile_contract(
    generated_release_dist: Path,
) -> None:
    manifest = _selected_manifest(generated_release_dist)

    manifest_profiles = manifest["profiles"]
    assert isinstance(manifest_profiles, dict)
    assert manifest_profiles
    assert "profile_catalog" not in manifest
    assert "catalog" not in manifest

    for profile_id, manifest_profile in manifest_profiles.items():
        _assert_profile_shape(profile_id, manifest_profile, f"manifest.profiles.{profile_id}")


def test_generated_release_has_no_public_profile_catalog_primitive(
    generated_release_dist: Path,
) -> None:
    forbidden_tokens = ("profile_catalog", "catalog.json", "capsem.profile_catalog")
    files_to_check = [
        generated_release_dist / "channels.json",
        generated_release_dist / "health.json",
        generated_release_dist / "assets" / CHANNEL / "manifest.json",
        generated_release_dist / "index.html",
        generated_release_dist / "channels" / CHANNEL / "index.html",
    ]
    files_to_check.extend(
        generated_release_dist
        / "channels"
        / CHANNEL
        / "profiles"
        / profile_id
        / "index.html"
        for profile_id in _selected_manifest(generated_release_dist)["profiles"]
    )

    catalog_files = [
        path.relative_to(generated_release_dist).as_posix()
        for path in generated_release_dist.rglob("catalog.json")
    ]
    assert catalog_files == []

    hits: list[str] = []
    for path in files_to_check:
        text = path.read_text(encoding="utf-8")
        for token in forbidden_tokens:
            if token in text:
                hits.append(f"{path.relative_to(generated_release_dist)} contains {token}")
    assert hits == []


def test_release_readiness_checker_uses_profile_contract_not_catalog() -> None:
    checker = (PROJECT_ROOT / "scripts" / "check-remote-release-readiness.py").read_text(
        encoding="utf-8"
    )
    forbidden_tokens = ("profile_catalog", "catalog.json", "capsem.profile_catalog")
    hits = [token for token in forbidden_tokens if token in checker]
    assert hits == []


def test_profile_owned_artifact_digests_match_files(
    generated_release_dist: Path,
) -> None:
    manifest = _selected_manifest(generated_release_dist)
    profiles = manifest["profiles"]
    assert isinstance(profiles, dict)
    assert profiles

    for profile_id, profile in profiles.items():
        for item in _profile_artifact_descriptors(profile):
            url = item["url"]
            payload = _read_bytes(generated_release_dist, url)
            digest = item["digest"]
            assert item["bytes"] == len(payload), f"{profile_id} {url} bytes"
            assert digest == {
                "sha256": hashlib.sha256(payload).hexdigest(),
                "blake3": blake3.blake3(payload).hexdigest(),
            }, f"{profile_id} {url} digest"


def test_pages_only_render_owned_release_facts(generated_release_dist: Path) -> None:
    manifest = _selected_manifest(generated_release_dist)
    root_page = (generated_release_dist / "index.html").read_text(encoding="utf-8")
    channel_page = (
        generated_release_dist / "channels" / CHANNEL / "index.html"
    ).read_text(encoding="utf-8")

    for page_name, page in (("root", root_page), ("channel", channel_page)):
        assert "HMAC" not in page, page_name
        assert "hmac" not in page, page_name
        assert "Evidence" not in page, page_name
        assert "Host SBOM" not in page, page_name
        assert "VM OBOM" not in page, page_name
        assert "Asset Release History" not in page, page_name
        assert "Current VM Assets" not in page, page_name
        assert "Software Inventory" not in page, page_name
        assert "VM OBOM" not in page, page_name
        assert "current_binary" not in page, page_name
        assert "current_assets" not in page, page_name

    profiles = manifest.get("profiles", {})
    assert isinstance(profiles, dict)
    assert profiles
    for profile_id, profile in profiles.items():
        profile_page = (
            generated_release_dist
            / "channels"
            / CHANNEL
            / "profiles"
            / profile_id
            / "index.html"
        ).read_text(encoding="utf-8")
        assert "HMAC" not in profile_page
        assert "hmac" not in profile_page
        assert "Capsem Binaries" not in profile_page
        assert "Current VM Assets" not in profile_page
        for field in FORBIDDEN_PROFILE_FIELDS:
            assert field not in profile_page
        for item in _profile_artifact_descriptors(profile):
            assert item["url"] in profile_page
            assert item["digest"]["sha256"] in profile_page
            assert item["digest"]["blake3"] in profile_page


def test_profile_software_inventory_is_complete_and_hashed(
    generated_release_dist: Path,
) -> None:
    manifest = _selected_manifest(generated_release_dist)
    profiles = manifest["profiles"]
    assert isinstance(profiles, dict)
    assert profiles

    for profile_id, profile in profiles.items():
        software = profile["software"]
        assert isinstance(software, list), profile_id
        assert software, profile_id
        seen_digests: dict[tuple[str, str], str] = {}
        for index, package in enumerate(software):
            context = f"profiles.{profile_id}.software[{index}]"
            assert isinstance(package["name"], str), context
            assert isinstance(package["version"], str), context
            assert package["version"] != "unversioned", context
            assert isinstance(package["source"], str), context
            assert isinstance(package["architecture"], str), context
            assert isinstance(package["evidence"], str), context
            assert package["evidence"].startswith("/assets/releases/"), context
            assert package["evidence"].endswith("-software-inventory.json"), context
            assert set(package["digest"]) == {"sha256", "blake3"}, context
            _assert_no_hmac(package, context)
            for digest_name, digest_value in package["digest"].items():
                previous = seen_digests.setdefault((digest_name, digest_value), package["name"])
                assert previous == package["name"], (
                    f"{context} shares {digest_name} digest with {previous}"
                )


def test_deterministic_graph_fixture_matches_release_contract() -> None:
    graph = _read_json(FIXTURE_GRAPH)
    offenders = _hmac_paths(graph)
    assert offenders == []

    for channel, manifests in graph["manifests"].items():
        for version, manifest in manifests.items():
            context = f"manifests.{channel}.{version}"
            assert "binaries" not in manifest, context
            assert isinstance(manifest["packages"], list), context
            assert manifest["packages"], context
            for package in manifest["packages"]:
                assert isinstance(package.get("binaries"), list), context
                assert package["binaries"], context
                for binary in package["binaries"]:
                    assert "package" not in binary, context
                    assert isinstance(binary.get("version"), str), context
                    assert binary["version"] != "unversioned", context
                    assert isinstance(binary.get("installed_path"), str), context
                    assert set(binary["digest"]) == {"sha256", "blake3"}, context


def test_deterministic_graph_profile_software_inventory_is_hashed() -> None:
    graph = _read_json(FIXTURE_GRAPH)
    for channel, manifests in graph["manifests"].items():
        for version, manifest in manifests.items():
            for profile_id, profile in manifest["profiles"].items():
                software = profile["software"]
                assert software, f"{channel}.{version}.{profile_id}"
                for index, package in enumerate(software):
                    context = (
                        f"manifests.{channel}.{version}.profiles.{profile_id}"
                        f".software[{index}]"
                    )
                    assert isinstance(package.get("architecture"), str), context
                    assert isinstance(package.get("evidence"), str), context
                    assert set(package["digest"]) == {"sha256", "blake3"}, context
                    _assert_no_hmac(package, context)


def test_release_site_source_does_not_render_fields_missing_from_contract() -> None:
    all_release_site_sources = [
        PROJECT_ROOT / "release-site" / "src" / "pages" / "index.astro",
        PROJECT_ROOT / "release-site" / "src" / "pages" / "channels" / "[id].astro",
        PROJECT_ROOT
        / "release-site"
        / "src"
        / "pages"
        / "channels"
        / "[channel]"
        / "profiles"
        / "[id].astro",
        PROJECT_ROOT / "release-site" / "src" / "lib" / "release-data.ts",
    ]
    forbidden_everywhere = {
        "hmac",
        "HMAC",
        "currentBinary",
        "currentAssets",
        "assetBase",
        "vmObomRows",
    }
    hits: list[str] = []
    for source in all_release_site_sources:
        text = source.read_text(encoding="utf-8")
        for token in sorted(forbidden_everywhere):
            if token in text:
                hits.append(f"{source.relative_to(PROJECT_ROOT)} contains {token}")

    channel_sources = [
        PROJECT_ROOT / "release-site" / "src" / "pages" / "index.astro",
        PROJECT_ROOT / "release-site" / "src" / "pages" / "channels" / "[id].astro",
    ]
    forbidden_on_channel_pages = {
        "Evidence",
        "Host SBOM",
        "VM OBOM",
        "Asset Release History",
        "Current VM Assets",
        "Software Inventory",
    }
    for source in channel_sources:
        text = source.read_text(encoding="utf-8")
        for token in sorted(forbidden_on_channel_pages):
            if token in text:
                hits.append(f"{source.relative_to(PROJECT_ROOT)} contains {token}")
    assert hits == []


@pytest.mark.parametrize(
    ("name", "check"),
    [
        ("channel records use status enum only", lambda dist: _check_channel_status_enum(dist)),
        ("channel records have no hmac", lambda dist: _check_channel_no_hmac(dist)),
        ("manifest record digests are real", lambda dist: _check_manifest_record_digests_real(dist)),
        ("selected manifest has no hmac", lambda dist: _check_selected_manifest_no_hmac(dist)),
        ("manifest has no top-level binaries", lambda dist: _check_no_top_level_binaries(dist)),
        ("manifest packages have urls", lambda dist: _check_packages_have_urls(dist)),
        ("manifest packages have bytes", lambda dist: _check_packages_have_bytes(dist)),
        ("manifest packages have real digests", lambda dist: _check_package_digests_real(dist)),
        ("packages own binary inventory", lambda dist: _check_packages_own_binaries(dist)),
        ("binary digests are real", lambda dist: _check_binary_digests_real(dist)),
        ("binaries do not repeat package field", lambda dist: _check_binaries_do_not_repeat_package(dist)),
        ("binaries have installed paths", lambda dist: _check_binaries_have_installed_paths(dist)),
        ("binaries have sbom refs", lambda dist: _check_binaries_have_sbom_refs(dist)),
        ("profiles have no current binary", lambda dist: _check_profiles_do_not_select_binary(dist)),
        ("profile config list is complete", lambda dist: _check_profile_config_complete(dist)),
        ("profile config digests are real", lambda dist: _check_profile_config_digests_real(dist)),
        ("profile images include kernel initrd rootfs", lambda dist: _check_profile_images_complete(dist)),
        ("profile image digests are real", lambda dist: _check_profile_image_digests_real(dist)),
        ("profile evidence digests are real", lambda dist: _check_profile_evidence_digests_real(dist)),
        ("software inventory is hashed", lambda dist: _check_software_inventory_hashed(dist)),
        ("root page has no profile-owned facts", lambda dist: _check_root_page_ownership(dist)),
        ("channel page has no profile-owned facts", lambda dist: _check_channel_page_ownership(dist)),
        ("profile pages render all config entries", lambda dist: _check_profile_pages_render_config(dist)),
        ("profile pages render all image artifacts", lambda dist: _check_profile_pages_render_images(dist)),
        ("profile pages render software hashes", lambda dist: _check_profile_pages_render_software(dist)),
    ],
)
def test_release_output_theater_regressions_are_caught(
    generated_release_dist: Path,
    name: str,
    check: Any,
) -> None:
    check(generated_release_dist)


def _current_manifest_record(channel: dict[str, Any]) -> dict[str, Any]:
    return next(record for record in channel["manifests"] if record["status"] == "current")


def _selected_manifest(dist: Path) -> dict[str, Any]:
    channels = _read_json(dist / "channels.json")
    record = _current_manifest_record(channels["channels"][CHANNEL])
    return json.loads(_read_bytes(dist, record["url"]))


def _read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def _read_bytes(dist: Path, release_url: str) -> bytes:
    assert release_url.startswith("/"), release_url
    path = dist / release_url.lstrip("/")
    assert path.is_file(), release_url
    return path.read_bytes()


def _assert_profile_shape(profile_id: str, profile: dict[str, Any], context: str) -> None:
    assert profile["id"] == profile_id
    assert FORBIDDEN_PROFILE_FIELDS.isdisjoint(profile), context
    assert isinstance(profile["revision"], str)
    assert isinstance(profile["min_capsem_version"], str)
    assert isinstance(profile["config"], list)
    assert isinstance(profile["images"], list)
    architectures = [image.get("architecture") for image in profile["images"]]
    assert architectures
    assert all(isinstance(architecture, str) for architecture in architectures)
    assert len(architectures) == len(set(architectures))
    _assert_no_hmac(profile, context)


def _profile_artifact_descriptors(profile: dict[str, Any]) -> Iterator[dict[str, Any]]:
    for item in profile["config"]:
        yield item
    for image in profile["images"]:
        for item in image["artifacts"]:
            yield item
        for item in image["evidence"]:
            yield item


def _assert_no_hmac(value: Any, context: str) -> None:
    if isinstance(value, dict):
        assert "hmac" not in value, context
        for key, child in value.items():
            _assert_no_hmac(child, f"{context}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            _assert_no_hmac(child, f"{context}[{index}]")


def _hmac_paths(value: Any, path: str = "$") -> list[str]:
    if isinstance(value, dict):
        hits = [f"{path}.hmac"] if "hmac" in value else []
        for key, child in value.items():
            hits.extend(_hmac_paths(child, f"{path}.{key}"))
        return hits
    if isinstance(value, list):
        hits: list[str] = []
        for index, child in enumerate(value):
            hits.extend(_hmac_paths(child, f"{path}[{index}]"))
        return hits
    return []


def _check_channel_status_enum(dist: Path) -> None:
    channels = _read_json(dist / "channels.json")
    for channel_id, channel in channels["channels"].items():
        records = channel["manifests"]
        assert records, channel_id
        current_count = 0
        for record in records:
            assert record["status"] in ALLOWED_RELEASE_STATUSES, channel_id
            assert set(record) >= {"version", "status", "url", "digest"}, channel_id
            current_count += record["status"] == "current"
        assert current_count == 1, channel_id


def _check_channel_no_hmac(dist: Path) -> None:
    _assert_no_hmac(_read_json(dist / "channels.json"), "channels")


def _check_manifest_record_digests_real(dist: Path) -> None:
    channels = _read_json(dist / "channels.json")
    for channel_id, channel in channels["channels"].items():
        for record in channel["manifests"]:
            _assert_digest_real(record["digest"], f"{channel_id}.{record['version']}")


def _check_selected_manifest_no_hmac(dist: Path) -> None:
    _assert_no_hmac(_selected_manifest(dist), "selected manifest")


def _check_no_top_level_binaries(dist: Path) -> None:
    assert "binaries" not in _selected_manifest(dist)


def _check_packages_have_urls(dist: Path) -> None:
    for package in _selected_manifest(dist)["packages"]:
        assert isinstance(package.get("url"), str) and package["url"], package
        assert package["url"] != "not published", package


def _check_packages_have_bytes(dist: Path) -> None:
    for package in _selected_manifest(dist)["packages"]:
        assert isinstance(package.get("bytes"), int), package
        assert package["bytes"] > 0, package


def _check_package_digests_real(dist: Path) -> None:
    for package in _selected_manifest(dist)["packages"]:
        _assert_digest_real(package["digest"], package["name"])


def _check_packages_own_binaries(dist: Path) -> None:
    packages = _selected_manifest(dist)["packages"]
    assert REQUIRED_PACKAGE_KINDS <= {package.get("kind") for package in packages}
    binary_names = {
        binary.get("name")
        for package in packages
        for binary in package.get("binaries", [])
    }
    assert REQUIRED_BINARY_NAMES <= binary_names
    for package in packages:
        binaries = package.get("binaries")
        assert isinstance(binaries, list), package
        assert binaries, package


def _check_binary_digests_real(dist: Path) -> None:
    for package in _selected_manifest(dist)["packages"]:
        for binary in package["binaries"]:
            _assert_digest_real(binary["digest"], f"{package['name']}:{binary['name']}")


def _check_binaries_do_not_repeat_package(dist: Path) -> None:
    for package in _selected_manifest(dist)["packages"]:
        for binary in package["binaries"]:
            assert "package" not in binary, binary


def _check_binaries_have_installed_paths(dist: Path) -> None:
    for package in _selected_manifest(dist)["packages"]:
        for binary in package["binaries"]:
            assert isinstance(binary.get("installed_path"), str), binary
            assert binary["installed_path"].startswith("/"), binary


def _check_binaries_have_sbom_refs(dist: Path) -> None:
    for package in _selected_manifest(dist)["packages"]:
        for binary in package["binaries"]:
            assert isinstance(binary.get("sbom_component_ref"), str), binary
            assert binary["sbom_component_ref"].startswith("SPDXRef-"), binary


def _check_profiles_do_not_select_binary(dist: Path) -> None:
    for profile_id, profile in _selected_manifest(dist)["profiles"].items():
        assert FORBIDDEN_PROFILE_FIELDS.isdisjoint(profile), profile_id


def _check_profile_config_complete(dist: Path) -> None:
    for profile_id, profile in _selected_manifest(dist)["profiles"].items():
        expected = _expected_profile_config_files(profile_id)
        actual = {Path(item["path"]).name for item in profile["config"]}
        assert expected <= actual, f"{profile_id} missing {sorted(expected - actual)}"


def _check_profile_config_digests_real(dist: Path) -> None:
    for profile_id, profile in _selected_manifest(dist)["profiles"].items():
        for item in profile["config"]:
            _assert_digest_real(item["digest"], f"{profile_id}:{item['path']}")


def _check_profile_images_complete(dist: Path) -> None:
    for profile_id, profile in _selected_manifest(dist)["profiles"].items():
        for arch, image in _profile_images(profile).items():
            kinds = {artifact["kind"] for artifact in image["artifacts"]}
            assert REQUIRED_IMAGE_ARTIFACT_KINDS <= kinds, (
                f"{profile_id}/{arch} missing "
                f"{sorted(REQUIRED_IMAGE_ARTIFACT_KINDS - kinds)}"
            )


def _check_profile_image_digests_real(dist: Path) -> None:
    for profile_id, profile in _selected_manifest(dist)["profiles"].items():
        for arch, image in _profile_images(profile).items():
            for artifact in image["artifacts"]:
                _assert_digest_real(artifact["digest"], f"{profile_id}:{arch}:{artifact['kind']}")


def _check_profile_evidence_digests_real(dist: Path) -> None:
    for profile_id, profile in _selected_manifest(dist)["profiles"].items():
        for arch, image in _profile_images(profile).items():
            evidence = image.get("evidence")
            assert isinstance(evidence, list) and evidence, f"{profile_id}:{arch}"
            for item in evidence:
                _assert_digest_real(item["digest"], f"{profile_id}:{arch}:{item['kind']}")


def _check_software_inventory_hashed(dist: Path) -> None:
    for profile_id, profile in _selected_manifest(dist)["profiles"].items():
        software = profile["software"]
        assert software, profile_id
        seen_digests: dict[tuple[str, str], str] = {}
        for item in software:
            assert isinstance(item.get("architecture"), str), item
            assert isinstance(item.get("evidence"), str), item
            assert isinstance(item.get("version"), str), item
            assert item["version"] != "unversioned", item
            _assert_digest_real(item["digest"], f"{profile_id}:{item['name']}")
            for digest_name, digest_value in item["digest"].items():
                previous = seen_digests.setdefault((digest_name, digest_value), item["name"])
                assert previous == item["name"], (
                    f"{profile_id}:{item['name']} shares {digest_name} digest with {previous}"
                )


def _check_root_page_ownership(dist: Path) -> None:
    page = (dist / "index.html").read_text(encoding="utf-8")
    _assert_page_excludes(page, _profile_owned_page_tokens(), "root")


def _check_channel_page_ownership(dist: Path) -> None:
    page = (dist / "channels" / CHANNEL / "index.html").read_text(encoding="utf-8")
    _assert_page_excludes(page, _profile_owned_page_tokens(), "channel")


def _check_profile_pages_render_config(dist: Path) -> None:
    for profile_id, profile in _selected_manifest(dist)["profiles"].items():
        page = _channel_profile_page(dist, profile_id)
        for item in profile["config"]:
            assert item["url"] in page, f"{profile_id}:{item['url']}"
            assert item["digest"]["sha256"] in page, f"{profile_id}:{item['url']}"
            assert item["digest"]["blake3"] in page, f"{profile_id}:{item['url']}"


def _check_profile_pages_render_images(dist: Path) -> None:
    for profile_id, profile in _selected_manifest(dist)["profiles"].items():
        page = _channel_profile_page(dist, profile_id)
        for image in _profile_images(profile).values():
            for artifact in image["artifacts"]:
                assert artifact["url"] in page, f"{profile_id}:{artifact['url']}"
                assert artifact["digest"]["sha256"] in page, f"{profile_id}:{artifact['url']}"
                assert artifact["digest"]["blake3"] in page, f"{profile_id}:{artifact['url']}"


def _check_profile_pages_render_software(dist: Path) -> None:
    for profile_id, profile in _selected_manifest(dist)["profiles"].items():
        page = _channel_profile_page(dist, profile_id)
        for item in profile["software"]:
            assert item["name"] in page, f"{profile_id}:{item}"
            assert item["version"] in page, f"{profile_id}:{item}"
            assert item["digest"]["sha256"] in page, f"{profile_id}:{item}"
            assert item["digest"]["blake3"] in page, f"{profile_id}:{item}"


def _assert_digest_real(digest: dict[str, Any], context: str) -> None:
    assert set(digest) == {"sha256", "blake3"}, context
    for name in ("sha256", "blake3"):
        value = digest[name]
        assert isinstance(value, str), context
        assert len(value) == 64, context
        int(value, 16)
        assert len(set(value)) > 1, f"{context} {name} is a placeholder"


def _expected_profile_config_files(profile_id: str) -> set[str]:
    profile_dir = PROFILE_CONFIG_ROOT / profile_id
    profile_toml = tomllib.loads((profile_dir / "profile.toml").read_text(encoding="utf-8"))
    declared = {
        Path(value).name
        for value in profile_toml.get("files", {}).values()
        if isinstance(value, str)
    }
    rule_files = {
        Path(value).name
        for value in profile_toml.get("rules", {}).get("files", [])
        if isinstance(value, str)
    }
    existing_required = {name for name in REQUIRED_PROFILE_CONFIG_FILES if (profile_dir / name).is_file()}
    return existing_required | declared | rule_files


def _profile_images(profile: dict[str, Any]) -> dict[str, dict[str, Any]]:
    images = profile["images"]
    assert isinstance(images, list), profile["id"]
    return {image["architecture"]: image for image in images}


def _channel_profile_page(dist: Path, profile_id: str) -> str:
    return (
        dist
        / "channels"
        / CHANNEL
        / "profiles"
        / profile_id
        / "index.html"
    ).read_text(encoding="utf-8")


def _profile_owned_page_tokens() -> set[str]:
    return {
        "Software Inventory",
        "Config Files",
        "Profile Images",
        "Profile Evidence",
        "Host SBOM",
        "VM OBOM",
        "Asset Release History",
        "Current VM Assets",
    }


def _assert_page_excludes(page: str, tokens: set[str], context: str) -> None:
    hits = sorted(token for token in tokens if token in page)
    assert hits == [], f"{context} page leaks {hits}"
