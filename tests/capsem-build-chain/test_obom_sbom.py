"""Release SBOM/OBOM/build-ledger contract tests."""

from __future__ import annotations

from pathlib import Path
import re


PROJECT_ROOT = Path(__file__).resolve().parents[2]
CDXGEN_VERSION = "12.7.0"


def _read(path: str) -> str:
    return (PROJECT_ROOT / path).read_text(encoding="utf-8")


def test_release_workflows_generate_binary_sbom_and_asset_obom() -> None:
    binary_workflow = _read(".github/workflows/release.yaml")
    asset_workflow = _read(".github/workflows/release-assets.yaml")
    channel_workflow = _read(".github/workflows/release-channel.yaml")

    assert f"npm install -g @cyclonedx/cdxgen@{CDXGEN_VERSION}" in asset_workflow
    assert "@cyclonedx/cdxgen@latest" not in asset_workflow
    assert "attestations: write" in asset_workflow
    assert "id-token: write" in asset_workflow
    assert "CAPSEM_CDXGEN_CMD: cdxgen" in asset_workflow
    assert asset_workflow.index("Install OBOM generator") < asset_workflow.index(
        "- name: Build VM assets"
    )
    assert asset_workflow.index("CAPSEM_CDXGEN_CMD: cdxgen") < asset_workflow.index(
        "just _build-rootfs"
    )
    assert "asset-channel-preview" in asset_workflow
    assert "Publish immutable GitHub asset release" in asset_workflow
    assert "Attest VM asset provenance" in asset_workflow
    assert "actions/attest-build-provenance@" in asset_workflow
    assert (
        "if: ${{ inputs.dry_run == false && steps.asset-delta.outputs.asset_blobs_changed == 'true' }}"
        in asset_workflow
    )
    assert "target/asset-release/assets-v*/*-vmlinuz" in asset_workflow
    assert "target/asset-release/assets-v*/*-initrd.img" in asset_workflow
    assert "target/asset-release/assets-v*/*-rootfs.erofs" in asset_workflow
    assert "target/asset-release/assets-v*/*-obom.cdx.json" in asset_workflow
    assert asset_workflow.index("Publish immutable GitHub asset release") < asset_workflow.index(
        "Attest VM asset provenance"
    )
    assert (
        'for key in ("vm_oboms", "host_sboms", "host_binary_files", "attestations")'
        in channel_workflow
    )

    assert "Generate packaged host SBOM" in binary_workflow
    assert "scripts/generate-host-binary-sbom.py" in binary_workflow
    assert "--output release-artifacts/capsem-sbom.spdx.json" in binary_workflow
    assert "cargo sbom --output-format spdx_json_2_3" not in binary_workflow
    assert "install_cargo_tool cargo-sbom" not in binary_workflow
    channel_sbom = binary_workflow.split(
        "  assemble-release-channel:", maxsplit=1
    )[1].split("- name: Verify binary channel artifacts", maxsplit=1)[0]
    assert "Generate packaged host SBOM" in channel_sbom
    assert "scripts/generate-host-binary-sbom.py" in channel_sbom
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
    builder = _read("src/capsem/builder/docker.py")

    assert 'OBOM_ASSET = "obom.cdx.json"' in builder
    assert 'BUILD_LEDGER_NAME = "build-ledger.log"' in builder
    assert f'CDXGEN_VERSION = "{CDXGEN_VERSION}"' in builder
    assert '"-t",\n            "rootfs"' in builder
    assert '"-t",\n            "os"' not in builder
    assert '"--no-validate"' in builder
    assert "def _normalize_cyclonedx_obom" in builder
    assert "def _cdx_validate_command" in builder
    assert '"capsem:evidence:scope", "value": "exported-rootfs"' in builder
    assert 'prop.get("name") == "cdx:osquery:category"' in builder
    assert 'run_cmd([\n            *_cdxgen_command(),' in builder
    assert '], capture=True)' in builder
    assert "def generate_cyclonedx_obom" in builder
    assert "cdxgen" in builder
    assert "CAPSEM_CDXGEN_CMD" in builder
    assert "The build ledger records declared build inputs" in builder
    assert "This OBOM is the runtime" in builder
    assert '"capsem.build_ledger.v1"' in builder


def test_cdxgen_is_pinned_identically_across_local_and_ci_asset_rails() -> None:
    builder = _read("src/capsem/builder/docker.py")
    host_builder = _read("docker/Dockerfile.host-builder")
    asset_workflow = _read(".github/workflows/release-assets.yaml")

    pins = {
        "builder": re.search(r'CDXGEN_VERSION = "([0-9.]+)"', builder),
        "host_builder": re.search(r'@cyclonedx/cdxgen@([0-9.]+)', host_builder),
        "asset_workflow": re.search(r'@cyclonedx/cdxgen@([0-9.]+)', asset_workflow),
    }
    assert all(match is not None for match in pins.values())
    assert {match.group(1) for match in pins.values() if match is not None} == {
        CDXGEN_VERSION
    }
    for text in (builder, host_builder, asset_workflow):
        assert "@cyclonedx/cdxgen@latest" not in text


def test_admin_materialization_and_service_routes_expose_verified_obom_evidence() -> None:
    admin = _read("crates/capsem-admin/src/main.rs")
    service = _read("crates/capsem-service/src/main.rs")
    api = _read("crates/capsem-service/src/api.rs")

    assert "materialize_profile_obom_descriptor" in admin
    assert 'manifest_assets.get("obom.cdx.json")' in admin
    assert "check_local_asset(assets_dir, arch, logical_name, hash, size)" in admin
    assert "read_obom_generator" in admin
    assert "ProfileMaterializedObomReport" in admin
    assert 'scope: "base_image"' in admin
    assert "source profile {location} must not contain generated obom pins" in admin

    assert 'route("/profiles/{profile_id}/obom", get(handle_profile_obom))' in service
    assert "fn profile_obom_info" in service
    assert "read_local_profile_obom" in service
    assert "profile OBOM hash mismatch" in service
    assert "profile OBOM size mismatch" in service
    assert "rootfs_hash" in api
    assert "generator_version" in api


def test_docs_describe_scope_without_claiming_user_runtime_inventory() -> None:
    build_verification = _read("docs/src/content/docs/security/build-verification.md")
    build_system = _read("docs/src/content/docs/architecture/build-system.md")
    service_api = _read("docs/src/content/docs/architecture/service-api.md")

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
