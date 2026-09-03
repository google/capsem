"""Release SBOM/OBOM/build-ledger contract tests."""

from __future__ import annotations

import ast
from pathlib import Path

from capsem_builder.image.config import load_guest_config

PROJECT_ROOT = Path(__file__).resolve().parents[2]


def _read(path: str) -> str:
    return (PROJECT_ROOT / path).read_text(encoding="utf-8")


def test_release_workflows_generate_binary_sbom_and_asset_obom() -> None:
    binary_workflow = _read(".github/workflows/release.yaml")
    asset_workflow = _read(".github/workflows/release-assets.yaml")
    channel_workflow = _read(".github/workflows/release-channel.yaml")

    assert "npm install -g @cyclonedx/cdxgen" not in asset_workflow
    assert "attestations: write" in asset_workflow
    assert "id-token: write" in asset_workflow
    assert "CAPSEM_CDXGEN_CMD" not in asset_workflow
    assert "asset-channel-preview" in asset_workflow
    assert "Publish immutable GitHub profile release" in asset_workflow
    assert "Attest VM asset provenance" in asset_workflow
    assert "actions/attest-build-provenance@" in asset_workflow
    publish_profile = asset_workflow.split("  publish-profile-release:", maxsplit=1)[1].split(
        "  deploy-channel:", maxsplit=1
    )[0]
    assert "needs.author-profile-release.outputs.release_needed == 'true'" in publish_profile
    assert "needs.test-profile-pairing.result == 'success'" in publish_profile
    assert "if: ${{ inputs.dry_run == false }}" in publish_profile
    assert "build_system/scripts/release/stage-profile-publication.py" in asset_workflow
    assert "build_system/scripts/release/verify-profile-publication.py" in asset_workflow
    assert "subject-path: cache/target/asset-release/profile-*/*" in asset_workflow
    assert publish_profile.index("Attest VM asset provenance") < publish_profile.index(
        "Publish immutable GitHub profile release"
    )
    assert (
        'for key in ("vm_oboms", "host_sboms", "host_binary_files", "attestations")'
        in channel_workflow
    )

    assert "Generate packaged host SBOM" in binary_workflow
    assert "build_system/scripts/release/generate-host-binary-sbom.py" in binary_workflow
    assert "--output release-artifacts/capsem-sbom.spdx.json" in binary_workflow
    assert "cargo sbom --output-format spdx_json_2_3" not in binary_workflow
    assert "install_cargo_tool cargo-sbom" not in binary_workflow
    author_sbom = binary_workflow.split("  author-binary-candidate:", maxsplit=1)[1].split(
        "  test-binary-pairing:", maxsplit=1
    )[0]
    assert "Generate packaged host SBOM once" in author_sbom
    assert "build_system/scripts/release/generate-host-binary-sbom.py" in author_sbom
    assembly = binary_workflow.split("  assemble-release-channel:", maxsplit=1)[1].split(
        "  verify-release-candidate:", maxsplit=1
    )[0]
    assert "Generate packaged host SBOM" not in assembly
    assert "name: binary-channel-candidate" in assembly
    create_release = binary_workflow.split("  create-release:", maxsplit=1)[1].split(
        "  assemble-release-channel:", maxsplit=1
    )[0]
    assert "name: binary-host-sbom" in create_release
    assert "Generate packaged host SBOM" not in create_release
    assert "Attest SBOM" in binary_workflow
    sbom_attestation = binary_workflow.split("- name: Attest SBOM", maxsplit=1)[1].split(
        "- name: Build summary", maxsplit=1
    )[0]
    assert "release-artifacts/*.pkg" in sbom_attestation
    assert "release-artifacts/*.deb" in sbom_attestation
    assert "predicate-type: https://spdx.dev/Document/v2.3" in binary_workflow
    assert "predicate-path: release-artifacts/capsem-sbom.spdx.json" in binary_workflow

    assert "build-assets:" not in binary_workflow
    assert "obom.cdx.json (arm64)" not in binary_workflow
    assert "vm-build-ledger-" not in binary_workflow


def test_builder_emits_obom_and_keeps_build_ledger_debug_scoped() -> None:
    builder = _read("build_system/builder/image/docker.py")
    evidence = _read("build_system/builder/release/obom.py")
    syntax = ast.parse(builder)
    cdxgen_calls: list[ast.Call] = []
    for node in ast.walk(syntax):
        if (
            not isinstance(node, ast.Call)
            or not isinstance(node.func, ast.Name)
            or node.func.id != "run_cmd"
            or not node.args
            or not isinstance(node.args[0], ast.List)
        ):
            continue
        command = node.args[0]
        if not any(
            isinstance(child, ast.Call)
            and isinstance(child.func, ast.Name)
            and child.func.id == "_cdxgen_command"
            for child in ast.walk(command)
        ):
            continue
        cdxgen_calls.append(node)

    assert 'OBOM_ASSET = "obom.cdx.json"' in builder
    assert 'BUILD_LEDGER_NAME = "build-ledger.log"' in builder
    assert len(cdxgen_calls) == 1
    cdxgen_call = cdxgen_calls[0]
    cdxgen_command = [
        child.value
        for child in ast.walk(cdxgen_call.args[0])
        if isinstance(child, ast.Constant) and isinstance(child.value, str)
    ]
    assert {"--pull", "never", "--network", "/rootfs", "-t", "rootfs", "-o"} <= set(cdxgen_command)
    assert any(
        isinstance(child, ast.Call)
        and isinstance(child.func, ast.Name)
        and child.func.id == "_scanner_output_command"
        for child in ast.walk(cdxgen_call.args[0])
    )
    keywords = {keyword.arg: keyword.value for keyword in cdxgen_call.keywords}
    assert isinstance(keywords["capture"], ast.Constant) and keywords["capture"].value is True
    assert isinstance(keywords["timeout"], ast.Name)
    assert keywords["timeout"].id == "OBOM_COMMAND_TIMEOUT_SECONDS"
    assert "def _normalize_cyclonedx_obom" in builder
    assert "def _cdx_validate_command" in builder
    assert '"capsem:evidence:scope", "exported-rootfs"' in evidence
    assert 'prop.get("name") == "cdx:osquery:category"' in evidence
    assert "def generate_cyclonedx_obom" in builder
    assert "cdxgen" in builder
    assert "CAPSEM_CDXGEN_CMD" not in builder
    assert "The build ledger records declared build inputs" in builder
    assert "This OBOM is the runtime" in builder
    assert '"capsem.build_ledger.v1"' in builder


def test_cdxgen_is_digest_pinned_in_the_one_asset_helper() -> None:
    config = load_guest_config(PROJECT_ROOT / "config/docker/image")
    host_builder = _read("build_system/docker/Dockerfile.host-builder")
    asset_workflow = _read(".github/workflows/release-assets.yaml")
    helper = _read("build_system/docker/Dockerfile.asset-tools")

    assert set(config.build.asset_tools.architectures) == set(config.build.architectures)
    for downloads in config.build.asset_tools.architectures.values():
        for binary in (downloads.cdxgen, downloads.cdx_validate):
            assert "/releases/download/v12.7.0/" in binary.url
            assert len(binary.sha256) == 64
    assert "sha256sum -c -" in helper
    assert "@cyclonedx/cdxgen" not in host_builder
    assert "@cyclonedx/cdxgen" not in asset_workflow
    assert "CAPSEM_CDXGEN_CMD" not in asset_workflow


def test_admin_materialization_and_service_routes_expose_verified_obom_evidence() -> None:
    admin = _read("crates/capsem-admin/src/main.rs")
    profile_images = _read("crates/capsem-admin/src/profile_images.rs")
    service_router = _read("crates/capsem-service/src/router_runtime.rs")
    profile_routes = _read("crates/capsem-service/src/profile_routes.rs")
    obom_routes = _read("crates/capsem-service/src/profile_routes/obom.rs")
    api = _read("crates/capsem-service/src/api.rs")

    assert "materialize_profile_obom_descriptor" in profile_images
    assert 'manifest_assets.get("obom.cdx.json")' in profile_images
    assert (
        "check_local_asset(assets_dir, arch, logical_name, hash, size)"
        in profile_images
    )
    assert "read_obom_generator" in profile_images
    assert "ProfileMaterializedObomReport" in admin
    assert 'scope: "base_image"' in profile_images
    assert (
        "source profile {location} must not contain generated obom pins"
        in profile_images
    )

    assert (
        'route("/profiles/{profile_id}/obom", get(handle_profile_obom))'
        in service_router
    )
    assert "mod obom;" in profile_routes
    assert "fn profile_obom_info" in obom_routes
    assert "read_local_profile_obom" in obom_routes
    assert "profile OBOM hash mismatch" in obom_routes
    assert "profile OBOM size mismatch" in obom_routes
    assert "rootfs_hash" in api
    assert "generator_version" in api


def test_docs_describe_scope_without_claiming_user_runtime_inventory() -> None:
    build_verification = _read("web/docs/src/content/docs/security/build-verification.md")
    build_system = _read("web/docs/src/content/docs/architecture/build-system.md")
    service_api = _read("web/docs/src/content/docs/architecture/service-api.md")

    assert "Host binaries publish a Software Bill of Materials" in build_verification
    assert "VM base images publish an Operations Bill of Materials" in build_verification
    assert "Base Linux VM image only" in build_verification
    assert "User session mutations, workspace writes, and post-boot state" in build_verification
    assert "component names and versions come from the OBOM" in build_verification

    assert "`obom.cdx.json`" in build_system
    assert "installed base-image package/component truth" in build_system
    assert "post-boot state" in build_system
    assert "debug evidence" in build_system

    assert "`/profiles/{profile_id}/obom`" in service_api
