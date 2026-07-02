"""Release output contract tests.

These tests intentionally assert the documented public graph shape. They are
expected to fail while the generator still emits legacy asset-channel output.
"""

from __future__ import annotations

import hashlib
import importlib.util
import json
import sys
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
FORBIDDEN_PROFILE_FIELDS = {
    "current_binary",
    "current_assets",
    "asset_version",
    "binary_version",
}

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

    assert manifest_url.startswith(f"/manifests/{CHANNEL}/")
    assert manifest_url.endswith("/manifest.json")
    assert manifest_url != f"/assets/{CHANNEL}/manifest.json"
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
            assert isinstance(binary["installed_path"], str)
            assert isinstance(binary["bytes"], int)
            assert set(binary["digest"]) == {"sha256", "blake3"}
            assert isinstance(binary["sbom_component_ref"], str)
            _assert_no_hmac(binary, binary_context)


def test_profile_catalog_matches_manifest_profiles_and_hashes(
    generated_release_dist: Path,
) -> None:
    channels = _read_json(generated_release_dist / "channels.json")
    channel = channels["channels"][CHANNEL]
    profile_catalog = channel["profile_catalog"]
    catalog_url = profile_catalog["source"]
    catalog_bytes = _read_bytes(generated_release_dist, catalog_url)
    catalog = json.loads(catalog_bytes)
    manifest = _selected_manifest(generated_release_dist)

    assert profile_catalog["hash"] == blake3.blake3(catalog_bytes).hexdigest()
    assert catalog["schema"] == "capsem.profile_catalog.v1"

    manifest_profiles = manifest["profiles"]
    assert isinstance(manifest_profiles, dict)
    assert manifest_profiles
    catalog_profiles = {profile["id"]: profile for profile in catalog["profiles"]}
    assert set(catalog_profiles) == set(manifest_profiles)

    for profile_id, manifest_profile in manifest_profiles.items():
        catalog_profile = catalog_profiles[profile_id]
        assert catalog_profile["revision"] == manifest_profile["revision"]
        _assert_profile_shape(profile_id, manifest_profile, f"manifest.profiles.{profile_id}")
        _assert_profile_shape(profile_id, catalog_profile, f"catalog.profiles.{profile_id}")


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
        assert "Current VM Assets" not in page, page_name
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
                    assert isinstance(binary.get("installed_path"), str), context
                    assert set(binary["digest"]) == {"sha256", "blake3"}, context


def test_release_site_source_does_not_render_fields_missing_from_contract() -> None:
    release_site_sources = [
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
    forbidden_tokens = {
        "hmac",
        "HMAC",
        "Current VM Assets",
        "currentBinary",
        "currentAssets",
        "assetBase",
        "vmObomRows",
    }
    hits: list[str] = []
    for source in release_site_sources:
        text = source.read_text(encoding="utf-8")
        for token in sorted(forbidden_tokens):
            if token in text:
                hits.append(f"{source.relative_to(PROJECT_ROOT)} contains {token}")
    assert hits == []


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
    assert isinstance(profile["images"], dict)
    _assert_no_hmac(profile, context)


def _profile_artifact_descriptors(profile: dict[str, Any]) -> Iterator[dict[str, Any]]:
    for item in profile["config"]:
        yield item
    for image in profile["images"].values():
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
