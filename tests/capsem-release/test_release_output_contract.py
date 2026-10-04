"""Release output contract tests.

These tests assert the documented public graph shape: packages own the binary
inventory, and one runtime document owns the VM images per architecture.
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
from capsem_builder.release.tools import check_remote_release_readiness as READINESS

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CHANNEL = "stable"
FIXTURE_GRAPH = (
    PROJECT_ROOT / "tests" / "capsem-release" / "fixtures" / "release-graph-stable-nightly.json"
)
RELEASE_SITE_PAGES = PROJECT_ROOT / "build_system" / "release_site" / "src" / "pages"
RUNTIME_FIELDS = {
    "revision",
    "source_commit",
    "status",
    "min_capsem_version",
    "max_capsem_version",
    "architectures",
}
RUNTIME_ARCHITECTURE_FIELDS = {
    "architecture",
    "package_inventory_revision",
    "image_revision",
    "software",
    "images",
    "evidence",
}
FORBIDDEN_PAGE_FIELDS = {"current_binary", "current_assets", "asset_version", "binary_version"}
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
    assert {package.get("kind") for package in packages} >= REQUIRED_PACKAGE_KINDS

    binary_names = {
        binary.get("name") for package in packages for binary in package.get("binaries", [])
    }
    assert binary_names >= REQUIRED_BINARY_NAMES

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


def test_manifest_runtime_is_the_runtime_contract(
    generated_release_dist: Path,
) -> None:
    manifest = _selected_manifest(generated_release_dist)

    assert set(manifest) == {"version", "channel", "status", "packages", "runtime"}
    _assert_runtime_shape(manifest["runtime"], "manifest.runtime")


def test_generated_release_publishes_no_catalog_or_config(
    generated_release_dist: Path,
) -> None:
    """The runtime is images and evidence; no catalog or config is public."""
    catalog_files = [
        path.relative_to(generated_release_dist).as_posix()
        for path in generated_release_dist.rglob("catalog.json")
    ]
    assert catalog_files == []

    hits: list[str] = []
    for path in (
        generated_release_dist / "channels.json",
        generated_release_dist / "health.json",
        generated_release_dist / "assets" / CHANNEL / "manifest.json",
        generated_release_dist / "index.html",
        generated_release_dist / "channels" / CHANNEL / "index.html",
        _runtime_page_path(generated_release_dist),
    ):
        text = path.read_text(encoding="utf-8")
        for token in ("catalog.json", '"profiles"', "/profiles/", '"config"'):
            if token in text:
                hits.append(f"{path.relative_to(generated_release_dist)} contains {token}")
    assert hits == []


def test_release_readiness_checker_publishes_no_catalog() -> None:
    checker = Path(READINESS.__file__).read_text(encoding="utf-8")
    assert "catalog.json" not in checker


def test_runtime_artifact_digests_match_files(
    generated_release_dist: Path,
) -> None:
    runtime = _selected_manifest(generated_release_dist)["runtime"]

    for item in _runtime_artifact_descriptors(runtime):
        url = item["url"]
        payload = _read_bytes(generated_release_dist, url)
        assert item["bytes"] == len(payload), f"{url} bytes"
        assert item["digest"] == {
            "sha256": hashlib.sha256(payload).hexdigest(),
            "blake3": blake3.blake3(payload).hexdigest(),
        }, f"{url} digest"


def test_pages_only_render_owned_release_facts(generated_release_dist: Path) -> None:
    runtime = _selected_manifest(generated_release_dist)["runtime"]
    root_page = (generated_release_dist / "index.html").read_text(encoding="utf-8")
    channel_page = (generated_release_dist / "channels" / CHANNEL / "index.html").read_text(
        encoding="utf-8"
    )

    for page_name, page in (("root", root_page), ("channel", channel_page)):
        assert "HMAC" not in page, page_name
        assert "hmac" not in page, page_name
        assert "Evidence" not in page, page_name
        assert "Host SBOM" not in page, page_name
        assert "VM OBOM" not in page, page_name
        assert "Asset Release History" not in page, page_name
        assert "Current VM Assets" not in page, page_name
        assert "Software Inventory" not in page, page_name
        assert "current_binary" not in page, page_name
        assert "current_assets" not in page, page_name

    runtime_page = _runtime_page(generated_release_dist)
    assert "HMAC" not in runtime_page
    assert "hmac" not in runtime_page
    assert "Capsem Binaries" not in runtime_page
    assert "Current VM Assets" not in runtime_page
    for field in FORBIDDEN_PAGE_FIELDS:
        assert field not in runtime_page
    for item in _runtime_artifact_descriptors(runtime):
        assert item["url"] in runtime_page
        assert _hash_label(item["digest"]["sha256"]) in runtime_page
        assert _hash_label(item["digest"]["blake3"]) in runtime_page


def test_runtime_software_inventory_is_complete_and_hashed(
    generated_release_dist: Path,
) -> None:
    runtime = _selected_manifest(generated_release_dist)["runtime"]
    revision = runtime["revision"]

    seen_digests: dict[tuple[str, str], str] = {}
    for arch, architecture in _runtime_architectures(runtime).items():
        software = architecture["software"]
        assert software, arch
        for index, package in enumerate(software):
            context = f"runtime.architectures.{arch}.software[{index}]"
            assert isinstance(package["name"], str), context
            assert isinstance(package["version"], str), context
            assert package["version"] != "unversioned", context
            assert isinstance(package["source"], str), context
            assert package["architecture"] == arch, context
            assert package["evidence"] == (
                f"/runtime/releases/{CHANNEL}/{revision}/{arch}/software-inventory.json"
            ), context
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
            assert "profiles" not in manifest, context
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


def test_deterministic_graph_runtime_software_inventory_is_hashed() -> None:
    graph = _read_json(FIXTURE_GRAPH)
    for channel, manifests in graph["manifests"].items():
        for version, manifest in manifests.items():
            runtime = manifest["runtime"]
            _assert_runtime_shape(runtime, f"manifests.{channel}.{version}.runtime")
            for arch, architecture in _runtime_architectures(runtime).items():
                software = architecture["software"]
                assert software, f"{channel}.{version}.{arch}"
                for index, package in enumerate(software):
                    context = f"manifests.{channel}.{version}.runtime.architectures.{arch}.software[{index}]"
                    assert package.get("architecture") == arch, context
                    assert isinstance(package.get("evidence"), str), context
                    assert set(package["digest"]) == {"sha256", "blake3"}, context
                    _assert_no_hmac(package, context)


def test_release_site_source_does_not_render_fields_missing_from_contract() -> None:
    all_release_site_sources = [
        RELEASE_SITE_PAGES / "index.astro",
        RELEASE_SITE_PAGES / "channels" / "[id].astro",
        RELEASE_SITE_PAGES / "channels" / "[channel]" / "runtime.astro",
        PROJECT_ROOT / "build_system" / "release_site" / "src" / "lib" / "release-data.ts",
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

    forbidden_on_channel_pages = {
        "Evidence",
        "Host SBOM",
        "VM OBOM",
        "Asset Release History",
        "Current VM Assets",
        "Software Inventory",
    }
    for source in (
        RELEASE_SITE_PAGES / "index.astro",
        RELEASE_SITE_PAGES / "channels" / "[id].astro",
    ):
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
        (
            "manifest record digests are real",
            lambda dist: _check_manifest_record_digests_real(dist),
        ),
        ("selected manifest has no hmac", lambda dist: _check_selected_manifest_no_hmac(dist)),
        ("manifest has no top-level binaries", lambda dist: _check_no_top_level_binaries(dist)),
        ("manifest packages have urls", lambda dist: _check_packages_have_urls(dist)),
        ("manifest packages have bytes", lambda dist: _check_packages_have_bytes(dist)),
        ("manifest packages have real digests", lambda dist: _check_package_digests_real(dist)),
        ("packages own binary inventory", lambda dist: _check_packages_own_binaries(dist)),
        ("binary digests are real", lambda dist: _check_binary_digests_real(dist)),
        (
            "binaries do not repeat package field",
            lambda dist: _check_binaries_do_not_repeat_package(dist),
        ),
        ("binaries have installed paths", lambda dist: _check_binaries_have_installed_paths(dist)),
        ("binaries have sbom refs", lambda dist: _check_binaries_have_sbom_refs(dist)),
        ("runtime selects no binary", lambda dist: _check_runtime_does_not_select_binary(dist)),
        (
            "runtime images include kernel initrd rootfs",
            lambda dist: _check_runtime_images_complete(dist),
        ),
        ("runtime image digests are real", lambda dist: _check_runtime_image_digests_real(dist)),
        (
            "runtime evidence digests are real",
            lambda dist: _check_runtime_evidence_digests_real(dist),
        ),
        ("software inventory is hashed", lambda dist: _check_software_inventory_hashed(dist)),
        ("root page has no runtime-owned facts", lambda dist: _check_root_page_ownership(dist)),
        (
            "channel page has no runtime-owned facts",
            lambda dist: _check_channel_page_ownership(dist),
        ),
        (
            "runtime page renders all image artifacts",
            lambda dist: _check_runtime_page_renders_images(dist),
        ),
        (
            "runtime page renders software hashes",
            lambda dist: _check_runtime_page_renders_software(dist),
        ),
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


def _assert_runtime_shape(runtime: dict[str, Any], context: str) -> None:
    assert isinstance(runtime, dict), context
    assert set(runtime) <= RUNTIME_FIELDS, (
        f"{context} carries {sorted(set(runtime) - RUNTIME_FIELDS)}"
    )
    assert isinstance(runtime["revision"], str), context
    assert runtime["status"] in ALLOWED_RELEASE_STATUSES, context
    assert isinstance(runtime["min_capsem_version"], str), context
    for arch, architecture in _runtime_architectures(runtime).items():
        assert set(architecture) == RUNTIME_ARCHITECTURE_FIELDS, f"{context}.{arch}"
        assert architecture["image_revision"] == runtime["revision"], f"{context}.{arch}"
        assert architecture["package_inventory_revision"] == runtime["revision"], (
            f"{context}.{arch}"
        )
    _assert_no_hmac(runtime, context)


def _runtime_artifact_descriptors(runtime: dict[str, Any]) -> Iterator[dict[str, Any]]:
    for architecture in _runtime_architectures(runtime).values():
        yield from architecture["images"]
        yield from architecture["evidence"]


def _runtime_architectures(runtime: dict[str, Any]) -> dict[str, dict[str, Any]]:
    architectures = runtime["architectures"]
    assert isinstance(architectures, list)
    assert architectures
    by_arch = {architecture["architecture"]: architecture for architecture in architectures}
    assert len(by_arch) == len(architectures)
    for arch, architecture in by_arch.items():
        assert isinstance(arch, str)
        assert isinstance(architecture["software"], list), arch
        assert isinstance(architecture["images"], list), arch
        assert isinstance(architecture["evidence"], list), arch
    return by_arch


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
    assert {package.get("kind") for package in packages} >= REQUIRED_PACKAGE_KINDS
    binary_names = {
        binary.get("name") for package in packages for binary in package.get("binaries", [])
    }
    assert binary_names >= REQUIRED_BINARY_NAMES
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


def _check_runtime_does_not_select_binary(dist: Path) -> None:
    runtime = _selected_manifest(dist)["runtime"]
    assert FORBIDDEN_PAGE_FIELDS.isdisjoint(runtime)
    for architecture in _runtime_architectures(runtime).values():
        assert FORBIDDEN_PAGE_FIELDS.isdisjoint(architecture)


def _check_runtime_images_complete(dist: Path) -> None:
    runtime = _selected_manifest(dist)["runtime"]
    for arch, architecture in _runtime_architectures(runtime).items():
        kinds = {artifact["kind"] for artifact in architecture["images"]}
        assert kinds >= REQUIRED_IMAGE_ARTIFACT_KINDS, (
            f"{arch} missing {sorted(REQUIRED_IMAGE_ARTIFACT_KINDS - kinds)}"
        )


def _check_runtime_image_digests_real(dist: Path) -> None:
    runtime = _selected_manifest(dist)["runtime"]
    for arch, architecture in _runtime_architectures(runtime).items():
        for artifact in architecture["images"]:
            assert artifact["url"].startswith(
                f"/runtime/releases/{CHANNEL}/{runtime['revision']}/{arch}/"
            )
            _assert_digest_real(artifact["digest"], f"{arch}:{artifact['kind']}")


def _check_runtime_evidence_digests_real(dist: Path) -> None:
    runtime = _selected_manifest(dist)["runtime"]
    for arch, architecture in _runtime_architectures(runtime).items():
        evidence = architecture["evidence"]
        assert evidence, arch
        for item in evidence:
            assert item["url"].startswith(
                f"/runtime/releases/{CHANNEL}/{runtime['revision']}/{arch}/"
            )
            _assert_digest_real(item["digest"], f"{arch}:{item['kind']}")


def _check_software_inventory_hashed(dist: Path) -> None:
    runtime = _selected_manifest(dist)["runtime"]
    seen_digests: dict[tuple[str, str], str] = {}
    for arch, architecture in _runtime_architectures(runtime).items():
        software = architecture["software"]
        assert software, arch
        for item in software:
            assert item.get("architecture") == arch, item
            assert isinstance(item.get("evidence"), str), item
            assert isinstance(item.get("version"), str), item
            assert item["version"] != "unversioned", item
            _assert_digest_real(item["digest"], f"{arch}:{item['name']}")
            for digest_name, digest_value in item["digest"].items():
                previous = seen_digests.setdefault((digest_name, digest_value), item["name"])
                assert previous == item["name"], (
                    f"{item['name']} shares {digest_name} digest with {previous}"
                )


def _check_root_page_ownership(dist: Path) -> None:
    page = (dist / "index.html").read_text(encoding="utf-8")
    _assert_page_excludes(page, _runtime_owned_page_tokens(), "root")


def _check_channel_page_ownership(dist: Path) -> None:
    page = (dist / "channels" / CHANNEL / "index.html").read_text(encoding="utf-8")
    _assert_page_excludes(page, _runtime_owned_page_tokens(), "channel")


def _check_runtime_page_renders_images(dist: Path) -> None:
    runtime = _selected_manifest(dist)["runtime"]
    page = _runtime_page(dist)
    for architecture in _runtime_architectures(runtime).values():
        for artifact in architecture["images"]:
            assert artifact["url"] in page, artifact["url"]
            assert _hash_label(artifact["digest"]["sha256"]) in page, artifact["url"]
            assert _hash_label(artifact["digest"]["blake3"]) in page, artifact["url"]


def _check_runtime_page_renders_software(dist: Path) -> None:
    runtime = _selected_manifest(dist)["runtime"]
    page = _runtime_page(dist)
    for architecture in _runtime_architectures(runtime).values():
        for item in architecture["software"]:
            assert item["name"] in page, item
            assert item["version"] in page, item
            assert _hash_label(item["digest"]["sha256"]) in page, item
            assert _hash_label(item["digest"]["blake3"]) in page, item


def _assert_digest_real(digest: dict[str, Any], context: str) -> None:
    assert set(digest) == {"sha256", "blake3"}, context
    for name in ("sha256", "blake3"):
        value = digest[name]
        assert isinstance(value, str), context
        assert len(value) == 64, context
        int(value, 16)
        assert len(set(value)) > 1, f"{context} {name} is a placeholder"


def _hash_label(value: str) -> str:
    return f"{value[:8]}..." if len(value) > 12 else value


def _runtime_page_path(dist: Path) -> Path:
    return dist / "channels" / CHANNEL / "runtime" / "index.html"


def _runtime_page(dist: Path) -> str:
    return _runtime_page_path(dist).read_text(encoding="utf-8")


def _runtime_owned_page_tokens() -> set[str]:
    return {
        "Installed Software",
        "Runtime Images",
        "Runtime Evidence",
        "Host SBOM",
        "VM OBOM",
        "Asset Release History",
        "Current VM Assets",
    }


def _assert_page_excludes(page: str, tokens: set[str], context: str) -> None:
    hits = sorted(token for token in tokens if token in page)
    assert hits == [], f"{context} page leaks {hits}"
