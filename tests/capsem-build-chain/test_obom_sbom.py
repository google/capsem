"""Release SBOM/OBOM/build-ledger contract tests."""

from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]


def _read(path: str) -> str:
    return (PROJECT_ROOT / path).read_text(encoding="utf-8")


def test_release_workflow_generates_and_publishes_sbom_and_obom() -> None:
    workflow = _read(".github/workflows/release.yaml")

    assert "npm install -g @cyclonedx/cdxgen@latest" in workflow
    assert "CAPSEM_CDXGEN_CMD: cdxgen" in workflow
    assert workflow.index("Install OBOM generator") < workflow.index("Build VM assets")
    assert workflow.index("CAPSEM_CDXGEN_CMD: cdxgen") < workflow.index("just build-rootfs")

    assert "Generate SBOM" in workflow
    assert "cargo sbom --output-format spdx_json_2_3 > capsem-sbom.spdx.json" in workflow
    assert "Attest SBOM" in workflow
    assert "predicate-type: https://spdx.dev/Document/v2.3" in workflow
    assert "predicate-path: release-artifacts/capsem-sbom.spdx.json" in workflow

    assert "obom.cdx.json (arm64)" in workflow
    assert "obom.cdx.json (x86_64)" in workflow
    assert "VM base-image OBOM published (CycloneDX, cdxgen, per arch)" in workflow
    assert 'build-ledger.log|tool-versions.txt|B3SUMS)' in workflow
    assert "Skipping debug-only $arch/$base from release upload" in workflow
    assert "vm-build-ledger-" not in workflow


def test_builder_emits_obom_and_keeps_build_ledger_debug_scoped() -> None:
    builder = _read("src/capsem/builder/docker.py")

    assert 'OBOM_ASSET = "obom.cdx.json"' in builder
    assert 'BUILD_LEDGER_NAME = "build-ledger.log"' in builder
    assert "def generate_cyclonedx_obom" in builder
    assert "cdxgen" in builder
    assert "CAPSEM_CDXGEN_CMD" in builder
    assert "The build ledger records declared build inputs" in builder
    assert "This OBOM is the runtime" in builder
    assert '"capsem.build_ledger.v1"' in builder


def test_admin_materialization_and_service_routes_expose_verified_obom_evidence() -> None:
    admin = _read("crates/capsem-admin/src/main.rs")
    service = _read("crates/capsem-service/src/main.rs")
    api = _read("crates/capsem-service/src/api.rs")

    assert "materialize_profile_obom_descriptor" in admin
    assert "check_local_asset(assets_dir, arch, \"obom.cdx.json\"" in admin
    assert "read_obom_generator" in admin
    assert "ProfileMaterializedObomReport" in admin
    assert "scope: \"base_image\"" in admin
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
