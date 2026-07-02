"""Release doctor contract tests."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path
from types import SimpleNamespace


PROJECT_ROOT = Path(__file__).resolve().parent.parent
FAST_DOCTOR_FLAG = "doctor " + "--" + "fast"
OLD_DEBUG_CRATE = "capsem-debug" + "-upstream"


def _readiness_checker_module():
    module_path = PROJECT_ROOT / "scripts/check-remote-release-readiness.py"
    spec = importlib.util.spec_from_file_location("check_remote_release_readiness", module_path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _release_site_contract_module():
    module_path = PROJECT_ROOT / "scripts/check-release-site-contract.py"
    spec = importlib.util.spec_from_file_location("check_release_site_contract", module_path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _cloudflare_pages_project_module():
    module_path = PROJECT_ROOT / "scripts/check-cloudflare-pages-project.py"
    spec = importlib.util.spec_from_file_location("check_cloudflare_pages_project", module_path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _recipe_block(name: str) -> str:
    lines = (PROJECT_ROOT / "justfile").read_text().splitlines()
    start = next(i for i, line in enumerate(lines) if line == name or line.startswith(f"{name} "))
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line.startswith((" ", "\t", "#")):
            end = i
            break
    return "\n".join(lines[start:end])


def _workflow_job_block(name: str, workflow_name: str = "ci.yaml") -> str:
    lines = (PROJECT_ROOT / ".github" / "workflows" / workflow_name).read_text().splitlines()
    start = next(i for i, line in enumerate(lines) if line == f"  {name}:")
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line.startswith("  ") and not line.startswith("    ") and line.endswith(":"):
            end = i
            break
    return "\n".join(lines[start:end])


def _workflow_text(name: str) -> str:
    return (PROJECT_ROOT / ".github" / "workflows" / name).read_text()


def _source_text(path: str) -> str:
    return (PROJECT_ROOT / path).read_text()


def _command_attribute_prefix(source: str, struct_name: str = "Args") -> str:
    marker = f"struct {struct_name}"
    assert marker in source
    return source[: source.index(marker)]


def test_smoke_runs_full_doctor_without_fast_escape_hatch() -> None:
    block = _recipe_block("smoke:")

    assert "{{cli_binary}} doctor" in block
    assert FAST_DOCTOR_FLAG not in block
    assert f"{{{{cli_binary}}}} {FAST_DOCTOR_FLAG}" not in block


def test_doctor_fix_builds_assets_for_each_checked_in_profile() -> None:
    source = (PROJECT_ROOT / "scripts" / "doctor-common.sh").read_text()

    assert "for profile in config/profiles/*/profile.toml" in source
    assert 'just build-assets "$(basename "$(dirname "$profile")")" "$arch"' in source
    assert '"touch .dev-setup && CAPSEM_SKIP_ASSET_CHECK=1 just build-assets"' not in source


def test_install_e2e_materializes_config_before_repacking_package() -> None:
    block = _recipe_block("test-install:")

    prepare_pos = block.find("bash scripts/prepare-install-test-assets.sh")
    materialize_pos = block.find("bash scripts/materialize-config.sh")
    repack_pos = block.find("scripts/repack-deb.sh")

    assert prepare_pos != -1
    assert materialize_pos != -1
    assert repack_pos != -1
    assert prepare_pos < materialize_pos
    assert materialize_pos < repack_pos
    assert "just _materialize-config" not in block


def test_ci_materializes_runtime_profiles_after_generating_settings() -> None:
    workflow = _workflow_job_block("test")

    generate_pos = workflow.find("bash scripts/generate-settings.sh")
    prepare_assets_pos = workflow.find("bash scripts/prepare-install-test-assets.sh")
    materialize_pos = workflow.find("bash scripts/materialize-config.sh")
    python_pos = workflow.find("Python schema tests with coverage")

    assert generate_pos != -1
    assert prepare_assets_pos != -1
    assert materialize_pos != -1
    assert python_pos != -1
    assert generate_pos < prepare_assets_pos < materialize_pos < python_pos


def test_ci_python_schema_pytest_paths_exist() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    coverage_step = workflow.split("- name: Python schema tests with coverage", maxsplit=1)[
        1
    ].split("# Python integration tests", maxsplit=1)[0]
    paths = sorted(set(re.findall(r"\btests/[^\s\\]+", coverage_step)))

    missing = [path for path in paths if not (PROJECT_ROOT / path).exists()]

    assert missing == []


def test_ci_has_stable_pr_gate_over_all_required_jobs() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    gate = _workflow_job_block("pr-gate")
    release_site_job = _workflow_job_block("release-site-build")

    assert "pull_request:" in workflow
    assert "push:" in workflow
    assert (
        "needs: [test-linux, test, test-install, docs-build, site-build, release-site-build]"
        in gate
    )
    assert "if: ${{ always() }}" in gate
    assert "TEST_LINUX_RESULT: ${{ needs.test-linux.result }}" in gate
    assert "TEST_MACOS_RESULT: ${{ needs.test.result }}" in gate
    assert "TEST_INSTALL_RESULT: ${{ needs.test-install.result }}" in gate
    assert "DOCS_BUILD_RESULT: ${{ needs.docs-build.result }}" in gate
    assert "SITE_BUILD_RESULT: ${{ needs.site-build.result }}" in gate
    assert "RELEASE_SITE_BUILD_RESULT: ${{ needs.release-site-build.result }}" in gate
    assert 'test "$TEST_LINUX_RESULT" = success' in gate
    assert 'test "$TEST_MACOS_RESULT" = success' in gate
    assert 'test "$TEST_INSTALL_RESULT" = success' in gate
    assert 'test "$DOCS_BUILD_RESULT" = success' in gate
    assert 'test "$SITE_BUILD_RESULT" = success' in gate
    assert 'test "$RELEASE_SITE_BUILD_RESULT" = success' in gate
    assert "python3 scripts/write-release-site-ci-fixture.py target/release-site-fixture" in release_site_job
    assert release_site_job.index(
        "python3 scripts/write-release-site-ci-fixture.py target/release-site-fixture"
    ) < release_site_job.index(
        "cargo run -p capsem-admin -- assets channel build"
    )
    assert '--manifest "file://$PWD/target/release-site-fixture/assets/manifest.json"' in release_site_job
    assert "--assets-dir target/release-site-fixture/assets" in release_site_job


def test_pr_gate_blocks_broken_docs_and_marketing_builds() -> None:
    workflow = _workflow_text("ci.yaml")
    docs_job = _workflow_job_block("docs-build")
    site_job = _workflow_job_block("site-build")
    gate = _workflow_job_block("pr-gate")
    docs_deploy = _workflow_text("docs.yaml")
    site_deploy = _workflow_text("site.yaml")
    docs_ci = _source_text("docs/src/content/docs/development/ci.md")
    docs_ci_text = " ".join(docs_ci.split())

    assert "pr-gate:" in workflow
    assert "docs-build:" in workflow
    assert "site-build:" in workflow
    assert (
        "needs: [test-linux, test, test-install, docs-build, site-build, release-site-build]"
        in gate
    )
    assert "DOCS_BUILD_RESULT: ${{ needs.docs-build.result }}" in gate
    assert "SITE_BUILD_RESULT: ${{ needs.site-build.result }}" in gate
    assert "RELEASE_SITE_BUILD_RESULT: ${{ needs.release-site-build.result }}" in gate
    assert 'test "$DOCS_BUILD_RESULT" = success' in gate
    assert 'test "$SITE_BUILD_RESULT" = success' in gate
    assert 'test "$RELEASE_SITE_BUILD_RESULT" = success' in gate

    assert "cache-dependency-path: docs/pnpm-lock.yaml" in docs_job
    assert "cd docs && pnpm install --frozen-lockfile" in docs_job
    assert "cd docs && pnpm run build" in docs_job
    assert "pages deploy" not in docs_job

    assert "cache-dependency-path: site/pnpm-lock.yaml" in site_job
    assert "cd site && pnpm install --frozen-lockfile" in site_job
    assert "cd site && pnpm run build" in site_job
    assert "pages deploy" not in site_job

    assert "pull_request:" not in docs_deploy
    assert "pull_request:" not in site_deploy
    assert "push:" in docs_deploy
    assert "push:" in site_deploy
    assert "branches: [main]" in docs_deploy
    assert "branches: [main]" in site_deploy

    assert "docs-build" in docs_ci
    assert "site-build" in docs_ci
    assert "`pr-gate` depends on `docs-build`, `site-build`, and `release-site-build`" in docs_ci_text


def test_ci_test_steps_do_not_mask_failures_with_true() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()

    for step_name in [
        "Unit tests (KVM backend) with coverage",
        "Unit tests with coverage",
        "Integration tests with coverage",
        "Frontend type-check, test, and build",
        "Python schema tests with coverage",
        "Python integration tests (non-VM suites)",
        "Verify all integration test imports",
        "Schema drift check",
        "Run install e2e tests",
        "Build docs",
        "Build site",
    ]:
        assert f"- name: {step_name}" in workflow
        step = workflow.split(f"- name: {step_name}", maxsplit=1)[1].split(
            "\n      - name:", maxsplit=1
        )[0]
        assert "|| true" not in step, step_name
        assert "continue-on-error: true" not in step, step_name


def test_release_channel_contract_suite_is_in_pr_and_local_gates() -> None:
    workflow = _workflow_job_block("test")
    just_test = _recipe_block("test:")
    local_suite = _source_text("tests/capsem-release/test_release_channel_contract.py")

    assert "tests/capsem-release/" in workflow
    assert "Python integration tests (non-VM suites)" in workflow
    assert "tests/capsem-release/" in just_test
    assert "--ignore=tests/capsem-release" in just_test
    assert "Build chain and release tests (serial)" in just_test
    assert "validator.validate_release_site(" in local_suite
    assert "test_release_channel_contract_rejects_swapped_manifest" in local_suite
    assert "test_release_channel_contract_ignores_stale_health_summary" in local_suite
    assert "test_release_channel_contract_rejects_cache_header_drift" in local_suite
    assert "test_two_generated_release_channels_have_same_machine_contract" in local_suite


def test_release_workflows_run_disjoint_lane_policy_gates() -> None:
    binary_workflow = _workflow_text("release.yaml")
    asset_workflow = _workflow_text("release-assets.yaml")
    deploy_workflow = _workflow_text("release-channel.yaml")
    ci_workflow = _workflow_job_block("test")

    assert "push:" in binary_workflow
    assert "tags: ['v*']" in binary_workflow
    assert "Verify binary release lane policy" in binary_workflow
    assert "tests/capsem-release/test_binary_lane_gate.py" in binary_workflow
    assert "tests/capsem-release/test_release_lane_diff_policy.py" in binary_workflow
    assert "just build-kernel" not in binary_workflow
    assert "just build-rootfs" not in binary_workflow

    assert "workflow_dispatch:" in asset_workflow
    assert "profile:" in asset_workflow
    assert "Verify profile release lane policy" in asset_workflow
    assert "tests/capsem-release/test_profile_lane_gate.py" in asset_workflow
    assert "tests/capsem-release/test_release_lane_diff_policy.py" in asset_workflow
    assert "BINARY_VERSION" not in asset_workflow
    assert "Record binary release metadata" not in asset_workflow

    assert "workflow_call:" in deploy_workflow
    assert "cargo run -p capsem-admin -- assets channel build" not in deploy_workflow
    assert "cloudflare/wrangler-action@v3" in deploy_workflow

    assert "tests/capsem-release/" in ci_workflow


def test_install_e2e_generates_manifest_through_admin_rail() -> None:
    script = (PROJECT_ROOT / "scripts" / "prepare-install-test-assets.sh").read_text()

    assert "cargo run -p capsem-admin -- manifest generate" in script
    assert "arm64|aarch64)" in script
    assert 'write_if_missing "$ASSETS_DIR/$arch/vmlinuz"' in script
    assert 'create_minimal_initrd_if_missing "$ASSETS_DIR/$arch/initrd.img"' in script
    assert 'write_if_missing "$ASSETS_DIR/$arch/initrd.img"' not in script
    assert "cpio -o -H newc" not in script
    assert "gzip.open" in script
    assert "TRAILER!!!" in script
    assert 'write_if_missing "$ASSETS_DIR/$arch/rootfs.erofs"' in script
    assert "scripts/gen_manifest.py" not in script


def test_vm_asset_release_is_manual_and_deploys_asset_channel() -> None:
    workflow = _workflow_text("release-assets.yaml")

    assert "workflow_dispatch:" in workflow
    assert "push:" not in workflow
    assert "tags:" not in workflow
    assert "deployments: write" in workflow
    assert "cloudflare-release-site-preflight:" in workflow
    assert "name: Cloudflare release site preflight" in workflow
    assert "Dry run: skipping Cloudflare Pages project preflight." in workflow
    assert "RELEASE_CHANNEL_PROJECT: release" in workflow
    assert "python scripts/check-cloudflare-pages-project.py" in workflow
    assert "--project \"$RELEASE_CHANNEL_PROJECT\"" in workflow
    assert "needs: cloudflare-release-site-preflight" in workflow
    assert workflow.index("cloudflare-release-site-preflight:") < workflow.index("build-assets:")
    assert workflow.index("Cloudflare release site preflight") < workflow.index(
        "Build VM assets"
    )
    assert "just build-kernel" in workflow
    assert "just build-rootfs" in workflow
    assert "cargo run -p capsem-admin -- manifest generate assets" in workflow
    assert "binary_version:" not in workflow
    assert "BINARY_VERSION" not in workflow
    assert '--version "$BINARY_VERSION"' not in workflow
    assert "cargo run -p capsem-admin -- assets channel build" in workflow
    assert '--manifest "file://$PWD/assets/manifest.json"' in workflow
    assert "cargo run -p capsem-admin -- assets channel check" in workflow
    assert "name: Preserve binary channel metadata" in workflow
    assert "scripts/preserve-binary-channel-metadata.py" in workflow
    assert "--manifest-path assets/manifest.json" in workflow
    assert "--manifest assets/manifest.json" not in workflow
    assert workflow.index("scripts/preserve-binary-channel-metadata.py") < workflow.index(
        "scripts/check-asset-release-delta.py"
    )
    assert workflow.index("scripts/preserve-binary-channel-metadata.py") < workflow.index(
        "cargo run -p capsem-admin -- assets channel build"
    )
    assert "name: asset-release-plan" in workflow
    assert "path: target/asset-release/" in workflow
    assert "for arch in arm64 x86_64; do" in workflow
    assert "for arch_dir in assets/*; do" not in workflow
    assert 'arch_dir="assets/$arch"' in workflow
    assert 'cp "$src" "$RELEASE_DIR/$arch-$logical_name"' in workflow
    assert "current-vmlinuz" not in workflow
    assert "current-initrd.img" not in workflow
    assert "current-rootfs.erofs" not in workflow
    assert "current-obom.cdx.json" not in workflow
    assert "asset_changed: ${{ steps.asset-delta.outputs.changed }}" in workflow
    assert "asset_blobs_changed: ${{ steps.asset-delta.outputs.asset_blobs_changed }}" in workflow
    assert "if: ${{ steps.asset-delta.outputs.asset_blobs_changed == 'true' }}" in workflow
    assert "if: ${{ inputs.dry_run == false && steps.asset-delta.outputs.asset_blobs_changed == 'true' }}" in workflow
    assert "--json-output target/asset-release-delta/delta.json" in workflow
    assert "name: asset-release-delta" in workflow
    assert "path: target/asset-release-delta/" in workflow
    assert "inputs.dry_run == true" in workflow
    assert "uses: ./.github/workflows/release-channel.yaml" in workflow
    assert "dist_artifact: asset-channel-preview" in workflow
    assert (
        "if: ${{ inputs.dry_run == false && needs.assemble-channel.outputs.asset_changed == 'true' }}"
        in workflow
    )
    docs = _source_text("docs/src/content/docs/development/ci.md")
    release_skill = _source_text("skills/release-process/SKILL.md")
    asset_skill = _source_text("skills/asset-pipeline/SKILL.md")
    for text in (docs, release_skill, asset_skill):
        assert "metadata-only asset release changes" in text
        assert "deploy the release channel without republishing immutable VM blobs" in text
        assert (
            "skip deployment only when current VM blob hashes, asset release metadata, "
            "and manifest policy are all unchanged"
        ) in text
        assert "manifest policy" in text
        assert "refresh_policy" in text
        assert "`binaries` metadata" in text
        assert "host SBOM" in text
        assert "binary attestation" in text
        assert "skip deployment when asset hashes are unchanged" not in text
        assert "asset_blobs_changed" in text


def test_asset_channel_deploy_consumes_generated_dist_artifact() -> None:
    workflow = _workflow_text("release-channel.yaml")

    assert "workflow_call:" in workflow
    assert "workflow_dispatch:" not in workflow
    assert "dist_artifact:" in workflow
    assert "deploy_branch:" in workflow
    assert "release_site_url:" in workflow
    assert "default: main" in workflow
    assert "default: https://release.capsem.org" in workflow
    assert "secrets:" in workflow
    assert "CLOUDFLARE_ACCOUNT_ID:" in workflow
    assert "CLOUDFLARE_API_TOKEN:" in workflow
    assert "required: true" in workflow
    assert "actions/download-artifact@v8" in workflow
    assert "DIST_DIR: target/release-channel" in workflow
    assert 'test -f "$DIST_DIR/index.html"' in workflow
    assert 'test -f "$DIST_DIR/health.json"' in workflow
    assert 'test -f "$DIST_DIR/_headers"' in workflow
    assert 'test -f "$DIST_DIR/assets/$CHANNEL/manifest.json"' in workflow
    assert 'find "$DIST_DIR" -type f -size +25M' in workflow
    assert "Pages dist contains oversized file" in workflow
    assert "cargo run -p capsem-admin -- assets channel build" not in workflow
    assert "Require Cloudflare credentials" in workflow
    assert "CLOUDFLARE_ACCOUNT_ID secret is required to deploy release.capsem.org" in workflow
    assert "CLOUDFLARE_API_TOKEN secret is required to deploy release.capsem.org" in workflow
    assert "Verify Cloudflare Pages project" in workflow
    assert "RELEASE_CHANNEL_PROJECT: release" in workflow
    assert "python scripts/check-cloudflare-pages-project.py" in workflow
    assert "--project \"$RELEASE_CHANNEL_PROJECT\"" in workflow
    assert workflow.index("Require Cloudflare credentials") < workflow.index(
        "Verify Cloudflare Pages project"
    )
    assert workflow.index("Verify Cloudflare Pages project") < workflow.index(
        "cloudflare/wrangler-action@v3"
    )
    assert (
        "pages deploy target/release-channel/ --project-name=release --branch=${{ inputs.deploy_branch || 'main' }}"
        in workflow
    )
    assert "assets/stable/manifest.json" not in workflow
    assert "Smoke public asset channel" in workflow
    assert "RELEASE_SITE_URL: ${{ inputs.release_site_url || 'https://release.capsem.org' }}" in workflow
    assert 'curl -fsSL "$RELEASE_SITE_URL/" -o /tmp/release-index.html' in workflow
    assert 'curl -fsSL "$RELEASE_SITE_URL/health.json" -o /tmp/release-health.json' in workflow
    assert (
        'curl -fsSL "$RELEASE_SITE_URL/assets/$CHANNEL/manifest.json" -o /tmp/release-manifest.json'
        in workflow
    )
    assert 'health.get("schema") != "capsem.assets_channel.health.v1"' in workflow
    assert "health ok mismatch" in workflow
    assert "health channel mismatch" in workflow
    assert "health state mismatch" in workflow
    assert "health index URL mismatch" in workflow
    assert "health health URL mismatch" in workflow
    assert 'health.get("urls", {}).get("manifest") != expected_manifest' in workflow
    assert "valid_asset_base(asset_base)" in workflow
    assert "health binary version mismatch" in workflow
    assert "health asset version mismatch" in workflow
    assert "expected_asset_compatibility" in workflow
    assert "health asset compatibility {field} mismatch" in workflow
    assert "health asset requirement binary mismatch" in workflow
    assert "def current_asset_file_refs" in workflow
    assert "manifest current asset release arches missing or not an object" in workflow
    assert "def check_health_asset_files" in workflow
    assert "health missing asset file {url}" in workflow
    assert "health asset {field} mismatch for {url}" in workflow
    assert "health unexpected asset file {url}" in workflow
    assert workflow.index("current_binary = current.get") < workflow.index(
        'health.get("binary", {}).get("version") != current_binary'
    )
    assert "health profile state mismatch" in workflow
    assert "expected_profile_compatibility" in workflow
    assert "health profile compatibility {field} mismatch" in workflow
    assert "health profile requirement {field} mismatch" in workflow
    assert 'for key in ("binary", "assets")' in workflow
    assert "health binary update latest mismatch" in workflow
    assert "health binary update current mismatch" in workflow
    assert "health binary update state mismatch" in workflow
    assert "health binary update source mismatch" in workflow
    assert "health binary update files mismatch" in workflow
    assert "def current_binary_file_refs" in workflow
    assert "manifest current binary release files missing or not a list" in workflow
    assert "def check_host_binary_files" in workflow
    assert "{label} host binary {field} mismatch for {url}" in workflow
    assert "{label} unexpected host binary file {url}" in workflow
    assert "health image update latest must be null while unpublished" in workflow
    assert "health image update current must be null while unpublished" in workflow
    assert "health image update state mismatch" in workflow
    assert "health image update source mismatch" in workflow
    assert "health asset update latest mismatch" in workflow
    assert "health asset update current mismatch" in workflow
    assert "health asset update state mismatch" in workflow
    assert "health asset update source mismatch" in workflow
    assert "health asset update manifest mismatch" in workflow
    assert "health asset update base mismatch" in workflow
    assert "health asset update compatibility mismatch" in workflow
    assert "health asset update requirement mismatch" in workflow
    assert "health asset update canonical compatibility mismatch" in workflow
    assert "health asset update canonical requirement mismatch" in workflow
    assert "health profile update source mismatch" in workflow
    assert "health profile update hash mismatch" in workflow
    assert "health profile update compatibility mismatch" in workflow
    assert "health profile update requirement mismatch" in workflow
    assert "def check_profile_catalog_artifact" in workflow
    assert "profile catalog {source} blake3 mismatch" in workflow
    assert "profile catalog {source} must not contain file:// URLs" in workflow
    assert "profile catalog {source} revision mismatch" in workflow
    assert "profile catalog {source} current_binary mismatch" in workflow
    assert "profile catalog {source} current_assets mismatch" in workflow
    assert "catalog_expected_compatibility" in workflow
    assert "requires_newer_assets" in workflow
    assert "profile catalog {source} compatibility {field} mismatch" in workflow
    assert "health updates.{key}.latest missing or not a string" in workflow
    assert 'for key in ("profiles", "images")' in workflow
    assert "health updates.{key}.latest missing" in workflow
    assert "health updates.{key}.state missing or not a string" in workflow
    assert 'for key in ("vm_oboms", "host_sboms", "host_binary_files", "attestations")' in workflow
    assert "health evidence.{key} missing or not a list" in workflow
    assert 'manifest.get("format") != 2' in workflow
    assert 'manifest.get("assets", {}).get("current") != current_assets' in workflow
    assert 'manifest.get("binaries", {}).get("current") != current_binary' in workflow
    assert '("current binary", current_binary)' in workflow
    assert '("current assets", current_assets)' in workflow
    assert '("generated timestamp", health.get("generated_at"))' in workflow
    assert '("profile revision", health.get("profiles", {}).get("revision"))' in workflow
    assert '("profile catalog", health.get("profiles", {}).get("source"))' in workflow
    assert '("channel manifest", expected_manifest)' in workflow
    assert "index page missing {label} {value}" in workflow
    assert 'manifest_asset_releases = manifest.get("assets", {}).get("releases", {})' in workflow
    assert "health asset releases missing or not a list" in workflow
    assert "health missing asset release {version}" in workflow
    assert '"deprecated_date", manifest_release.get("deprecated_date")' in workflow
    assert "health asset release {version} {field} mismatch" in workflow
    assert "health current asset release date missing" in workflow
    assert "index page missing current asset release date" in workflow
    assert "def fetch_headers" in workflow
    assert 'RELEASE_VALIDATOR_USER_AGENT = "CapsemReleaseValidator/1.0"' in workflow
    assert "def release_site_request" in workflow
    assert "headers={\"User-Agent\": RELEASE_VALIDATOR_USER_AGENT}" in workflow
    assert "urllib.request.urlopen(release_site_request(url), timeout=20)" in workflow
    assert 'release_site_request(url, method="HEAD")' in workflow
    assert "def check_cache_header" in workflow
    assert 'check_cache_header("release index", f"{release_site_url}/", ("no-cache", "must-revalidate"))' in workflow
    assert "Cache-Control must contain {directive}" in workflow
    assert '("public", "max-age=31536000", "immutable")' in workflow
    assert '"Channel Manifest"' in workflow
    assert '"Manifest URL"' in workflow
    assert '"Capsem Binaries"' in workflow
    assert '"Profiles"' in workflow
    assert '"Asset Release History"' in workflow
    assert "index page missing {marker}" in workflow
    assert "Current Asset Files" not in workflow
    assert "release.capsem.org smoke failed after deploy." in workflow


def test_release_channel_deploy_runs_python_contract_validator_after_cloudflare_deploy() -> None:
    workflow = _workflow_text("release-channel.yaml")

    assert "Validate deployed asset channel content" in workflow
    assert "uv run python scripts/check-release-site-contract.py" in workflow
    assert "--release-site \"$RELEASE_SITE_URL\"" in workflow
    assert "--channel \"$CHANNEL\"" in workflow
    assert "--attempts 6" in workflow
    assert "--delay-seconds 10" in workflow
    assert workflow.index("cloudflare/wrangler-action@v3") < workflow.index(
        "Validate deployed asset channel content"
    )
    assert workflow.index("Validate deployed asset channel content") < workflow.index(
        "Smoke public asset channel"
    )


def test_release_channel_staging_workflow_exercises_reusable_deploy_without_release_builds() -> None:
    workflow = _workflow_text("release-channel-staging.yaml")
    reusable = _workflow_text("release-channel.yaml")
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    release_skill = (PROJECT_ROOT / "skills/release-process/SKILL.md").read_text()
    asset_skill = (PROJECT_ROOT / "skills/asset-pipeline/SKILL.md").read_text()

    assert "workflow_dispatch:" in workflow
    assert "default: staging" in workflow
    assert "default: https://staging.release-eq7.pages.dev" in workflow
    assert "build-assets:" not in workflow
    assert "build-app-macos:" not in workflow
    assert "build-app-linux:" not in workflow
    assert "just build-kernel" not in workflow
    assert "just build-rootfs" not in workflow
    assert "scripts/write-release-site-ci-fixture.py" in workflow
    assert "--without-binary-files" in workflow
    assert "--assets-dir target/release-channel-staging-fixture/assets" in workflow
    assert "--asset-source-base" not in workflow
    assert "pnpm run build:channel" in workflow
    assert "cargo run -p capsem-admin -- assets channel check" in workflow
    assert "name: asset-channel-staging-preview" in workflow
    assert "uses: ./.github/workflows/release-channel.yaml" in workflow
    assert "dist_artifact: asset-channel-staging-preview" in workflow
    assert "deploy_branch: ${{ inputs.deploy_branch }}" in workflow
    assert "release_site_url: ${{ inputs.release_site_url }}" in workflow
    assert (
        "pages deploy target/release-channel/ --project-name=release "
        "--branch=${{ inputs.deploy_branch || 'main' }}"
    ) in reusable

    for text in (docs, release_skill, asset_skill):
        assert "release-channel-staging.yaml" in text
        assert "without invoking `build-assets`" in text or "without invoking VM asset builds" in text


def test_release_site_contract_script_fails_on_content_drift(capsys) -> None:
    validator = _release_site_contract_module()

    class FakeChecker:
        BLAKE3_IMPORT_ERROR = None

        @staticmethod
        def check_release_site_dns(release_site: str):
            assert release_site == "https://release.capsem.org"
            return SimpleNamespace(ok=True, name="release.capsem.org DNS", detail="ok")

        @staticmethod
        def check_release_site_contract(release_site: str, channel: str):
            assert release_site == "https://release.capsem.org"
            assert channel == "stable"
            return SimpleNamespace(
                ok=False,
                name="release.capsem.org contract",
                detail=(
                    "health asset hash mismatch for "
                    "/assets/releases/2030.0101.1/arm64-vmlinuz"
                ),
            )

    exit_code = validator.validate_release_site(
        release_site="https://release.capsem.org",
        channel="stable",
        attempts=1,
        delay_seconds=0,
        checker=FakeChecker,
    )

    captured = capsys.readouterr()
    assert exit_code == 1
    assert "health asset hash mismatch" in captured.err
    assert "arm64-vmlinuz" in captured.err


def test_release_channel_cloudflare_prerequisites_are_documented() -> None:
    workflow = _workflow_text("release-channel.yaml")
    release_assets = _workflow_text("release-assets.yaml")
    checker = _source_text("scripts/check-cloudflare-pages-project.py")
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    release_skill = (PROJECT_ROOT / "skills/release-process/SKILL.md").read_text()
    asset_skill = (PROJECT_ROOT / "skills/asset-pipeline/SKILL.md").read_text()

    for required in (
        "CLOUDFLARE_ACCOUNT_ID",
        "CLOUDFLARE_API_TOKEN",
        "release",
        "release.capsem.org",
    ):
        assert required in workflow
        assert required in release_assets
        assert required in checker
        assert required in docs
        assert required in release_skill
        assert required in asset_skill

    docs_text = " ".join(docs.split())
    release_skill_text = " ".join(release_skill.split())
    asset_skill_text = " ".join(asset_skill.split())
    for text in (docs_text, release_skill_text, asset_skill_text):
        text_lower = text.lower()
        assert "Release-channel Cloudflare prerequisites" in text
        assert "Pages project serving `release.capsem.org`" in text
        assert "`release.capsem.org` custom domain" in text
        assert "`CLOUDFLARE_ACCOUNT_ID`" in text
        assert "`CLOUDFLARE_API_TOKEN`" in text
        assert "`scripts/check-release-site-contract.py`" in text
        assert "BLAKE3/SHA-256 content" in text
        assert "cache headers" in text_lower
        assert "rather than only checking that files exist" in text_lower
        assert "before running a live binary or vm asset channel deploy" in text_lower


def test_cloudflare_pages_project_checker_reports_visibility_failures() -> None:
    checker = _cloudflare_pages_project_module()

    ok, detail = checker.validate_project_response(
        checker.CloudflareResponse(
            200,
            {"success": True, "result": {"name": "release"}},
        ),
        "release",
    )
    assert ok is True
    assert "release is visible" in detail

    ok, detail = checker.validate_project_response(
        checker.CloudflareResponse(
            404,
            {
                "success": False,
                "errors": [
                    {
                        "code": 8000007,
                        "message": (
                            "Project not found. The specified project name does not "
                            "match any of your existing projects."
                        ),
                    }
                ],
            },
        ),
        "release",
    )
    assert ok is False
    assert "Cloudflare Pages project release is not visible" in detail
    assert "8000007: Project not found" in detail
    assert "CLOUDFLARE_ACCOUNT_ID/API_TOKEN" in detail


def test_asset_channel_deploy_smoke_verifies_public_evidence_artifacts() -> None:
    workflow = _workflow_text("release-channel.yaml")
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())

    assert "astral-sh/setup-uv@v5" in workflow
    assert "uv run python3 - <<'PY'" in workflow
    assert "import hashlib" in workflow
    assert "import blake3" in workflow
    assert "def check_evidence_artifact" in workflow
    assert (
        'check_evidence_artifact(sbom, "sha256", "sha256", "host SBOM evidence", "spdx")'
        in workflow
    )
    assert (
        'check_evidence_artifact(obom, "hash", "blake3", "VM OBOM evidence", "cyclonedx")'
        in workflow
    )
    assert 'data = fetch_bytes(resolve_release_url(url))' in workflow
    assert "health evidence host_sboms missing for published binary files" in workflow
    assert "health evidence vm_oboms missing for published VM assets" in workflow
    assert "health evidence attestations missing for published artifacts" in workflow
    assert "attestation_predicate_evidence_urls" in workflow
    assert "attestation predicate_url {predicate_url} missing from {predicate_label}" in workflow
    assert "attestation subject {subject} missing from published file lists" in workflow
    assert "resolves published host SBOM and VM OBOM evidence artifacts from `health.json`" in docs_text
    assert "verifies their advertised hashes and sizes" in docs_text
    assert "validates their SPDX 2.3 or CycloneDX document shape" in docs_text
    assert "validates attestation subjects and predicate URLs" in docs_text
    assert "VM asset attestations are incomplete unless" in docs_text
    assert "`github_attestations_vm_assets`" in docs_text
    assert "`predicate_url` points at the published VM OBOM evidence" in docs_text


def test_docs_preserve_vm_obom_attestation_predicate_contract() -> None:
    docs_text = " ".join(_source_text("docs/src/content/docs/development/ci.md").split())

    assert "VM asset attestations are incomplete unless" in docs_text
    assert "`github_attestations_vm_assets`" in docs_text
    assert "`predicate_url` points at the published VM OBOM evidence" in docs_text


def test_architecture_docs_preserve_vm_obom_attestation_predicate_contract() -> None:
    docs_text = " ".join(
        _source_text("docs/src/content/docs/architecture/asset-pipeline.md").split()
    )

    assert "SBOM and VM OBOM evidence" in docs_text
    assert "VM asset attestations are incomplete unless" in docs_text
    assert "`github_attestations_vm_assets`" in docs_text
    assert "`predicate_url` points at the published VM OBOM evidence" in docs_text


def test_release_channel_cache_header_documentation_matches_deploy_smoke() -> None:
    workflow = _workflow_text("release-channel.yaml")
    ci_docs = _source_text("docs/src/content/docs/development/ci.md")
    architecture_docs = _source_text("docs/src/content/docs/architecture/asset-pipeline.md")
    release_skill = _source_text("skills/release-process/SKILL.md")
    asset_skill = _source_text("skills/asset-pipeline/SKILL.md")

    assert "def check_cache_header" in workflow
    assert '("no-cache", "must-revalidate")' in workflow
    assert '("public", "max-age=31536000", "immutable")' in workflow

    for source in [ci_docs, architecture_docs, release_skill, asset_skill]:
        normalized = " ".join(source.split())
        assert "Cache-Control" in source
        assert "no-cache" in source
        assert "must-revalidate" in source
        assert "public, max-age=31536000, immutable" in source
        assert "mutable" in normalized
        assert "immutable" in normalized
        assert "release-channel" in normalized


def test_cdxgen_release_tool_prerequisite_is_documented() -> None:
    release_preflight = _source_text("scripts/check-release-workflow.sh")
    doctor = _source_text("scripts/doctor-common.sh")
    asset_workflow = _workflow_text("release-assets.yaml")
    docs_and_skills = [
        _source_text("docs/src/content/docs/development/getting-started.md"),
        _source_text("docs/src/content/docs/development/ci.md"),
        _source_text("skills/dev-start/SKILL.md"),
        _source_text("skills/dev-setup/SKILL.md"),
    ]

    assert "cdxgen not found (npm install -g @cyclonedx/cdxgen)" in release_preflight
    assert "for tool in gh openssl cargo-sbom cdxgen" in doctor
    assert 'skip "$tool (only needed for releases)"' in doctor
    assert "npm install -g @cyclonedx/cdxgen@latest" in asset_workflow
    assert "CAPSEM_CDXGEN_CMD: cdxgen" in asset_workflow

    for source in docs_and_skills:
        normalized = " ".join(source.split())
        assert "cdxgen" in source
        assert "release-only" in normalized.lower()
        assert "npm install -g @cyclonedx/cdxgen" in source
        assert "check-release-workflow.sh" in source


def test_cross_surface_update_smoke_prerequisites_are_covered_locally() -> None:
    cli = _source_text("crates/capsem/src/update.rs")
    cli_status = _source_text("crates/capsem/src/main.rs")
    service = _source_text("crates/capsem-service/src/tests.rs")
    tray = _source_text("crates/capsem-tray/src/menu.rs")
    tui = _source_text("crates/capsem-tui/src/tests.rs")
    frontend = _source_text("frontend/src/lib/__tests__/update-status.test.ts")
    frontend_api = _source_text("frontend/src/lib/__tests__/api.test.ts")

    assert "Profile catalog update available" in cli
    assert "Run `capsem update --assets` separately to refresh VM assets." in cli
    assert "--assets cannot be combined with --corp" in cli
    assert "update_status_lines_separate_available_and_blocked_tracks" in cli_status
    assert "available (binary" in cli_status
    assert "blocked (assets, images)" in cli_status

    assert "update_route_apply_dry_run_plans_binary_profiles_and_assets" in service
    assert "update_route_apply_confirmed_dispatches_binary_profiles_and_assets" in service
    assert "update_route_apply_rejects_ambiguous_action_body" in service
    assert 'json!(["update", "--yes"])' in service
    assert 'json!(["update", "--assets"])' in service

    assert "spec_mixed_binary_and_asset_updates_share_indicator" in tray
    assert "spec_blocked_profile_update_shows_blocked_indicator" in tray
    assert "spec_blocked_asset_update_shows_blocked_indicator" in tray
    assert "Updates: Binary, VM assets" in tray
    assert "Updates: Binary; blocked: Profiles" in tray

    assert "tui_update_smoke_matrix_covers_release_states_and_actions" in tui
    for case in [
        "binary-update",
        "profile-update",
        "asset-update",
        "mixed-binary-asset-update",
    ]:
        assert case in tui
    assert "ControlAction::Update { assets: false }" in tui
    assert "ControlAction::Update { assets: true }" in tui

    assert "summarizes mixed binary and VM asset updates without profile noise" in frontend
    assert "treats profile catalog updates as a first-class available track" in frontend
    assert "keeps blocked profile dashboard tracks visible beside available asset tracks" in frontend
    assert "Binary, VM assets available" in frontend
    assert "VM assets available for future sessions" in frontend

    assert "applies binary/profile and asset update actions through typed confirmed bodies" in frontend_api
    assert "plans update actions without confirmation only through dry runs" in frontend_api
    assert "applyUpdateAction('binary_profiles'" in frontend_api
    assert "applyUpdateAction('assets'" in frontend_api


def test_docs_and_marketing_sites_build_on_pr_and_deploy_on_main_only() -> None:
    ci_workflow = _workflow_text("ci.yaml")
    expectations = [
        (
            "docs.yaml",
            "docs",
            "docs-build",
            "capsem-docs",
            "Smoke public docs site",
            "https://docs.capsem.org",
            "docs.capsem.org smoke failed after deploy.",
        ),
        (
            "site.yaml",
            "site",
            "site-build",
            "capsem",
            "Smoke public marketing site",
            "https://capsem.org",
            "capsem.org smoke failed after deploy.",
        ),
    ]

    for workflow_name, directory, ci_job, project_name, smoke_name, site_url, failure in expectations:
        workflow = _workflow_text(workflow_name)
        trigger = workflow.split("\njobs:", maxsplit=1)[0]
        push_trigger = trigger.split("  push:", maxsplit=1)[1]
        ci_block = _workflow_job_block(ci_job)

        assert "pull_request:" not in trigger, workflow_name
        assert "push:" in workflow, workflow_name
        assert "branches: [main]" in workflow, workflow_name
        assert "paths:" not in push_trigger, workflow_name
        assert f"cache-dependency-path: {directory}/pnpm-lock.yaml" in ci_block
        assert f"cd {directory} && pnpm install --frozen-lockfile" in ci_block
        assert f"cd {directory} && pnpm run build" in ci_block
        assert (
            "needs: [test-linux, test, test-install, docs-build, site-build, release-site-build]"
            in ci_workflow
        )
        assert f"cd {directory} && pnpm install --frozen-lockfile" in workflow
        assert f"cd {directory} && pnpm run build" in workflow
        assert (
            "if: ${{ github.event_name == 'push' && github.ref == 'refs/heads/main' }}" in workflow
        )
        assert f"pages deploy {directory}/dist/ --project-name={project_name}" in workflow
        assert smoke_name in workflow
        assert f"SITE_URL: {site_url}" in workflow
        assert 'curl -fsSLI "$SITE_URL/" -o' in workflow
        assert "grep -qi '^content-type: text/html'" in workflow
        assert 'grep -q "The fastest way to ship with AI securely."' in workflow
        assert failure in workflow
        assert "release-channel.yaml" not in workflow
        assert "release.yaml" not in workflow
        assert "release-assets.yaml" not in workflow


def test_binary_release_uses_asset_channel_and_does_not_publish_vm_assets() -> None:
    workflow = _workflow_text("release.yaml")
    create_release = _workflow_job_block("create-release", "release.yaml")
    assemble_channel = _workflow_job_block("assemble-release-channel", "release.yaml")
    trigger = workflow.split("\npermissions:", maxsplit=1)[0]

    assert "push:" in trigger
    assert "tags: ['v*']" in trigger
    assert "deployments: write" in workflow
    assert "workflow_dispatch:" not in trigger
    assert "pull_request:" not in trigger
    assert "branches:" not in trigger
    assert "ASSET_MANIFEST_URL: https://release.capsem.org/assets/stable/manifest.json" in workflow
    assert "  build-assets:" not in workflow
    assert "vm-assets-" not in workflow
    assert "assets/current" not in workflow
    assert """echo '{"releases":{}}'""" not in workflow
    assert "Create stub v2 asset manifest for unit tests" in workflow
    assert "just build-kernel" not in workflow
    assert "just build-rootfs" not in workflow
    assert "cargo run -p capsem-admin -- manifest generate assets" not in workflow
    assert "generate_checksums(Path('unified-assets')" not in workflow
    assert 'gh release upload ${{ github.ref_name }} "release-artifacts/$arch' not in workflow
    assert "release-artifacts/manifest.json" not in workflow
    assert "assets-v{asset_version}" in workflow
    assert '--manifest "$ASSET_MANIFEST_URL"' in workflow
    assert "release.capsem.org" in workflow
    assert "assets channel record-binary" in workflow
    assert (
        '--asset-source-base "https://github.com/google/capsem/releases/download/assets-v{asset_version}"'
        in workflow
    )
    assert "uses: ./.github/workflows/release-channel.yaml" in workflow
    assert "dist_artifact: binary-channel-preview" in workflow
    assert "needs: [deploy-release-channel]" in workflow
    assert "cloudflare/wrangler-action" not in workflow
    assert "pages deploy" not in workflow
    assert "tests/capsem-release/test_binary_lane_gate.py" in workflow
    assert "tests/capsem-release/test_release_lane_diff_policy.py" in workflow
    assert "CLOUDFLARE_" not in workflow
    for logical_name in ("vmlinuz", "initrd.img", "rootfs.erofs", "obom.cdx.json"):
        assert f"release-artifacts/{logical_name}" not in create_release
        assert f"release-artifacts/*{logical_name}" not in create_release
    assert "release-artifacts/*.pkg" in create_release
    assert "release-artifacts/*.deb" in create_release
    assert "release-artifacts/capsem-sbom.spdx.json" in create_release
    assert "gh release create ${{ github.ref_name }}" in create_release
    assert '[ -f "$deb" ] && gh release upload ${{ github.ref_name }} "$deb"' in create_release
    assert "target/binary-channel/manifest.json" in assemble_channel
    assert assemble_channel.index("Fetch current asset channel manifest") < assemble_channel.index(
        "Record binary release metadata in channel manifest"
    )
    assert assemble_channel.index("Record binary release metadata in channel manifest") < (
        assemble_channel.index("Build release channel with existing VM assets")
    )
    assert assemble_channel.index("Build release channel with existing VM assets") < (
        assemble_channel.index("Check binary-updated release channel")
    )


def test_binary_release_channel_assembly_preflights_canonical_artifacts() -> None:
    assemble_channel = _workflow_job_block("assemble-release-channel", "release.yaml")

    assert "Verify binary channel artifacts" in assemble_channel
    assert "release-artifacts/capsem-sbom.spdx.json" in assemble_channel
    assert "::error::release-artifacts/capsem-sbom.spdx.json missing" in assemble_channel
    assert "release-artifacts/*.pkg" in assemble_channel
    assert "release-artifacts/*.deb" in assemble_channel
    assert "::error::no installable host package artifact found" in assemble_channel
    assert assemble_channel.index("Verify binary channel artifacts") < assemble_channel.index(
        "Fetch current asset channel manifest"
    )
    assert assemble_channel.index("Verify binary channel artifacts") < assemble_channel.index(
        "Record binary release metadata in channel manifest"
    )


def test_binary_release_staging_dry_run_is_separate_from_tag_release() -> None:
    workflow = _workflow_text("release-binary-staging.yaml")
    real_release = _workflow_text("release.yaml")
    assemble_channel = _workflow_job_block(
        "assemble-binary-channel",
        "release-binary-staging.yaml",
    )

    real_trigger = real_release.split("\npermissions:", maxsplit=1)[0]
    assert "workflow_dispatch:" not in real_trigger
    assert "tags: ['v*']" in real_trigger

    assert "workflow_dispatch:" in workflow
    assert "push:" not in workflow
    assert "tags:" not in workflow
    assert "contents: read" in workflow
    assert "deployments: write" not in workflow
    assert "secrets: inherit" not in workflow
    assert "uses: ./.github/workflows/release-channel.yaml" not in workflow
    assert "pages deploy" not in workflow
    assert "gh release create" not in workflow
    assert "gh release upload" not in workflow
    assert "just build-kernel" not in workflow
    assert "just build-rootfs" not in workflow
    assert "cargo run -p capsem-admin -- manifest generate assets" not in workflow
    assert "build-assets:" not in workflow
    for logical_name in ("vmlinuz", "initrd.img", "rootfs.erofs", "obom.cdx.json"):
        assert f"release-artifacts/{logical_name}" not in workflow
        assert f"release-artifacts/*{logical_name}" not in workflow

    assert "ASSET_MANIFEST_URL: https://release.capsem.org/assets/stable/manifest.json" in workflow
    assert "release-artifacts/Capsem-${VERSION}.pkg" in assemble_channel
    assert "release-artifacts/Capsem_${VERSION}_arm64.deb" in assemble_channel
    assert "release-artifacts/capsem-sbom.spdx.json" in assemble_channel
    assert "Record binary release metadata in channel manifest" in assemble_channel
    assert "assets channel record-binary" in assemble_channel
    assert "manifest.before.json" in assemble_channel
    assert "binary dry-run changed VM asset metadata" in assemble_channel
    assert '"vm_asset_jobs": "not_run"' in assemble_channel
    assert '"vm_assets_unchanged": True' in assemble_channel
    assert "Build binary channel preview with existing VM assets" in assemble_channel
    assert "assets channel build" in assemble_channel
    assert "assets channel check" in assemble_channel
    assert "name: binary-channel-dry-run-bundle" in assemble_channel
    assert "target/binary-channel-dry-run/" in assemble_channel
    assert "target/release-channel/" in assemble_channel


def test_binary_release_summary_names_pkg_and_deb_sbom_coverage() -> None:
    create_release = _workflow_job_block("create-release", "release.yaml")

    assert "SBOM attested (SPDX 2.3, pkg + deb)" in create_release
    assert "SBOM attested (SPDX 2.3, pkg)" not in create_release


def test_binary_release_does_not_publish_latest_json_updater_metadata() -> None:
    workflow = _workflow_text("release.yaml")
    docs = _source_text("docs/src/content/docs/development/ci.md")
    release_skill = _source_text("skills/release-process/SKILL.md")

    assert "latest.json" not in workflow
    assert "api.github.com/repos/google/capsem/releases/latest" not in workflow
    assert "binary freshness comes from the release-channel health index" in docs
    assert "releases do not rebuild or upload VM assets, and they do not publish" in docs
    assert "`latest.json`; binary freshness comes from the release-channel health index" in docs
    assert "`latest.json` is absent in the current release rail" in release_skill
    assert "Do not make release creation depend on `latest.json`" in release_skill


def test_self_update_docs_match_verified_package_execution() -> None:
    update_rs = _source_text("crates/capsem/src/update.rs")
    install_tests = _source_text("tests/capsem-install/test_update.py")
    install_skill = _source_text("skills/dev-installation/SKILL.md")
    architecture_skill = _source_text("skills/site-architecture/SKILL.md")
    service_docs = _source_text("docs/src/content/docs/architecture/service-architecture.md")

    assert "apply_binary_installer_plan(&plan).await?" in update_rs
    assert "Binary update applied. Restart Capsem" in update_rs
    assert "test_macos_update_yes_applies_verified_pkg_with_package_manager" in install_tests
    assert "test_linux_update_yes_applies_verified_deb_with_package_manager" in install_tests
    assert "/usr/sbin/installer -pkg {cached} -target /" in install_tests
    assert "apt-get install --yes {cached}" in install_tests
    assert "and print the tested package-manager apply command (`sudo" not in install_skill
    assert "downloads verified binary installers, prints the package-manager apply command," not in (
        architecture_skill
    )
    assert "prints the\ntested package-manager apply command for the verified package" not in (
        service_docs
    )
    assert "prints the tested package-manager apply command for audit" in install_skill
    assert "executes it through `sudo`" in install_skill
    assert "executes it with `--yes`" in architecture_skill
    assert "executes that command through\n`sudo`" in service_docs


def test_installation_skill_documents_full_host_binary_cohort() -> None:
    install_skill = _source_text("skills/dev-installation/SKILL.md")
    install_fixture = _source_text("tests/capsem-install/conftest.py")

    binaries_match = re.search(r"BINARIES = \[(.*?)\]", install_fixture, re.S)
    assert binaries_match is not None
    binaries = re.findall(r'"([^"]+)"', binaries_match.group(1))
    assert binaries

    for binary in binaries:
        assert binary in install_skill
    assert "all packaged host binaries expose a version surface" in install_skill
    assert "capsem update --yes" in install_skill


def test_installation_skill_documents_deb_preinstall_restart_rail() -> None:
    install_skill = _source_text("skills/dev-installation/SKILL.md")
    deb_preinst = _source_text("scripts/deb-preinst.sh")
    repack_deb = _source_text("scripts/repack-deb.sh")

    assert "systemctl --user stop capsem.service" in deb_preinst
    assert "event=kill_process" in deb_preinst
    assert 'cp "$SCRIPT_DIR/deb-preinst.sh" "$WORK_DIR/deb/DEBIAN/preinst"' in repack_deb

    assert "deb-preinst.sh" in install_skill
    assert "DEBIAN/preinst" in install_skill
    assert "systemctl --user stop capsem.service" in install_skill
    assert "stale helper cohort before package replacement" in install_skill


def test_release_skill_documents_deb_preinstall_restart_rail() -> None:
    release_skill = _source_text("skills/release-process/SKILL.md")
    deb_preinst = _source_text("scripts/deb-preinst.sh")
    repack_deb = _source_text("scripts/repack-deb.sh")

    assert "systemctl --user stop capsem.service" in deb_preinst
    assert "event=kill_process" in deb_preinst
    assert 'cp "$SCRIPT_DIR/deb-preinst.sh" "$WORK_DIR/deb/DEBIAN/preinst"' in repack_deb
    assert "preinst plus postinst scripts" in repack_deb
    assert "DEBIAN/preinst script" in repack_deb

    assert "deb-preinst.sh" in release_skill
    assert "DEBIAN/preinst" in release_skill
    assert "systemctl --user stop capsem.service" in release_skill
    assert "stale helper cohort before package replacement" in release_skill


def _install_release_graph_contract_fixture(
    checker,
    *,
    index_text: str | None = None,
    channels_mutator=None,
    manifest_mutator=None,
    catalog_mutator=None,
    payload_mutator=None,
    headers_mutator=None,
) -> dict[str, object]:
    site = "https://release.capsem.org"
    channel = "stable"
    generated_at = "2030-01-01T00:00:00Z"
    current_binary = "1.4.0"
    current_assets = "2030.0101.1"
    profile_revision = "profiles-2030.0101.1"
    manifest_path = "/assets/stable/manifest.json"
    profile_source = f"/profiles/releases/{profile_revision}/catalog.json"
    asset_base = "/assets/releases"

    catalog = {
        "schema": "capsem.profile_catalog.v1",
        "revision": profile_revision,
        "state": "current",
        "current_binary": current_binary,
        "current_assets": current_assets,
        "compatibility": {
            "binary": current_binary,
            "assets": current_assets,
            "min_binary": current_binary,
            "min_assets": current_assets,
            "requires_newer_binary": False,
            "requires_newer_assets": False,
        },
        "profiles": [],
    }
    if catalog_mutator is not None:
        catalog_mutator(catalog)
    catalog_bytes = (json.dumps(catalog, sort_keys=True) + "\n").encode()
    catalog_hash = checker.blake3.blake3(catalog_bytes).hexdigest()

    manifest = {
        "format": 2,
        "asset_base": asset_base,
        "assets": {
            "current": current_assets,
            "releases": {
                current_assets: {
                    "date": "2030-01-01",
                    "deprecated": False,
                    "min_binary": current_binary,
                    "arches": {},
                }
            },
        },
        "binaries": {
            "current": current_binary,
            "releases": {
                current_binary: {
                    "date": "2030-01-01",
                    "deprecated": False,
                    "min_assets": current_assets,
                    "files": [],
                }
            },
        },
    }
    if manifest_mutator is not None:
        manifest_mutator(manifest)
    manifest_bytes = (json.dumps(manifest, sort_keys=True) + "\n").encode()
    manifest_hash = checker.blake3.blake3(manifest_bytes).hexdigest()

    channels = {
        "version": 1,
        "generated_at": generated_at,
        "channels": {
            channel: {
                "label": "Stable",
                "profile_catalog": {
                    "source": profile_source,
                    "hash": catalog_hash,
                    "revision": profile_revision,
                },
                "manifests": [
                    {
                        "version": current_binary,
                        "status": "current",
                        "url": manifest_path,
                        "asset_base": asset_base,
                        "binary_version": current_binary,
                        "asset_version": current_assets,
                        "digest": {
                            "blake3": manifest_hash,
                            "sha256": "a" * 64,
                            "hmac": "stable-manifest-hmac",
                        },
                    }
                ],
            }
        },
    }
    if channels_mutator is not None:
        channels_mutator(channels)

    payloads = {
        f"{site}{manifest_path}": manifest_bytes,
        f"{site}{profile_source}": catalog_bytes,
    }
    if payload_mutator is not None:
        payload_mutator(payloads, checker)

    if index_text is None:
        index_text = " ".join(
            [
                current_binary,
                current_assets,
                "2030-01-01",
                generated_at,
                profile_revision,
                profile_source,
                manifest_path,
                "/channels.json",
            ]
        )

    headers = {
        f"{site}/": "no-cache, must-revalidate",
        f"{site}/channels.json": "no-cache, must-revalidate",
        f"{site}{manifest_path}": "no-cache, must-revalidate",
        f"{site}{profile_source}": "public, max-age=31536000, immutable",
    }
    if headers_mutator is not None:
        headers_mutator(headers)

    def fake_fetch_text(url: str):
        if url == f"{site}/":
            return checker.FetchText(text=index_text)
        return checker.FetchText(text="", error=f"unexpected text fetch {url}")

    def fake_fetch_json(url: str):
        if url == f"{site}/channels.json":
            return checker.FetchJson(data=channels)
        return checker.FetchJson(data=None, error=f"unexpected json fetch {url}")

    def fake_fetch_bytes(url: str):
        data = payloads.get(url)
        if data is None:
            return checker.FetchBytes(b"", f"unexpected fetch {url}")
        return checker.FetchBytes(data)

    def fake_fetch_headers(url: str):
        cache_control = headers.get(url)
        if cache_control is None:
            return checker.FetchHeaders({}, f"unexpected header fetch {url}")
        return checker.FetchHeaders({"cache-control": cache_control})

    checker.fetch_text = fake_fetch_text
    checker.fetch_json = fake_fetch_json
    checker.fetch_bytes = fake_fetch_bytes
    checker.fetch_headers = fake_fetch_headers
    return {
        "site": site,
        "channel": channel,
        "manifest_path": manifest_path,
        "profile_source": profile_source,
        "current_binary": current_binary,
        "current_assets": current_assets,
        "profile_revision": profile_revision,
        "manifest": manifest,
        "channels": channels,
        "catalog": catalog,
    }


def test_remote_readiness_accepts_channels_manifest_profile_graph_contract() -> None:
    checker = _readiness_checker_module()
    fixture = _install_release_graph_contract_fixture(checker)

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert result.ok, result.detail
    assert "channels.json" in result.detail
    assert "manifest" in result.detail
    assert "profile catalog" in result.detail


def test_remote_readiness_helper_edge_cases_reject_malformed_release_contract() -> None:
    checker = _readiness_checker_module()
    fixture = _install_release_graph_contract_fixture(
        checker,
        channels_mutator=lambda channels: channels.update({"channels": []}),
    )

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert "channels catalog missing or not an object" in result.detail
    assert "channels.stable missing or not an object" in result.detail


def test_remote_readiness_rejects_stale_index_profile_metadata() -> None:
    checker = _readiness_checker_module()
    fixture = _install_release_graph_contract_fixture(checker, index_text="1.4.0 2030.0101.1")

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert "index missing generated timestamp 2030-01-01T00:00:00Z" in result.detail
    assert "index missing profile revision profiles-2030.0101.1" in result.detail
    assert (
        "index missing profile catalog /profiles/releases/profiles-2030.0101.1/catalog.json"
        in result.detail
    )
    assert "index missing channel manifest /assets/stable/manifest.json" in result.detail
    assert "index missing channels catalog /channels.json" in result.detail


def test_remote_readiness_rejects_channel_manifest_digest_drift() -> None:
    checker = _readiness_checker_module()
    fixture = _install_release_graph_contract_fixture(
        checker,
        channels_mutator=lambda channels: channels["channels"]["stable"]["manifests"][0][
            "digest"
        ].update({"blake3": "0" * 64}),
    )

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert "channel manifest BLAKE3 mismatch" in result.detail


def test_remote_readiness_rejects_manifest_pointer_drift() -> None:
    checker = _readiness_checker_module()
    wrong_manifest_url = "https://release.capsem.org/assets/nightly/manifest.json"

    def copy_manifest_to_wrong_url(payloads: dict[str, bytes], _checker) -> None:
        payloads[wrong_manifest_url] = payloads[
            "https://release.capsem.org/assets/stable/manifest.json"
        ]

    fixture = _install_release_graph_contract_fixture(
        checker,
        channels_mutator=lambda channels: channels["channels"]["stable"]["manifests"][0].update(
            {"url": "/assets/nightly/manifest.json"}
        ),
        payload_mutator=copy_manifest_to_wrong_url,
    )

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert "channel manifest URL mismatch" in result.detail


def test_remote_readiness_rejects_profile_catalog_artifact_drift() -> None:
    checker = _readiness_checker_module()
    fixture = _install_release_graph_contract_fixture(
        checker,
        channels_mutator=lambda channels: channels["channels"]["stable"]["profile_catalog"].update(
            {"hash": "0" * 64}
        ),
    )

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert (
        "profile catalog /profiles/releases/profiles-2030.0101.1/catalog.json blake3 mismatch"
        in result.detail
    )


def test_remote_readiness_rejects_profile_catalog_content_drift() -> None:
    checker = _readiness_checker_module()

    def stale_catalog(catalog: dict[str, object]) -> None:
        catalog["schema"] = "capsem.profile_catalog.v0"
        catalog["revision"] = "profiles-stale"

    fixture = _install_release_graph_contract_fixture(checker, catalog_mutator=stale_catalog)

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    source = fixture["profile_source"]
    assert f"profile catalog {source} schema mismatch" in result.detail
    assert f"profile catalog {source} revision mismatch" in result.detail


def test_remote_readiness_rejects_asset_file_metadata_drift() -> None:
    checker = _readiness_checker_module()
    asset_path = "/assets/releases/2030.0101.1/arm64-rootfs.erofs"
    asset_url = f"https://release.capsem.org{asset_path}"

    def add_asset(manifest: dict[str, object]) -> None:
        release = manifest["assets"]["releases"]["2030.0101.1"]
        release["arches"] = {
            "arm64": {"rootfs.erofs": {"hash": "blake3:" + "0" * 64, "size": 4}}
        }

    def add_payload(payloads: dict[str, bytes], checker) -> None:
        payloads[asset_url] = b"rootfs"

    def add_headers(headers: dict[str, str]) -> None:
        headers[asset_url] = "public, max-age=31536000, immutable"

    fixture = _install_release_graph_contract_fixture(
        checker,
        manifest_mutator=add_asset,
        payload_mutator=add_payload,
        headers_mutator=add_headers,
    )

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert f"VM asset file {asset_path} size mismatch" in result.detail


def test_remote_readiness_rejects_cache_header_drift() -> None:
    checker = _readiness_checker_module()
    fixture = _install_release_graph_contract_fixture(
        checker,
        headers_mutator=lambda headers: headers.update(
            {"https://release.capsem.org/channels.json": "public, max-age=31536000"}
        ),
    )

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert (
        "channels JSON https://release.capsem.org/channels.json Cache-Control must contain no-cache"
        in result.detail
    )


def test_binary_release_verifies_packages_hydrate_vm_assets_from_public_channel() -> None:
    verify_downloads = _workflow_job_block("verify-release-downloads", "release.yaml")

    assert "needs: [deploy-release-channel]" in verify_downloads
    assert 'curl -fsSL "$ASSET_MANIFEST_URL" -o /tmp/verify/manifest.json' in verify_downloads
    assert "asset_base = m.get('asset_base') or '/assets/releases'" in verify_downloads
    assert "manifest_origin = f'{manifest.scheme}://{manifest.netloc}'" in verify_downloads
    assert "if '{asset_version}' in base:" in verify_downloads
    assert "version_base = base.replace('{asset_version}', asset_version)" in verify_downloads
    assert "version_base = f'{base}/{asset_version}'" in verify_downloads
    assert "if version_base.startswith('/'):" in verify_downloads
    assert "version_base = f'{manifest_origin}{version_base}'" in verify_downloads
    assert "asset_url(cur, arch, name)" in verify_downloads
    assert 'BASE="${ASSET_MANIFEST_URL%/stable/manifest.json}/releases"' not in verify_downloads
    assert 'url="$BASE/$asset_version/$arch-$name"' not in verify_downloads
    assert 'expected_hash="${hash#blake3:}"' in verify_downloads
    assert 'curl -fsSL "$url" -o "$blob"' in verify_downloads
    assert "import blake3" in verify_downloads
    assert "actual = blake3.blake3(path.read_bytes()).hexdigest()" in verify_downloads
    assert "::error::$url blake3 mismatch" in verify_downloads
    assert "asset URLs are unreachable or hash-mismatched" in verify_downloads
    assert 'code=$(curl -sIL -o /dev/null -w "%{http_code}" "$url")' in verify_downloads
    assert 'gh release download "${{ github.ref_name }}"' in verify_downloads
    assert '--pattern "Capsem_*_${deb_arch}.deb" -D /tmp/deb' in verify_downloads
    assert "skipping binary e2e" not in verify_downloads
    assert "::warning::no .deb" not in verify_downloads
    assert "::warning::no 'capsem' CLI" not in verify_downloads
    assert 'ar x "$deb"' in verify_downloads
    assert "::error::no .deb for ${deb_arch} on this release" in verify_downloads
    assert "::error::no 'capsem' CLI inside .deb" in verify_downloads
    assert "CAPSEM_HOME=/tmp/capsem-home" in verify_downloads
    assert (
        'cp ./usr/share/capsem/assets/manifest.json "$CAPSEM_HOME/assets/manifest.json"'
        in verify_downloads
    )
    assert (
        'cp ./usr/share/capsem/assets/manifest-origin.json "$CAPSEM_HOME/assets/manifest-origin.json"'
        in verify_downloads
    )
    assert '"$CAPSEM_BIN" update --assets' in verify_downloads
    assert 'find "$CAPSEM_HOME/assets/$host_arch" -name "${f%.*}-*"' in verify_downloads
    assert "End-to-end download verified against release.capsem.org." in verify_downloads


def test_manifest_source_inputs_are_url_only() -> None:
    build_pkg = (PROJECT_ROOT / "scripts" / "build-pkg.sh").read_text()
    repack_deb = (PROJECT_ROOT / "scripts" / "repack-deb.sh").read_text()
    release = _workflow_text("release.yaml")
    release_assets = _workflow_text("release-assets.yaml")
    release_channel = _workflow_text("release-channel.yaml")
    admin = (PROJECT_ROOT / "crates/capsem-admin/src/main.rs").read_text()

    for script in (build_pkg, repack_deb):
        assert "--manifest requires a URL" in script
        assert "manifest must be a URL" in script
        assert "pathlib.Path(source).read_bytes()" not in script

    for workflow in (release, release_assets, release_channel):
        source_lines = [
            line.strip() for line in workflow.splitlines() if line.strip().startswith("--manifest ")
        ]
        for line in source_lines:
            if "profile materialize" in line:
                continue
            if "$ASSET_MANIFEST_URL" in line:
                assert (
                    "ASSET_MANIFEST_URL: https://release.capsem.org/assets/stable/manifest.json"
                    in workflow
                )
            else:
                assert "file://" in line or "https://" in line or "http://" in line
            assert "--manifest assets/manifest.json" not in line
            assert '--manifest "$MANIFEST_PATH"' not in line

    assert "manifest must be a URL" in admin
    assert "unsupported {label} URL scheme" in admin


def test_asset_channel_documented_as_assets_manifest_url_not_release_index_json() -> None:
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    asset_skill = (PROJECT_ROOT / "skills/asset-pipeline/SKILL.md").read_text()
    release_skill = (PROJECT_ROOT / "skills/release-process/SKILL.md").read_text()
    release_skill_text = " ".join(release_skill.split())

    for text in (docs,):
        normalized_text = " ".join(text.split())
        assert "https://release.capsem.org/assets/stable/manifest.json" in text
        assert "target/release-channel/assets/<channel>/manifest.json" in text or (
            "target/release-channel/assets/stable/manifest.json" in text
        )
        assert "capsem.assets_channel.health.v1" in text
        assert "host SBOM references" in text
        assert "explicit `updates` block" in text
        assert "`latest` targets" in text
        assert "binary/assets/profile/image freshness checks" in text
        assert "dated asset release history" in text
        assert "first channel bootstrap may have no host binary evidence yet" in normalized_text
        assert (
            "once binary files are published, missing host SBOM evidence is release-blocking"
            in normalized_text
        )
        assert "channels/stable/index.json" not in text

    asset_skill_text = " ".join(asset_skill.split())
    assert "https://release.capsem.org/assets/stable/manifest.json" in asset_skill
    assert "target/release-channel/assets/<channel>/manifest.json" in asset_skill
    assert "`channels.json`" in asset_skill
    assert "package artifacts separate from per-binary inventory" in asset_skill_text
    assert (
        "Profiles own profile images, config files, software inventory, ABOM/OBOM evidence"
        in asset_skill_text
    )
    assert "channels/stable/index.json" not in asset_skill

    assert "https://release.capsem.org/assets/stable/manifest.json" in release_skill
    assert "target/release-channel/assets/<channel>/manifest.json" in release_skill
    assert "`channels.json`" in release_skill
    assert "per-channel manifest JSON" in release_skill
    assert "package artifacts separately from the per-binary inventory" in release_skill_text
    assert (
        "Profiles own their config files, profile images, ABOM/OBOM evidence"
        in release_skill_text
    )
    assert "channels/stable/index.json" not in release_skill


def test_release_skill_keeps_binary_and_asset_verification_decoupled() -> None:
    release_skill = (PROJECT_ROOT / "skills/release-process/SKILL.md").read_text()
    release_skill_text = " ".join(release_skill.split())

    assert "asset-channel-preview" in release_skill
    assert "generated dist artifact" in release_skill
    assert "smoke-check `https://release.capsem.org/`, `/channels.json`, and" in release_skill
    assert "`/assets/<channel>/manifest.json`" in release_skill
    assert "reject stale public HTML" in release_skill_text
    assert "generated timestamp, manifest URL, manifest version" in release_skill_text
    assert "profile revision, profile catalog URL" in release_skill
    assert "image artifact URLs" in release_skill
    assert "evidence URLs" in release_skill
    assert "Host SBOM evidence is incomplete unless" in release_skill
    assert "per-binary metadata" in release_skill
    assert "fetch immutable profile catalogs and profile-owned artifacts" in release_skill
    assert "attestation subjects and predicate URLs" in release_skill
    assert "curl -fsSL https://release.capsem.org/channels.json" in release_skill
    assert "curl -fsSL https://release.capsem.org/assets/stable/manifest.json" in release_skill
    assert "gh release download vX.Y.Z --pattern manifest.json" not in release_skill
    assert "VM asset manifests" in release_skill
    assert "root channel catalog live on" in release_skill
    assert "`ci.yaml` runs `docs-build`, `site-build`, and `release-site-build` under `pr-gate`" in release_skill_text
    assert "`docs.yaml` and `site.yaml` deploy and smoke only on" in release_skill
    assert "`https://docs.capsem.org/` plus `/getting-started/`" in release_skill
    assert "`https://capsem.org/` for marketing" in release_skill
    assert "must not depend on release tags or VM asset publication" in release_skill


def test_release_process_skill_documents_multi_channel_graph() -> None:
    release_skill = (PROJECT_ROOT / "skills/release-process/SKILL.md").read_text()
    release_skill_text = " ".join(release_skill.split())

    for required in [
        "`channels.json` lists every channel",
        "versioned manifest records",
        "exactly one `status` enum value",
        "`current`, `supported`, `deprecated`, or `revoked`",
        "Manifest records are retained for auditability",
        "package artifacts separately from the per-binary inventory",
        "Binaries are executable files inside those packages",
        "SHA-256, BLAKE3, HMAC",
        "Profiles own their config files, profile images, ABOM/OBOM evidence",
        "`min_capsem_version`",
        "Binary releases remain tag-triggered",
        "Manual VM asset releases",
        "`release-assets.yaml`",
        "`release-channel.yaml`",
        "`CAPSEM_RELEASE_MANIFEST_URL=https://release.capsem.org/assets/stable/manifest.json`",
        "`https://release.capsem.org/assets/nightly/manifest.json`",
        "stable-to-nightly acceptance",
    ]:
        assert required in release_skill_text, required

    assert "binary lane" in release_skill
    assert "profile lane" in release_skill
    assert "channel discovery lane" in release_skill
    assert "final stable-to-nightly switch" in release_skill
    assert "health.json" not in release_skill
    assert "current binary" not in release_skill_text
    assert "VM artifact" not in release_skill
    assert "schema_version" not in release_skill


def test_asset_and_install_skills_document_channel_switching() -> None:
    asset_skill = (PROJECT_ROOT / "skills/asset-pipeline/SKILL.md").read_text()
    install_skill = (PROJECT_ROOT / "skills/dev-installation/SKILL.md").read_text()
    combined = "\n".join([asset_skill, install_skill])
    combined_text = " ".join(combined.split())

    for required in [
        "`channels.json` lists all channels",
        "all versioned manifest records",
        "one status enum value",
        "`current`, `supported`, `deprecated`, or `revoked`",
        "package artifacts separate from per-binary inventory",
        "Profiles own profile images, config files, software inventory, ABOM/OBOM evidence",
        "`min_capsem_version`",
        "`--manifest` must be a URL",
        "`--manifest` and `--corp` are URL-only inputs",
        "`file:///absolute/path/to/manifest.json`",
        "`https://release.capsem.org/assets/stable/manifest.json`",
        "`https://release.capsem.org/assets/nightly/manifest.json`",
        "cached update checks must coexist under `update-checks/`",
        "Updating the co-work nightly profile",
        "must not mutate stable, packages, per-binary inventory, or other profiles",
    ]:
        assert required in combined_text, required

    assert "health.json" not in combined
    assert "current binary" not in combined_text
    assert "VM artifact" not in combined
    assert "schema_version" not in combined


def test_release_skills_preserve_vm_obom_attestation_predicate_contract() -> None:
    release_skill = (PROJECT_ROOT / "skills/release-process/SKILL.md").read_text()
    asset_skill = (PROJECT_ROOT / "skills/asset-pipeline/SKILL.md").read_text()

    for skill in (release_skill, asset_skill):
        skill_text = " ".join(skill.split())
        assert "VM asset attestations are incomplete unless" in skill_text
        assert "`github_attestations_vm_assets`" in skill_text
        assert "`predicate_url` points at the published VM OBOM evidence" in skill_text


def test_site_skills_preserve_every_main_merge_deploy_rail() -> None:
    site_infra_skill = (PROJECT_ROOT / "skills/site-infra/SKILL.md").read_text()
    site_marketing_skill = (PROJECT_ROOT / "skills/site-marketing/SKILL.md").read_text()
    site_infra_text = " ".join(site_infra_skill.split())
    site_marketing_text = " ".join(site_marketing_skill.split())

    assert "`ci.yaml` runs the merge-blocking `docs-build` job" in site_infra_text
    assert (
        "deploys only on every push to `main` and smokes `https://docs.capsem.org/`"
        in site_infra_text
    )
    assert "plus `/getting-started/`" in site_infra_text
    assert "`ci.yaml` runs the merge-blocking `site-build` job" in site_marketing_text
    assert (
        "deploys only on every push to `main` and smokes `https://capsem.org/`"
        in site_marketing_text
    )

    for skill in (site_infra_skill, site_marketing_skill):
        assert "independent from binary releases" in skill
        assert "manual VM asset releases" in skill
        assert "`release.capsem.org` asset-channel workflow" in skill


def test_capsem_update_checks_release_channel_manifest_not_github_latest() -> None:
    update_rs = (PROJECT_ROOT / "crates/capsem/src/update.rs").read_text()

    assert "https://release.capsem.org/assets/stable/manifest.json" in update_rs
    assert "DEFAULT_RELEASE_MANIFEST_URL" in update_rs
    assert "CAPSEM_RELEASE_MANIFEST_URL" in update_rs
    assert "api.github.com/repos/google/capsem/releases/latest" not in update_rs


def test_docs_do_not_teach_bare_manifest_paths_for_package_inputs() -> None:
    docs = [
        PROJECT_ROOT / "docs/src/content/docs/architecture/asset-pipeline.md",
        PROJECT_ROOT / "docs/src/content/docs/security/build-verification.md",
    ]

    for path in docs:
        text = path.read_text()
        assert "--manifest /path/to/assets/manifest.json" not in text, path
        assert "--manifest file:///path/to/assets/manifest.json" in text, path


def test_asset_skill_documents_custom_manifest_url_contract() -> None:
    skill = (PROJECT_ROOT / "skills/asset-pipeline/SKILL.md").read_text()

    assert "capsem update --assets --manifest <URL>" in skill
    assert "`--manifest` is URL-shaped" in skill
    assert "`file:///absolute/path/to/manifest.json`" in skill
    assert "`https://...` or `http://...`" in skill
    assert "`--corp` provisions corporate policy config" in skill


def test_ci_docs_describes_three_independent_publication_rails() -> None:
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()

    assert (
        "| `release.yaml` | Tag push (`v*`) | Build apps (macOS + Linux), package with the current public asset manifest, create GitHub release, then update release.capsem.org binary metadata |"
        in docs
    )
    assert (
        "| `release-assets.yaml` | Manual | Build VM assets, generate `assets/manifest.json`, and optionally deploy the asset channel |"
        in docs
    )
    assert (
        "| `release-channel-staging.yaml` | Manual | Build a deterministic staging asset channel fixture, deploy it to a Cloudflare Pages preview branch, and validate the same release-channel contract without invoking `build-assets`, `build-app-macos`, or `build-app-linux` |"
        in docs
    )
    assert (
        "| `release-binary-staging.yaml` | Manual | Build a deterministic binary-channel dry-run bundle from fake host packages and the live asset manifest, then prove VM asset metadata is unchanged without creating a GitHub release or deploying release.capsem.org |"
        in docs
    )
    assert (
        "| `docs.yaml` | Push to main | Deploy docs.capsem.org on each main merge, then smoke the live docs site |"
        in docs
    )
    assert (
        "| `site.yaml` | Push to main | Deploy capsem.org on each main merge, then smoke the live marketing site |"
        in docs
    )
    assert (
        "| `release-channel.yaml` | Called by binary or asset release | Deploy release.capsem.org from the generated release-channel site artifact |"
        in docs
    )
    assert "release.yaml` | Tag push (`v*`) | Build assets" not in docs
    assert "generated asset manifest artifact" not in docs
    assert "### pr-gate (ubuntu-latest)" in docs
    assert "`test-linux`, `test`, `test-install`, `docs-build`, `site-build`, and" in docs
    assert "`release-site-build`, runs even" in docs
    assert "fails unless every dependency job reports" in docs
    assert "After Cloudflare deploys, `release-channel.yaml` smoke" in docs
    assert "`https://release.capsem.org/` index" in docs
    assert "`/health.json`, and" in docs
    assert "`/assets/<channel>/manifest.json` before the workflow can pass" in docs
    assert "`docs.yaml` and `site.yaml` are independent from binary and VM asset release" in docs
    assert "`https://docs.capsem.org/`, content type `text/html`" in docs
    assert "`https://capsem.org/`, content type `text/html`" in docs


def test_ci_docs_compare_pr_gate_to_just_test_with_named_substitutions() -> None:
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    just_test = _recipe_block("test:")

    for stage in [
        "Audits + lint + frontend",
        "Cross-compile agent (both arches)",
        "Rust: test suite with coverage",
        "Python: non-serial tests (n=4 parallel)",
        "Python: serial timing and benchmark tests",
        "Python: Build chain and release tests (serial)",
        "Injection test",
        "Integration test",
        "Benchmarks",
        "Cross-compile Linux release (Docker)",
        "Install e2e tests (Docker + systemd)",
    ]:
        assert stage in just_test

    assert "## PR gate compared with `just test`" in docs
    assert (
        "| Audits, lint, frontend check/test/build | `test` job: dependency audit, Python lint/type/skills, frontend check/vitest/build | Same signal, split for GitHub summaries |"
        in docs
    )
    assert (
        "| VM-heavy Python suites (`pytest tests/ -n 4`) | Import collection only on hosted PR runners | Runner substitution: full execution remains a local/release gate until PR runners can host Apple VZ reliably |"
        in docs
    )
    assert (
        "| Legacy injection/integration scripts and benchmark recording | Not run in hosted PR CI | Runner substitution: still required by local `just test` before release work is claimed |"
        in docs
    )
    assert (
        "| Docs, marketing, and release-channel site builds | `docs-build`, `site-build`, and `release-site-build` install and build `docs/`, `site/`, and `release-site/` before `pr-gate` can pass | Merge-blocking build proof; deploy happens only after merge or explicit release-channel publication |"
        in docs
    )
    assert "`pr-gate` is the only status that should be required by branch protection" in docs
    assert "`pr-gate` depends on `docs-build`, `site-build`, and `release-site-build`" in docs_text
    assert (
        "needs: [test-linux, test, test-install, docs-build, site-build, release-site-build]"
        in workflow
    )


def test_remote_release_readiness_checker_is_read_only_and_covers_live_gates() -> None:
    script = (PROJECT_ROOT / "scripts/check-remote-release-readiness.py").read_text()
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())

    assert "Read-only remote release readiness checks" in script
    assert "git\", \"rev-list\", \"--left-right\", \"--count\"" in script
    assert "gh\", \"workflow\", \"view\", \"ci.yaml\"" in script
    assert "branches/{branch}/protection" in script
    assert "repos/{repo}/rules/branches/{branch}" in script
    assert "socket.getaddrinfo" in script
    assert "urllib.request.urlopen" in script
    assert "https://release.capsem.org" in script
    assert "/assets/{channel}/manifest.json" in script
    assert "/channels.json" in script
    assert "channels catalog" in script
    assert "channel manifest BLAKE3 mismatch" in script
    assert "pr-gate" in script
    assert "REQUIRED_PR_GATE_JOBS" in script
    assert '"release-site-build"' in script
    assert "current asset release date" in script
    assert 'RELEASE_VALIDATOR_USER_AGENT = "CapsemReleaseValidator/1.0"' in script
    assert "release_site_request(url)" in script
    for forbidden in [
        "git push",
        "gh release create",
        "gh release upload",
        "wrangler",
        "pages deploy",
        "--method PATCH",
        "--method PUT",
        "--method DELETE",
    ]:
        assert forbidden not in script

    assert "scripts/check-remote-release-readiness.py" in docs
    assert "read-only" in docs
    assert "remote `ci.yaml` exposes `pr-gate`" in docs_text
    assert "aggregates `test-linux`, `test`, `test-install`, `docs-build`, `site-build`, and `release-site-build`" in (
        docs_text
    )
    assert "runs with `if: ${{ always() }}` and asserts every dependency result" in docs_text
    assert "branch protection or active branch rulesets require `pr-gate`" in docs_text
    assert "`release.capsem.org` resolves and serves the asset channel" in docs_text


def test_remote_release_readiness_fetches_with_validator_user_agent(monkeypatch) -> None:
    checker = _readiness_checker_module()
    requests = []

    class FakeResponse:
        headers = {"Cache-Control": "no-cache, must-revalidate"}

        def __enter__(self):
            return self

        def __exit__(self, *_args):
            return False

        def read(self) -> bytes:
            return b"ok"

    def fake_urlopen(request, *, timeout: int):
        requests.append(request)
        assert timeout == 20
        return FakeResponse()

    monkeypatch.setattr(checker.urllib.request, "urlopen", fake_urlopen)

    body = checker.fetch_bytes("https://release.capsem.org/")
    headers = checker.fetch_headers("https://release.capsem.org/health.json")

    assert body == checker.FetchBytes(b"ok")
    assert headers == checker.FetchHeaders({"cache-control": "no-cache, must-revalidate"})
    assert [request.full_url for request in requests] == [
        "https://release.capsem.org/",
        "https://release.capsem.org/health.json",
    ]
    assert requests[0].get_header("User-agent") == "CapsemReleaseValidator/1.0"
    assert requests[1].get_header("User-agent") == "CapsemReleaseValidator/1.0"
    assert requests[1].get_method() == "HEAD"


def test_live_release_activation_order_is_documented() -> None:
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    release_skill = (PROJECT_ROOT / "skills/release-process/SKILL.md").read_text()
    asset_skill = (PROJECT_ROOT / "skills/asset-pipeline/SKILL.md").read_text()

    for text in (docs, release_skill, asset_skill):
        normalized = " ".join(text.split())
        normalized_lower = normalized.lower()
        assert "Live release activation order" in text
        assert "publish or merge the release-rail commits to `main`" in normalized_lower
        assert "wait for the expanded `pr-gate` to pass on `main`" in normalized_lower
        assert "require only `pr-gate` in branch protection or active rulesets" in normalized_lower
        assert "fail-closed `pr-gate` shape" in normalized_lower
        assert (
            "provision the `release.capsem.org` cloudflare pages project and dns"
            in normalized_lower
        )
        assert (
            "run `uv run python scripts/check-remote-release-readiness.py`"
            in normalized_lower
        )
        assert "manual VM asset workflow as a dry run" in normalized
        assert "release-binary-staging.yaml" in normalized
        assert "binary-channel-dry-run-bundle" in normalized
        assert "proof.json" in normalized
        assert (
            "vm asset metadata was not changed" in normalized_lower
            or "vm asset metadata did not change" in normalized_lower
        )
        assert "do not add `workflow_dispatch` to the real tag-triggered `release.yaml`" in normalized_lower
        assert (
            "run the tag-triggered binary release rail only from an immutable `vx.y.z` tag"
            in normalized_lower
        )
        assert (
            "run the manual vm asset workflow live only after reviewing `asset-release-plan`"
            in normalized_lower
        )
        assert "installed update smokes" in normalized
        assert normalized_lower.index(
            "publish or merge the release-rail commits to `main`"
        ) < normalized_lower.index(
            "require only `pr-gate` in branch protection or active rulesets"
        )
        assert normalized_lower.index(
            "provision the `release.capsem.org` cloudflare pages project and dns"
        ) < normalized.index(
            "manual VM asset workflow as a dry run"
        )
        assert normalized.index("release-binary-staging.yaml") < normalized_lower.index(
            "run the tag-triggered binary release rail only from an immutable `vx.y.z` tag"
        )


def test_remote_release_readiness_requires_expanded_pr_gate() -> None:
    module = _readiness_checker_module()

    inline = """
jobs:
  test-linux:
    runs-on: ubuntu-latest
  test:
    runs-on: ubuntu-latest
  test-install:
    runs-on: ubuntu-latest
  docs-build:
    runs-on: ubuntu-latest
  site-build:
    runs-on: ubuntu-latest
  release-site-build:
    runs-on: ubuntu-latest
  pr-gate:
    needs: [test-linux, test, test-install, docs-build, site-build, release-site-build]
""".strip()
    multiline = """
jobs:
  pr-gate:
    needs:
      - test-linux
      - test
      - test-install
      - docs-build
      - site-build
      - release-site-build
    if: ${{ always() }}
""".strip()
    stale = inline.replace(", docs-build, site-build, release-site-build", "")
    non_failing = inline + "\n    steps:\n      - run: echo ok\n"
    fail_closed = inline + """
    if: ${{ always() }}
    steps:
      - name: Require all CI jobs
        env:
          TEST_LINUX_RESULT: ${{ needs.test-linux.result }}
          TEST_MACOS_RESULT: ${{ needs.test.result }}
          TEST_INSTALL_RESULT: ${{ needs.test-install.result }}
          DOCS_BUILD_RESULT: ${{ needs.docs-build.result }}
          SITE_BUILD_RESULT: ${{ needs.site-build.result }}
          RELEASE_SITE_BUILD_RESULT: ${{ needs.release-site-build.result }}
        run: |
          test "$TEST_LINUX_RESULT" = success
          test "$TEST_MACOS_RESULT" = success
          test "$TEST_INSTALL_RESULT" = success
          test "$DOCS_BUILD_RESULT" = success
          test "$SITE_BUILD_RESULT" = success
          test "$RELEASE_SITE_BUILD_RESULT" = success
"""

    assert module.workflow_job_needs(module.workflow_job_block(inline, "pr-gate")) == {
        "test-linux",
        "test",
        "test-install",
        "docs-build",
        "site-build",
        "release-site-build",
    }
    assert module.workflow_job_needs(module.workflow_job_block(multiline, "pr-gate")) == {
        "test-linux",
        "test",
        "test-install",
        "docs-build",
        "site-build",
        "release-site-build",
    }
    assert not {
        "docs-build",
        "site-build",
        "release-site-build",
    }.issubset(module.workflow_job_needs(module.workflow_job_block(stale, "pr-gate")))
    assert module.pr_gate_contract_failures(
        module.workflow_job_block(fail_closed, "pr-gate")
    ) == []
    assert module.pr_gate_contract_failures(
        module.workflow_job_block(non_failing, "pr-gate")
    ) == [
        "pr-gate does not run with if: ${{ always() }}",
        "pr-gate does not assert test-linux result",
        "pr-gate does not assert test result",
        "pr-gate does not assert test-install result",
        "pr-gate does not assert docs-build result",
        "pr-gate does not assert site-build result",
        "pr-gate does not assert release-site-build result",
    ]


def test_remote_release_readiness_checker_reports_unpublished_local_commits() -> None:
    script = (PROJECT_ROOT / "scripts/check-remote-release-readiness.py").read_text()
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())

    assert "def check_local_branch_publication" in script
    assert "HEAD is ahead of {base} by {ahead} commit(s)" in script
    assert "HEAD is behind {base} by {behind} commit(s)" in script
    assert "publish or merge release-rail commits before claiming remote readiness" in script
    assert "local checkout has unpublished commits" in docs_text
    assert "publish or merge those commits before changing remote protection" in docs_text


def test_remote_release_readiness_missing_dependency_reports_setup_hint(tmp_path: Path) -> None:
    shadow = tmp_path / "shadow"
    shadow.mkdir()
    (shadow / "blake3.py").write_text(
        "raise ModuleNotFoundError(\"No module named 'blake3'\")\n",
        encoding="utf-8",
    )

    result = subprocess.run(
        [
            sys.executable,
            str(PROJECT_ROOT / "scripts/check-remote-release-readiness.py"),
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=30,
        env={"PYTHONPATH": str(shadow)},
    )

    assert result.returncode == 2
    assert "missing Python dependency: blake3" in result.stderr
    assert "uv run python scripts/check-remote-release-readiness.py" in result.stderr
    assert "Traceback" not in result.stderr


def test_remote_release_readiness_requires_active_pr_gate_rule() -> None:
    module = _readiness_checker_module()
    script = (PROJECT_ROOT / "scripts/check-remote-release-readiness.py").read_text()
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())

    assert module.classic_protection_requires_pr_gate(
        {"required_status_checks": {"contexts": ["pr-gate"]}}
    )
    assert module.classic_protection_requires_pr_gate(
        {"required_status_checks": {"checks": [{"context": "pr-gate"}]}}
    )
    assert module.active_branch_rules_require_pr_gate(
        [
            {
                "type": "required_status_checks",
                "parameters": {
                    "required_status_checks": [
                        {"context": "test-linux"},
                        {"context": "pr-gate"},
                    ]
                },
            }
        ]
    )
    assert not module.active_branch_rules_require_pr_gate(
        {
            "enforcement": "evaluate",
            "rules": [
                {
                    "type": "required_status_checks",
                    "parameters": {"required_status_checks": [{"context": "pr-gate"}]},
                }
            ],
        }
    )
    assert not module.active_branch_rules_require_pr_gate(
        [{"type": "pull_request", "parameters": {"message": "mention pr-gate only"}}]
    )
    assert 'repos/{repo}/rules/branches/{branch}' in script
    assert 'repos/{repo}/rulesets/{ruleset_id}' not in script
    assert "active branch rules" in script
    assert "branch protection or active branch rulesets require `pr-gate`" in docs_text


def test_remote_release_readiness_checker_verifies_public_evidence_artifacts() -> None:
    module = _readiness_checker_module()
    script = (PROJECT_ROOT / "scripts/check-remote-release-readiness.py").read_text()
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())
    sbom_bytes = b'{"spdxVersion":"SPDX-2.3"}'
    obom_bytes = b'{"bomFormat":"CycloneDX"}'
    sbom_url = "https://github.com/google/capsem/releases/download/v1.0.0/capsem-sbom.spdx.json"
    obom_path = "/assets/releases/2030.0101.1/arm64-obom.cdx.json"
    obom_url = f"https://release.capsem.test{obom_path}"
    payloads = {
        sbom_url: sbom_bytes,
        obom_url: obom_bytes,
    }

    def fake_fetch_bytes(url: str):
        data = payloads.get(url)
        if data is None:
            return module.FetchBytes(b"", f"unexpected fetch {url}")
        return module.FetchBytes(data)

    module.fetch_bytes = fake_fetch_bytes
    health = {
        "assets": {
            "files": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(obom_bytes).hexdigest(),
                    "size": len(obom_bytes),
                }
            ]
        },
        "evidence": {
            "vm_oboms": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(obom_bytes).hexdigest(),
                    "size": len(obom_bytes),
                }
            ],
            "host_sboms": [
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(sbom_bytes).hexdigest(),
                    "size": len(sbom_bytes),
                }
            ],
            "host_binary_files": [
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(sbom_bytes).hexdigest(),
                    "size": len(sbom_bytes),
                }
            ],
            "attestations": [
                {
                    "name": "github_attestations_vm_assets",
                    "scope": "vm_assets",
                    "workflow": ".github/workflows/release-assets.yaml",
                    "predicate_type": "https://slsa.dev/provenance/v1",
                    "predicate_url": obom_path,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [obom_path],
                },
                {
                    "name": "github_attestations_host_sbom",
                    "scope": "host_sbom",
                    "workflow": ".github/workflows/release.yaml",
                    "predicate_type": "https://spdx.dev/Document/v2.3",
                    "predicate_url": sbom_url,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [sbom_url],
                },
            ],
        },
    }

    assert module.check_release_evidence("https://release.capsem.test", health) == []

    corrupted = json.loads(json.dumps(health))
    corrupted["evidence"]["vm_oboms"][0]["hash"] = "0" * 64
    failures = module.check_release_evidence("https://release.capsem.test", corrupted)
    assert "VM OBOM evidence /assets/releases/2030.0101.1/arm64-obom.cdx.json blake3 mismatch" in failures

    assert "def check_release_evidence" in script
    assert '"sha256", "host SBOM evidence", "spdx"' in script
    assert '"blake3", "VM OBOM evidence", "cyclonedx"' in script
    assert "hashlib.sha256" in script
    assert "blake3.blake3" in script
    assert "attestation subject {subject} missing from published file lists" in script
    assert "attestation_predicate_evidence_urls" in script
    assert "attestation predicate_url {predicate_url} missing from {predicate_label}" in script
    assert "resolves published host SBOM and VM OBOM evidence artifacts" in docs_text
    assert "verifies their advertised hashes and sizes" in docs_text
    assert "validates their SPDX 2.3 or CycloneDX document shape" in docs_text
    assert "validates attestation subjects and predicate URLs" in docs_text


def test_remote_release_readiness_checker_verifies_vm_asset_file_content() -> None:
    module = _readiness_checker_module()
    script = (PROJECT_ROOT / "scripts/check-remote-release-readiness.py").read_text()
    workflow = _workflow_text("release-channel.yaml")
    rootfs_url = (
        "https://github.com/google/capsem/releases/download/"
        "assets-v2030.0101.1/arm64-rootfs.erofs"
    )
    rootfs_bytes = b"rootfs-content"

    module.fetch_bytes = lambda url: module.FetchBytes(
        rootfs_bytes if url == rootfs_url else b"", None
    )
    item = {
        "arch": "arm64",
        "logical_name": "rootfs.erofs",
        "url": rootfs_url,
        "hash": module.blake3.blake3(rootfs_bytes).hexdigest(),
        "size": len(rootfs_bytes),
    }

    assert (
        module.fetch_and_verify_evidence_artifact(
            "https://release.capsem.org", item, "blake3", "VM asset file"
        )
        == []
    )

    item["hash"] = "0" * 64
    assert (
        f"VM asset file {rootfs_url} blake3 mismatch"
        in module.fetch_and_verify_evidence_artifact(
            "https://release.capsem.org", item, "blake3", "VM asset file"
        )
    )
    assert 'fetch_and_verify_evidence_artifact(' in script
    assert '"VM asset file"' in script
    assert 'check_evidence_artifact(item, "hash", "blake3", "VM asset file")' in workflow


def test_remote_release_readiness_rejects_evidence_content_drift() -> None:
    module = _readiness_checker_module()
    bad_sbom_bytes = b'{"spdxVersion":"SPDX-2.2"}'
    bad_obom_bytes = b'{"bomFormat":"not-cyclonedx"}'
    package_url = "https://github.com/google/capsem/releases/download/v1.0.0/Capsem-1.0.0.pkg"
    sbom_url = "https://github.com/google/capsem/releases/download/v1.0.0/capsem-sbom.spdx.json"
    obom_path = "/assets/releases/2030.0101.1/arm64-obom.cdx.json"
    obom_url = f"https://release.capsem.test{obom_path}"
    payloads = {sbom_url: bad_sbom_bytes, obom_url: bad_obom_bytes}

    def fake_fetch_bytes(url: str):
        data = payloads.get(url)
        if data is None:
            return module.FetchBytes(b"", f"unexpected fetch {url}")
        return module.FetchBytes(data)

    module.fetch_bytes = fake_fetch_bytes
    health = {
        "assets": {
            "files": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(bad_obom_bytes).hexdigest(),
                    "size": len(bad_obom_bytes),
                }
            ]
        },
        "evidence": {
            "vm_oboms": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(bad_obom_bytes).hexdigest(),
                    "size": len(bad_obom_bytes),
                }
            ],
            "host_sboms": [
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(bad_sbom_bytes).hexdigest(),
                    "size": len(bad_sbom_bytes),
                }
            ],
            "host_binary_files": [
                {
                    "name": "Capsem-1.0.0.pkg",
                    "url": package_url,
                    "sha256": "1" * 64,
                    "size": 42,
                },
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(bad_sbom_bytes).hexdigest(),
                    "size": len(bad_sbom_bytes),
                },
            ],
            "attestations": [
                {
                    "name": "github_attestations_host_sbom",
                    "scope": "host_sbom",
                    "workflow": ".github/workflows/release.yaml",
                    "predicate_type": "https://spdx.dev/Document/v2.3",
                    "predicate_url": sbom_url,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [package_url],
                },
                {
                    "name": "github_attestations_vm_assets",
                    "scope": "vm_assets",
                    "workflow": ".github/workflows/release-assets.yaml",
                    "predicate_type": "https://slsa.dev/provenance/v1",
                    "predicate_url": obom_path,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [obom_path],
                },
            ],
        },
    }

    failures = module.check_release_evidence("https://release.capsem.test", health)

    assert f"host SBOM evidence {sbom_url} spdxVersion mismatch" in failures
    assert f"VM OBOM evidence {obom_path} bomFormat mismatch" in failures


def test_release_rejects_sha1_only_spdx_file_checksums() -> None:
    module = _readiness_checker_module()
    sbom_url = "https://github.com/google/capsem/releases/download/v1.4.0/capsem-sbom.spdx.json"
    sha1_only_spdx = b"""{
      "spdxVersion": "SPDX-2.3",
      "files": [
        {
          "SPDXID": "SPDXRef-File-capsem-gateway",
          "checksums": [
            {
              "algorithm": "SHA1",
              "checksumValue": "2a2bebeee60f894f3599e06c755c91944f1c3cc8"
            }
          ]
        }
      ]
    }"""
    module.fetch_bytes = lambda url: module.FetchBytes(
        sha1_only_spdx if url == sbom_url else b"", None
    )
    item = {
        "name": "capsem-sbom.spdx.json",
        "url": sbom_url,
        "sha256": hashlib.sha256(sha1_only_spdx).hexdigest(),
        "size": len(sha1_only_spdx),
    }

    failures = module.fetch_and_verify_evidence_artifact(
        "https://release.capsem.org", item, "sha256", "host SBOM evidence", "spdx"
    )

    assert (
        f"host SBOM evidence {sbom_url} SPDX file SPDXRef-File-capsem-gateway "
        "missing SHA256 checksum"
    ) in failures
    script = _source_text("scripts/check-remote-release-readiness.py")
    assert "missing SHA256 checksum" in script
    assert 'algorithm.upper() == "SHA256"' in script


def test_remote_readiness_allows_first_channel_bootstrap_without_host_evidence() -> None:
    module = _readiness_checker_module()
    obom_bytes = b'{"bomFormat":"CycloneDX"}'
    obom_path = "/assets/releases/2030.0101.1/arm64-obom.cdx.json"
    obom_url = f"https://release.capsem.test{obom_path}"

    def fake_fetch_bytes(url: str):
        if url == obom_url:
            return module.FetchBytes(obom_bytes)
        return module.FetchBytes(b"", f"unexpected fetch {url}")

    module.fetch_bytes = fake_fetch_bytes
    health = {
        "assets": {
            "files": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(obom_bytes).hexdigest(),
                    "size": len(obom_bytes),
                }
            ]
        },
        "evidence": {
            "vm_oboms": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(obom_bytes).hexdigest(),
                    "size": len(obom_bytes),
                }
            ],
            "host_sboms": [],
            "host_binary_files": [],
            "attestations": [
                {
                    "name": "github_attestations_vm_assets",
                    "scope": "vm_assets",
                    "workflow": ".github/workflows/release-assets.yaml",
                    "predicate_type": "https://slsa.dev/provenance/v1",
                    "predicate_url": obom_path,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [obom_path],
                }
            ],
        },
    }

    assert module.check_release_evidence("https://release.capsem.test", health) == []

    with_binary_without_sbom = json.loads(json.dumps(health))
    with_binary_without_sbom["evidence"]["host_binary_files"] = [
        {
            "name": "Capsem-1.4.1.pkg",
            "url": "https://github.com/google/capsem/releases/download/v1.4.1/Capsem-1.4.1.pkg",
            "sha256": "0" * 64,
            "size": 123,
        }
    ]
    failures = module.check_release_evidence(
        "https://release.capsem.test", with_binary_without_sbom
    )
    assert "health evidence host_sboms missing for published binary files" in failures


def test_release_channel_smoke_and_remote_readiness_validate_matching_attestation_predicate_evidence() -> None:
    module = _readiness_checker_module()
    script = (PROJECT_ROOT / "scripts/check-remote-release-readiness.py").read_text()
    workflow = _workflow_text("release-channel.yaml")
    sbom_bytes = b'{"spdxVersion":"SPDX-2.3"}'
    obom_bytes = b'{"bomFormat":"CycloneDX"}'
    sbom_url = "https://github.com/google/capsem/releases/download/v1.0.0/capsem-sbom.spdx.json"
    obom_path = "/assets/releases/2030.0101.1/arm64-obom.cdx.json"
    obom_url = f"https://release.capsem.test{obom_path}"
    payloads = {
        sbom_url: sbom_bytes,
        obom_url: obom_bytes,
    }

    def fake_fetch_bytes(url: str):
        data = payloads.get(url)
        if data is None:
            return module.FetchBytes(b"", f"unexpected fetch {url}")
        return module.FetchBytes(data)

    module.fetch_bytes = fake_fetch_bytes
    health = {
        "assets": {
            "files": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(obom_bytes).hexdigest(),
                    "size": len(obom_bytes),
                }
            ]
        },
        "evidence": {
            "vm_oboms": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(obom_bytes).hexdigest(),
                    "size": len(obom_bytes),
                }
            ],
            "host_sboms": [
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(sbom_bytes).hexdigest(),
                    "size": len(sbom_bytes),
                }
            ],
            "host_binary_files": [
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(sbom_bytes).hexdigest(),
                    "size": len(sbom_bytes),
                }
            ],
            "attestations": [
                {
                    "name": "github_attestations_vm_assets",
                    "scope": "vm_assets",
                    "workflow": ".github/workflows/release-assets.yaml",
                    "predicate_type": "https://slsa.dev/provenance/v1",
                    "predicate_url": obom_path,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [obom_path],
                },
                {
                    "name": "github_attestations_host_sbom",
                    "scope": "host_sbom",
                    "workflow": ".github/workflows/release.yaml",
                    "predicate_type": "https://spdx.dev/Document/v2.3",
                    "predicate_url": sbom_url,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [sbom_url],
                },
            ],
        },
    }

    assert module.check_release_evidence("https://release.capsem.test", health) == []

    corrupted = json.loads(json.dumps(health))
    corrupted["evidence"]["attestations"][0]["predicate_url"] = (
        "/assets/releases/2030.0101.1/missing-obom.cdx.json"
    )
    assert (
        "attestation predicate_url /assets/releases/2030.0101.1/missing-obom.cdx.json "
        "missing from VM OBOM evidence"
    ) in module.check_release_evidence("https://release.capsem.test", corrupted)

    missing_predicate = json.loads(json.dumps(health))
    del missing_predicate["evidence"]["attestations"][0]["predicate_url"]
    assert "health evidence VM asset attestation predicate_url missing" in (
        module.check_release_evidence("https://release.capsem.test", missing_predicate)
    )

    assert "attestation_predicate_evidence_urls" in script
    assert '"VM OBOM evidence"' in script
    assert '"host SBOM evidence"' in script
    assert "VM asset attestation predicate_url missing" in script
    assert "missing from {predicate_label}" in script
    assert "attestation_predicate_evidence_urls" in workflow
    assert '"VM OBOM evidence"' in workflow
    assert '"host SBOM evidence"' in workflow
    assert "VM asset attestation predicate_url missing" in workflow
    assert "missing from {predicate_label}" in workflow


def test_remote_readiness_rejects_attestation_rail_drift() -> None:
    module = _readiness_checker_module()
    workflow = _workflow_text("release-channel.yaml")
    obom_bytes = b'{"bomFormat":"CycloneDX"}'
    obom_path = "/assets/releases/2030.0101.1/arm64-obom.cdx.json"
    obom_url = f"https://release.capsem.test{obom_path}"
    module.fetch_bytes = lambda url: module.FetchBytes(
        obom_bytes if url == obom_url else b"",
        None if url == obom_url else f"unexpected fetch {url}",
    )
    health = {
        "assets": {
            "files": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(obom_bytes).hexdigest(),
                    "size": len(obom_bytes),
                }
            ]
        },
        "evidence": {
            "vm_oboms": [
                {
                    "arch": "arm64",
                    "logical_name": "obom.cdx.json",
                    "url": obom_path,
                    "hash": module.blake3.blake3(obom_bytes).hexdigest(),
                    "size": len(obom_bytes),
                }
            ],
            "host_sboms": [],
            "host_binary_files": [],
            "attestations": [
                {
                    "name": "github_attestations_vm_assets",
                    "scope": "host_binaries",
                    "workflow": ".github/workflows/release.yaml",
                    "predicate_type": "https://slsa.dev/provenance/v1",
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [obom_path],
                }
            ],
        },
    }

    failures = module.check_release_evidence("https://release.capsem.test", health)

    assert "health evidence github_attestations_vm_assets scope mismatch" in failures
    assert "health evidence github_attestations_vm_assets workflow mismatch" in failures
    assert "attestation_expected_rails" in workflow
    assert "health evidence {attestation_name} scope mismatch" in workflow
    assert "health evidence {attestation_name} workflow mismatch" in workflow


def test_remote_readiness_rejects_host_sbom_attestation_subjects_missing_package() -> None:
    module = _readiness_checker_module()
    script = (PROJECT_ROOT / "scripts/check-remote-release-readiness.py").read_text()
    sbom_bytes = b'{"spdxVersion":"SPDX-2.3"}'
    sbom_url = "https://github.com/google/capsem/releases/download/v1.4.1/capsem-sbom.spdx.json"
    pkg_url = "https://github.com/google/capsem/releases/download/v1.4.1/Capsem-1.4.1.pkg"

    def fake_fetch_bytes(url: str):
        if url == sbom_url:
            return module.FetchBytes(sbom_bytes)
        return module.FetchBytes(b"", f"unexpected fetch {url}")

    module.fetch_bytes = fake_fetch_bytes
    health = {
        "assets": {"files": []},
        "evidence": {
            "vm_oboms": [],
            "host_sboms": [
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(sbom_bytes).hexdigest(),
                    "size": len(sbom_bytes),
                }
            ],
            "host_binary_files": [
                {
                    "name": "Capsem-1.4.1.pkg",
                    "url": pkg_url,
                    "sha256": "1" * 64,
                    "size": 123,
                },
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(sbom_bytes).hexdigest(),
                    "size": len(sbom_bytes),
                },
            ],
            "attestations": [
                {
                    "name": "github_attestations_host_sbom",
                    "scope": "host_sbom",
                    "workflow": ".github/workflows/release.yaml",
                    "predicate_type": "https://spdx.dev/Document/v2.3",
                    "predicate_url": sbom_url,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [sbom_url],
                }
            ],
        },
    }

    failures = module.check_release_evidence("https://release.capsem.test", health)
    assert (
        "health evidence host SBOM attestation subjects missing "
        "https://github.com/google/capsem/releases/download/v1.4.1/Capsem-1.4.1.pkg"
    ) in failures

    health["evidence"]["attestations"][0]["subjects"].append(pkg_url)
    assert module.check_release_evidence("https://release.capsem.test", health) == []

    assert "host_sbom_attestation_subjects" in script
    assert "github_attestations_host_sbom" in script
    assert "host SBOM attestation subjects missing" in script


def test_remote_readiness_rejects_noncanonical_host_sbom_evidence() -> None:
    module = _readiness_checker_module()
    workflow = _workflow_text("release-channel.yaml")
    sbom_bytes = b'{"spdxVersion":"SPDX-2.3"}'
    sbom_url = "https://github.com/google/capsem/releases/download/v1.4.1/capsem-sbom.spdx.json"
    pkg_url = "https://github.com/google/capsem/releases/download/v1.4.1/Capsem-1.4.1.pkg"

    def fake_fetch_bytes(url: str):
        if url == sbom_url:
            return module.FetchBytes(sbom_bytes)
        return module.FetchBytes(b"", f"unexpected fetch {url}")

    module.fetch_bytes = fake_fetch_bytes
    health = {
        "assets": {"files": []},
        "evidence": {
            "vm_oboms": [],
            "host_sboms": [
                {
                    "name": "not-the-canonical-sbom.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(sbom_bytes).hexdigest(),
                    "size": len(sbom_bytes),
                }
            ],
            "host_binary_files": [
                {
                    "name": "Capsem-1.4.1.pkg",
                    "url": pkg_url,
                    "sha256": "1" * 64,
                    "size": 123,
                },
                {
                    "name": "capsem-sbom.spdx.json",
                    "url": sbom_url,
                    "sha256": hashlib.sha256(sbom_bytes).hexdigest(),
                    "size": len(sbom_bytes),
                },
            ],
            "attestations": [
                {
                    "name": "github_attestations_host_sbom",
                    "scope": "host_sbom",
                    "workflow": ".github/workflows/release.yaml",
                    "predicate_type": "https://spdx.dev/Document/v2.3",
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [pkg_url],
                }
            ],
        },
    }

    failures = module.check_release_evidence("https://release.capsem.test", health)

    assert f"host SBOM evidence {sbom_url} name mismatch" in failures
    assert "health evidence host SBOM attestation predicate_url missing" in failures
    assert "host SBOM evidence {url} name mismatch" in workflow
    assert "host SBOM attestation predicate_url missing" in workflow


def test_release_channel_smoke_host_sbom_attestation_subjects_cover_packages() -> None:
    workflow = _workflow_text("release-channel.yaml")

    assert "host_sbom_attestation_subjects" in workflow
    assert "github_attestations_host_sbom" in workflow
    assert "host SBOM attestation subjects missing" in workflow


def test_remote_release_readiness_checker_verifies_live_cache_headers() -> None:
    module = _readiness_checker_module()
    script = (PROJECT_ROOT / "scripts/check-remote-release-readiness.py").read_text()
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())
    calls: list[str] = []
    headers = {
        "https://release.capsem.test/": "no-cache, must-revalidate",
        "https://release.capsem.test/channels.json": "no-cache, must-revalidate",
        "https://release.capsem.test/assets/stable/manifest.json": "no-cache, must-revalidate",
        "https://release.capsem.test/assets/releases/2030.0101.1/arm64-rootfs.erofs": (
            "public, max-age=31536000, immutable"
        ),
        "https://release.capsem.test/profiles/releases/2026.06.08.7/catalog.json": (
            "public, max-age=31536000, immutable"
        ),
    }

    def fake_fetch_headers(url: str):
        calls.append(url)
        cache_control = headers.get(url)
        if cache_control is None:
            return module.FetchHeaders({}, f"unexpected header fetch {url}")
        return module.FetchHeaders({"cache-control": cache_control})

    module.fetch_headers = fake_fetch_headers
    asset_files = [
        {
            "url": "/assets/releases/2030.0101.1/arm64-rootfs.erofs",
            "hash": "a" * 64,
            "size": 4,
        }
    ]
    profile_source = "/profiles/releases/2026.06.08.7/catalog.json"

    assert (
        module.check_release_cache_headers(
            "https://release.capsem.test", "stable", profile_source, asset_files
        )
        == []
    )
    assert calls == list(headers)

    headers["https://release.capsem.test/assets/stable/manifest.json"] = (
        "public, max-age=31536000, immutable"
    )
    failures = module.check_release_cache_headers(
        "https://release.capsem.test", "stable", profile_source, asset_files
    )
    assert (
        "channel manifest https://release.capsem.test/assets/stable/manifest.json "
        "Cache-Control must contain no-cache"
    ) in failures

    assert "def check_release_cache_headers" in script
    assert "release_site_request(url, method=\"HEAD\")" in script
    assert "RELEASE_VALIDATOR_USER_AGENT" in script
    assert "Cache-Control must contain {directive}" in script
    assert "max-age=31536000" in script
    assert "Cache-Control" in docs
    assert "mutable release-channel pointers" in docs_text
    assert "immutable asset and profile artifacts" in docs_text


def test_ci_installs_b3sum_before_bootstrap_asset_hash_checks() -> None:
    workflow = _workflow_job_block("test")

    install_tools_pos = workflow.find("- name: Install tools")
    b3sum_pos = workflow.find("cargo install b3sum --locked")
    bootstrap_pos = workflow.find("uv run python -m pytest tests/capsem-bootstrap/")

    assert install_tools_pos != -1
    assert b3sum_pos != -1
    assert bootstrap_pos != -1
    assert install_tools_pos < b3sum_pos < bootstrap_pos


def test_ci_provides_sha256sum_before_codecov_uploads_on_macos() -> None:
    workflow = _workflow_job_block("test")

    install_tools_pos = workflow.find("- name: Install tools")
    sha256sum_pos = workflow.find("printf '%s\\n' '#!/bin/sh' 'exec shasum -a 256 \"$@\"'")
    codecov_pos = workflow.find("Upload Rust unit test coverage")

    assert install_tools_pos != -1
    assert sha256sum_pos != -1
    assert codecov_pos != -1
    assert install_tools_pos < sha256sum_pos < codecov_pos


def test_guest_network_doctor_is_hermetic_by_default() -> None:
    diagnostics = PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_network.py"
    source = diagnostics.read_text()

    assert "CAPSEM_RUN_PUBLIC_NETWORK_SMOKE" not in source
    assert "google.com" not in source
    assert "api.openai.com" not in source
    assert "api.anthropic.com" not in source
    assert "cdn.elie.net" not in source


def test_guest_network_doctor_exercises_oauth_fixture() -> None:
    diagnostics = PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_network.py"
    source = diagnostics.read_text()

    assert "/oauth/token" in source
    assert "grant_type=authorization_code" in source


def test_mock_server_helper_exports_https_fixture_for_host_callers() -> None:
    helper = (PROJECT_ROOT / "scripts" / "mock_server.py").read_text()

    assert "CAPSEM_MOCK_SERVER_HTTPS_BASE_URL" in helper
    assert "https_base_url" in helper
    assert "CAPSEM_MOCK_SERVER_BASE_URL" in helper


def test_guest_network_doctor_requires_local_mock_server_instead_of_skipping() -> None:
    diagnostics = PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_network.py"
    source = diagnostics.read_text()
    helper = source.split("def _require_local_mock_url", maxsplit=1)[1].split(
        "\n\n# ---------------------------------------------------------------",
        maxsplit=1,
    )[0]

    assert "pytest.skip" not in helper
    assert "pytest.fail" in helper
    assert "LOCAL_MOCK_SERVER_ENV" in helper
    assert 'LOCAL_MOCK_SERVER_ENV = "CAPSEM_MOCK_SERVER_BASE_URL"' in source


def test_guest_network_doctor_has_no_skipped_protocol_proofs() -> None:
    diagnostics = PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_network.py"
    source = diagnostics.read_text()

    assert "pytest.skip" not in source


def test_doctor_session_validation_starts_mock_server() -> None:
    source = (PROJECT_ROOT / "scripts" / "doctor_session_test.py").read_text()

    assert "from mock_server import start_mock_server, stop_process" in source
    assert "CAPSEM_MOCK_SERVER_BASE_URL" in source
    assert '[binary, "run", "capsem-doctor"]' in source


def test_release_scripts_use_shared_mock_server_helper() -> None:
    helper = PROJECT_ROOT / "scripts" / "mock_server.py"
    assert helper.exists(), "release scripts need one shared mock-server helper"

    direct_imports = [
        "scripts/doctor_session_test.py",
        "scripts/integration_test.py",
    ]
    helper_imports = [
        "tests/capsem-serial/test_mock_server_protocol_benchmark.py",
    ]
    for rel in direct_imports:
        source = (PROJECT_ROOT / rel).read_text()
        assert "from mock_server import" in source
        assert "def _read_mock_server_ready" not in source
        assert "def _start_mock_server" not in source
    for rel in helper_imports:
        source = (PROJECT_ROOT / rel).read_text()
        assert "from helpers.mock_server import" in source
        assert "def _read_mock_server_ready" not in source
        assert "def _start_mock_server" not in source


def test_mock_server_is_the_only_hermetic_fixture_server_contract() -> None:
    current_files = [
        PROJECT_ROOT / "scripts" / "mock_server.py",
        PROJECT_ROOT / "tests" / "helpers" / "mock_server.py",
        PROJECT_ROOT / "crates" / "capsem-mock-server" / "src" / "main.rs",
        PROJECT_ROOT / "guest" / "artifacts" / "capsem_bench" / "__main__.py",
        PROJECT_ROOT / "guest" / "artifacts" / "capsem_bench" / "helpers.py",
    ]

    for path in current_files:
        text = path.read_text()
        assert OLD_DEBUG_CRATE not in text
        assert "debug_upstream" not in text
        assert "CAPSEM_BENCH_MOCK_SERVER_PROTOCOL_BASE_URL" not in text

    assert (PROJECT_ROOT / "crates" / OLD_DEBUG_CRATE).exists() is False
    assert (PROJECT_ROOT / "crates" / "capsem-mock-server").exists()
    assert not list((PROJECT_ROOT / "scripts").glob("*mock_server_impl*"))
    assert (PROJECT_ROOT / "scripts" / "debug_upstream.py").exists() is False
    assert (PROJECT_ROOT / "tests" / "helpers" / "debug_upstream.py").exists() is False


def test_ci_workflow_references_only_live_workspace_packages_and_skills() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            cwd=PROJECT_ROOT,
            text=True,
        )
    )
    packages = {package["name"] for package in metadata["packages"]}
    referenced = set(re.findall(r"(?:^|\\s)-p\\s+([a-z0-9_-]+)", workflow))
    unknown = sorted(referenced - packages)

    assert unknown == []
    assert OLD_DEBUG_CRATE not in workflow
    assert "validate-skills skills" in workflow
    assert "validate-skills config/skills" not in workflow


def test_ci_builds_frontend_before_compiling_tauri_app_tests() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    build_pos = workflow.find("cd frontend && pnpm run build")
    capsem_app_pos = workflow.find("-p capsem-app")
    coverage_pos = workflow.rfind("cargo llvm-cov nextest --no-cfg-coverage", 0, capsem_app_pos)

    assert build_pos != -1, "Tauri frontendDist must exist before capsem-app tests compile"
    assert coverage_pos != -1
    assert capsem_app_pos != -1
    assert build_pos < coverage_pos < capsem_app_pos


def test_frontend_generated_settings_use_one_shared_rail() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    release_workflow = (PROJECT_ROOT / ".github" / "workflows" / "release.yaml").read_text()
    just = (PROJECT_ROOT / "justfile").read_text()

    generate_pos = workflow.find("bash scripts/generate-settings.sh")
    first_frontend_build_pos = workflow.find("cd frontend && pnpm run build")
    frontend_check_pos = workflow.find("pnpm run check")
    release_generate_pos = release_workflow.find("bash scripts/generate-settings.sh")
    release_frontend_check_pos = release_workflow.find("pnpm run check")

    assert generate_pos != -1
    assert first_frontend_build_pos != -1
    assert frontend_check_pos != -1
    assert release_generate_pos != -1
    assert release_frontend_check_pos != -1
    assert generate_pos < first_frontend_build_pos
    assert generate_pos < frontend_check_pos
    assert release_generate_pos < release_frontend_check_pos
    assert "bash scripts/generate-settings.sh" in just
    assert "dev-frontend: _pnpm-install _generate-settings" in just
    assert 'build-ui profile="debug": _pnpm-install _generate-settings' in just
    assert "test-frontend: _pnpm-install _generate-settings" in just
    assert "uv run python scripts/generate_schema.py" not in just


def test_settings_generator_uses_current_config_authority() -> None:
    generator = (PROJECT_ROOT / "scripts" / "generate_schema.py").read_text()

    assert 'PROJECT_ROOT / "config" / "docker" / "image"' in generator
    assert 'PROJECT_ROOT / "guest"' not in generator
    assert '"guest/config"' not in generator


def test_runtime_credential_store_does_not_use_native_keychain() -> None:
    runtime_files = [
        PROJECT_ROOT / "crates" / "capsem-core" / "src" / "credential_broker.rs",
        PROJECT_ROOT / "crates" / "capsem" / "src" / "service_install.rs",
        PROJECT_ROOT / "crates" / "capsem-service" / "src" / "main.rs",
        PROJECT_ROOT / "crates" / "capsem" / "src" / "main.rs",
        PROJECT_ROOT / "crates" / "capsem-gateway" / "src" / "main.rs",
    ]
    forbidden = [
        "CAPSEM_CREDENTIAL_BROKER_TEST_STORE",
        "org.capsem.credentials",
        "com.capsem.credential",
        "credential_store_backend_native",
        "durable_store_write_native",
        "durable_store_read_native",
        "durable_store_hydrate_native",
        "security find-generic-password",
        "security add-generic-password",
        "security delete-generic-password",
        "keyring::",
        "security_framework",
        "SecKeychain",
    ]

    for path in runtime_files:
        source = path.read_text()
        for needle in forbidden:
            assert needle not in source, f"{path} must not call native Keychain storage"

    broker = runtime_files[0].read_text()
    assert "CAPSEM_CREDENTIAL_STORE_PATH" in broker
    assert "default_credential_store_path()" in broker


def test_installer_codesigns_helpers_with_stable_identifiers() -> None:
    """Dev/package helper signatures must not get hash-derived identities.

    Hash-derived ad-hoc identifiers make macOS authorization prompts repeat
    after every rebuild. The installed helper binaries use stable Capsem
    identifiers even when the signing identity is ad-hoc in local/dev builds.
    """

    postinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall").read_text()
    simulate_install = (PROJECT_ROOT / "scripts" / "simulate-install.sh").read_text()
    expected = [
        "org.capsem.cli",
        "org.capsem.service",
        "org.capsem.process",
        "org.capsem.tui",
        "org.capsem.mcp",
        "org.capsem.mcp.aggregator",
        "org.capsem.mcp.builtin",
        "org.capsem.gateway",
        "org.capsem.tray",
        "org.capsem.admin",
    ]

    for script in [postinstall, simulate_install]:
        assert "codesign_identifier_for_bin()" in script
        assert 'codesign --sign - --identifier "$identifier"' in script
        for identifier in expected:
            assert identifier in script


def test_binary_update_installer_scripts_replace_and_restart_full_helper_cohort() -> None:
    preinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "preinstall").read_text()
    postinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall").read_text()
    deb_preinst = (PROJECT_ROOT / "scripts" / "deb-preinst.sh").read_text()
    deb_postinst = (PROJECT_ROOT / "scripts" / "deb-postinst.sh").read_text()
    repack_deb = (PROJECT_ROOT / "scripts" / "repack-deb.sh").read_text()
    required_bins = [
        "capsem",
        "capsem-service",
        "capsem-process",
        "capsem-tui",
        "capsem-mcp",
        "capsem-mcp-aggregator",
        "capsem-mcp-builtin",
        "capsem-gateway",
        "capsem-tray",
        "capsem-admin",
    ]
    stale_companions = [
        "capsem-service",
        "capsem-gateway",
        "capsem-tray",
        "capsem-process",
        "capsem-mcp-aggregator",
        "capsem-mcp-builtin",
    ]

    assert 'launchctl bootout "gui/$(id -u "$USER")" "$PLIST"' in preinstall
    assert "launchctl unload" in preinstall
    for name in stale_companions:
        assert name in preinstall
        assert 'pkill -9 -f "$CAPSEM_DIR/bin/$name"' in preinstall
        assert name in deb_preinst
        assert 'pkill -9 -f "$CAPSEM_DIR/bin/$name"' in deb_preinst
    assert "pkill -9 -x capsem-app" in preinstall
    assert "systemctl --user stop capsem.service" in deb_preinst
    assert "event=stop_systemd_user_service" in deb_preinst
    assert "event=kill_process" in deb_preinst
    assert 'cp "$SCRIPT_DIR/deb-preinst.sh" "$WORK_DIR/deb/DEBIAN/preinst"' in repack_deb
    assert 'chmod 755 "$WORK_DIR/deb/DEBIAN/preinst"' in repack_deb
    assert 'rm -rf "$USER_HOME/Applications/Capsem.app"' in preinstall
    assert "rm -rf /Applications/Capsem.app" in preinstall
    assert "rm -rf /usr/local/share/capsem" in preinstall

    for script in (postinstall, deb_postinst):
        for name in required_bins:
            assert name in script
        assert "update --assets" in script
        assert "event=assets_hydrated" in script

    assert 'src="$PKG_SHARE/bin/$bin"' in postinstall
    assert 'cp "$src" "$CAPSEM_DIR/bin/$bin"' in postinstall
    assert 'su "$USER" -c "$CAPSEM_DIR/bin/capsem install"' in postinstall
    assert "event=service_registered" in postinstall
    assert 'grep -q "Service:   ok"' in postinstall
    assert 'grep -q "Gateway:   ok"' in postinstall
    assert 'su "$USER" -c "open /Applications/Capsem.app"' in postinstall
    assert "capsem-tray &" not in postinstall
    assert "event=service_not_ready" in postinstall

    assert 'ln -sf "/usr/bin/$bin" "$CAPSEM_DIR/bin/$bin"' in deb_postinst
    assert 'su "$TARGET_USER" -c "XDG_RUNTIME_DIR=$XDG_DIR $CAPSEM_DIR/bin/capsem install"' in (
        deb_postinst
    )
    assert "event=service_install_invoked" in deb_postinst


def test_helper_version_surfaces_support_installed_update_smoke() -> None:
    """Helper binaries must expose --version so update smokes can prove cohort drift."""

    for path, struct_name in [
        ("crates/capsem-admin/src/main.rs", "Cli"),
        ("crates/capsem-mcp-aggregator/src/main.rs", "Args"),
        ("crates/capsem-gateway/src/main.rs", "Args"),
        ("crates/capsem-tray/src/main.rs", "Args"),
    ]:
        command = _command_attribute_prefix(_source_text(path), struct_name)
        assert "#[command" in command and "version" in command, path

    for path, binary in [
        ("crates/capsem-mcp/src/main.rs", "capsem-mcp"),
        ("crates/capsem-mcp-builtin/src/main.rs", "capsem-mcp-builtin"),
    ]:
        source = _source_text(path)
        assert 'arg == "--version" || arg == "-V"' in source, path
        assert f'println!("{binary} {{}}", env!("CARGO_PKG_VERSION"))' in source, path


def test_desktop_shell_does_not_run_native_updater_or_background_https_check() -> None:
    """The GUI must not perform hidden native updater HTTPS work on startup.

    In 1.3 update checks go through the explicit service `/update/check` route.
    The Tauri updater plugin brings its own HTTP stack and platform verifier,
    which can touch macOS Keychain/trust APIs outside Capsem's service logs.
    """

    app_manifest = (PROJECT_ROOT / "crates" / "capsem-app" / "Cargo.toml").read_text()
    app_source = (PROJECT_ROOT / "crates" / "capsem-app" / "src" / "main.rs").read_text()
    tauri_conf = (PROJECT_ROOT / "crates" / "capsem-app" / "tauri.conf.json").read_text()
    capabilities = (
        PROJECT_ROOT / "crates" / "capsem-app" / "capabilities" / "default.json"
    ).read_text()

    forbidden = [
        "tauri-plugin-updater",
        "tauri_plugin_updater",
        "UpdaterExt",
        "check_for_update_with_prompt",
        "check_for_app_update",
        "createUpdaterArtifacts",
        '"updater"',
        "updater:default",
    ]
    for text in [app_manifest, app_source, tauri_conf, capabilities]:
        for needle in forbidden:
            assert needle not in text


def test_rust_http_stack_uses_webpki_roots_not_platform_keychain_verifier() -> None:
    """Runtime HTTP clients must not pull macOS platform trust/keychain APIs."""

    manifest = (PROJECT_ROOT / "Cargo.toml").read_text()
    reqwest_line = next(line for line in manifest.splitlines() if line.startswith("reqwest = "))
    assert 'version = "0.12"' in reqwest_line
    assert "rustls-tls-webpki-roots" in reqwest_line
    assert '"rustls"' not in reqwest_line

    service_manifest = (PROJECT_ROOT / "crates" / "capsem-service" / "Cargo.toml").read_text()
    ort_line = next(line for line in service_manifest.splitlines() if line.startswith("ort = "))
    assert "default-features = false" in ort_line
    assert '"tls-rustls"' in ort_line
    assert '"tls-native"' not in ort_line

    for package in ["rustls-platform-verifier", "native-tls", "security-framework"]:
        result = subprocess.run(
            ["cargo", "tree", "-i", package, "--workspace", "--edges", "all"],
            cwd=PROJECT_ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
        if result.returncode != 0:
            assert "did not match any packages" in result.stdout
            continue
        assert package not in result.stdout


def test_stop_command_stays_before_status_and_credential_hydration() -> None:
    source = (PROJECT_ROOT / "crates" / "capsem" / "src" / "main.rs").read_text()

    stop_arm = re.search(
        r"Commands::Misc\(MiscCommands::Stop\) => \{\n(?P<body>.*?)\n        \}",
        source,
        re.DOTALL,
    )
    assert stop_arm is not None
    body = stop_arm.group("body")

    assert "service_install::stop_service().await?" in body
    assert 'println!("Service stopped.");' in body
    assert "return Ok(());" in body

    forbidden = [
        "UdsClient",
        "client::UdsClient",
        "service_json",
        "/profiles/status",
        "/corp/info",
        "/vms/list",
        "credential",
        "status_client",
        "list_client",
        "try_ensure_service",
    ]
    for needle in forbidden:
        assert needle not in body, f"`capsem stop` must not touch {needle}"

    client_creation = source.find("let client = UdsClient::new")
    stop_position = source.find("Commands::Misc(MiscCommands::Stop)")
    assert stop_position != -1
    assert client_creation != -1
    assert stop_position < client_creation


def test_changelog_does_not_advertise_keychain_credential_storage_for_1_3() -> None:
    changelog = (PROJECT_ROOT / "CHANGELOG.md").read_text()
    section = changelog.split("## [1.3.1782571508]", maxsplit=1)[1].split("\n## [", maxsplit=1)[0]

    assert "Disabled the macOS Keychain-backed credential broker store" in section
    assert "file-backed durable storage" in section
    assert "Added credential broker plugin support with Keychain-backed storage" not in section
    assert "single `org.capsem.credentials` Keychain vault item" not in section
    assert "credential store/keychain" not in section


def test_release_docs_identify_body_blobs_as_forensic_truth() -> None:
    telemetry = (
        PROJECT_ROOT / "docs" / "src" / "content" / "docs" / "architecture" / "session-telemetry.md"
    ).read_text()
    network = (
        PROJECT_ROOT / "docs" / "src" / "content" / "docs" / "security" / "network-isolation.md"
    ).read_text()
    debug_skill = (PROJECT_ROOT / "skills" / "dev-session-debug" / "SKILL.md").read_text()
    mcp_skill = (PROJECT_ROOT / "skills" / "dev-mcp" / "SKILL.md").read_text()

    for text in (telemetry, network, debug_skill, mcp_skill):
        assert "event_body_blobs" in text

    assert "forensic" in telemetry
    assert "body truth is in `event_body_blobs`" in telemetry
    assert "blob table is the ledger" in telemetry
    assert "blob table is the forensic body source" in network
    assert "not the forensic source of truth" in debug_skill
    assert "MCP-only body rail" in mcp_skill

    stale_claims = [
        "| `request_body_preview` | TEXT | First 4 KB of request body |",
        "| `response_body_preview` | TEXT | First 4 KB of response body |",
        "| `request_body_preview` | First 4 KB of request body |",
        "| `response_body_preview` | First 4 KB of response body |",
        "request_preview TEXT,              -- first 256KB",
        "response_preview TEXT,             -- first 256KB",
    ]
    combined = "\n".join([telemetry, network, debug_skill, mcp_skill])
    for claim in stale_claims:
        assert claim not in combined


def test_release_docs_reject_old_service_routes_and_manifest_signing() -> None:
    architecture_skill = (PROJECT_ROOT / "skills" / "site-architecture" / "SKILL.md").read_text()
    release_skill = (PROJECT_ROOT / "skills" / "release-process" / "SKILL.md").read_text()

    current_service_table = architecture_skill.split("### Service HTTP API", maxsplit=1)[1].split(
        "### MCP tools", maxsplit=1
    )[0]
    for retired in [
        "`/provision`",
        "`/list`",
        "`/info/{id}`",
        "`/stop/{id}`",
        "`/resume/{name}`",
        "`/persist/{id}`",
        "`/write_file/{id}`",
        "`/read_file/{id}?path=...`",
    ]:
        assert retired not in current_service_table

    assert "`/vms/create`" in current_service_table
    assert "`/vms/list`" in current_service_table
    assert "`/vms/{id}/status`" in current_service_table
    assert "Unknown routes must\nreturn 404" in current_service_table

    assert "Do not resurrect local VM manifest signing" in release_skill
    for stale in [
        "Install manifest-signing tools before signing",
        "Local manifest signing is part of setup",
        "bootstrap.sh` must install `minisign`",
        "Sign package payload manifest",
    ]:
        assert stale not in release_skill


def test_release_docs_name_tool_calls_as_canonical_tool_ledger() -> None:
    docs = [
        PROJECT_ROOT / "docs" / "src" / "content" / "docs" / "architecture" / "mcp-gateway.md",
        PROJECT_ROOT
        / "docs"
        / "src"
        / "content"
        / "docs"
        / "architecture"
        / "session-telemetry.md",
        PROJECT_ROOT / "skills" / "dev-mcp" / "SKILL.md",
        PROJECT_ROOT / "skills" / "dev-session-debug" / "SKILL.md",
    ]
    combined = "\n".join(path.read_text() for path in docs)

    assert "tool_calls` is the canonical" in combined
    assert "mcp_calls" not in combined
    assert "An MCP `tools/call` without a matching" in combined


def test_frontend_coverage_runner_declares_its_provider() -> None:
    package_json = json.loads((PROJECT_ROOT / "frontend" / "package.json").read_text())

    assert "@vitest/coverage-v8" in package_json["devDependencies"]


def test_frontend_coverage_artifacts_are_not_typechecked_or_misuploaded() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    tsconfig = json.loads((PROJECT_ROOT / "frontend" / "tsconfig.json").read_text())

    assert "frontend/coverage/coverage-final.json" in workflow
    assert "coverage/frontend/coverage-final.json" not in workflow
    assert "coverage" in tsconfig["exclude"]


def test_pr_ci_coverage_reports_without_local_threshold_abort() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()

    assert "--fail-under-lines" not in workflow
    assert "cargo llvm-cov report --no-cfg-coverage" not in workflow
    assert "codecov-unit.json" in workflow
    assert "coverage-summary.txt" in workflow
    assert "codecov-linux.json" in workflow
    assert "coverage-summary-linux.txt" in workflow


def test_linux_ci_coverage_cannot_hang_without_a_named_failure() -> None:
    workflow = _workflow_job_block("test-linux")
    nextest = tomllib.loads((PROJECT_ROOT / ".config" / "nextest.toml").read_text())

    coverage_step = workflow.split("- name: Unit tests (KVM backend) with coverage", maxsplit=1)[
        1
    ].split("- name: Upload Linux coverage", maxsplit=1)[0]
    slow_timeout = nextest["profile"]["ci"]["slow-timeout"]

    assert "timeout-minutes:" in coverage_step
    assert "cargo llvm-cov nextest" in coverage_step
    assert "--profile ci" in coverage_step
    assert slow_timeout == {
        "period": "120s",
        "terminate-after": 3,
        "grace-period": "10s",
        "on-timeout": "fail",
    }


def test_pr_ci_python_coverage_is_not_a_monolithic_vm_tree_rerun() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    coverage_step = workflow.split("- name: Python schema tests with coverage", maxsplit=1)[
        1
    ].split(
        "# Python integration tests that need no VM",
        maxsplit=1,
    )[0]

    assert "pytest tests/ --cov" not in coverage_step
    assert "tests/capsem-install" not in coverage_step
    assert "tests/capsem-serial" not in coverage_step
    assert "tests/ironbank" not in coverage_step
    assert "tests/capsem-mcp" not in coverage_step
    assert "tests/capsem-service" not in coverage_step
    assert "tests/test_config.py" in coverage_step
    assert "tests/test_manifest.py" in coverage_step
    assert "tests/test_models.py" in coverage_step
    assert "tests/test_skills.py" in coverage_step
    assert "--cov=src/capsem" in coverage_step


def test_focused_route_latency_wrapper_stays_serial() -> None:
    wrapper = PROJECT_ROOT / "tests" / "ironbank" / "test_route_latency.py"
    source = wrapper.read_text(encoding="utf-8")

    assert "pytest.mark.serial" in source
    assert "pytestmark" in source


def test_generate_settings_creates_catalog_directory_before_redirect() -> None:
    script = (PROJECT_ROOT / "scripts" / "generate-settings.sh").read_text()

    mkdir_pos = script.find('mkdir -p "$ROOT/target/config/profiles"')
    catalog_pos = script.find("target/config/profiles/catalog.generated.json")

    assert mkdir_pos != -1
    assert catalog_pos != -1
    assert mkdir_pos < catalog_pos


def test_live_provider_dotenv_files_are_gitignored() -> None:
    for name in [".env", ".env.local", ".env.ironbank"]:
        ignored = subprocess.run(
            ["git", "check-ignore", "-q", name],
            cwd=PROJECT_ROOT,
            check=False,
        )
        assert ignored.returncode == 0, f"{name} must be gitignored before live canaries"


def test_pr_ci_non_vm_python_tests_prepare_assets_and_signed_binaries() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    block = workflow.split("- name: Python integration tests (non-VM suites)", maxsplit=1)[1].split(
        "# Verify all integration test suites", maxsplit=1
    )[0]

    asset_pos = block.find("bash scripts/prepare-install-test-assets.sh")
    build_pos = block.find(
        "cargo build -p capsem-process -p capsem-service -p capsem -p capsem-mcp"
    )
    bench_package_pos = block.find("-p capsem-bench")
    bench_binary_pos = block.find("target/debug/capsem-bench-rs")
    sign_pos = block.find("codesign --sign - --entitlements entitlements.plist --force")
    pytest_pos = block.find("uv run python -m pytest tests/capsem-bootstrap/")

    assert asset_pos != -1
    assert build_pos != -1
    assert bench_package_pos != -1
    assert bench_binary_pos != -1
    assert "target/debug/capsem-bench;" not in block
    assert sign_pos != -1
    assert pytest_pos != -1
    assert asset_pos < pytest_pos
    assert build_pos < pytest_pos
    assert bench_package_pos < bench_binary_pos
    assert sign_pos < pytest_pos


def test_kvm_checkpoint_x86_state_tests_are_arch_gated() -> None:
    source = (
        PROJECT_ROOT / "crates" / "capsem-core" / "src" / "hypervisor" / "kvm" / "checkpoint.rs"
    ).read_text()
    tests = source.split("#[cfg(test)]\nmod tests", maxsplit=1)[1]

    assert "fn test_header() -> CheckpointHeader" in tests
    assert "let header = test_header();" in tests
    assert "CheckpointHeader::current" not in tests

    x86_symbols = [
        "fn snapshot(",
        "fn vm_snapshot()",
        "fn mmio(",
        "fn writes_header_and_memory()",
        "fn restores_memory_and_vcpu_state()",
        "fn overwrites_atomically()",
        "fn rejects_missing_parent()",
        "fn removes_temp_file_after_create_failure()",
        "fn restore_rejects_wrong_ram_size()",
        "fn restore_rejects_wrong_vcpu_count()",
        "fn restore_rejects_trailing_bytes()",
    ]
    for symbol in x86_symbols:
        prefix = tests.split(symbol, maxsplit=1)[0].rsplit("\n", maxsplit=4)[0]
        window = tests[len(prefix) : tests.find(symbol)]
        assert '#[cfg(target_arch = "x86_64")]' in window


def test_mock_server_uses_rust_fixture_crate() -> None:
    root_cargo = (PROJECT_ROOT / "Cargo.toml").read_text()
    cli_cargo = (PROJECT_ROOT / "crates" / "capsem" / "Cargo.toml").read_text()
    cli_main = (PROJECT_ROOT / "crates" / "capsem" / "src" / "main.rs").read_text()

    assert '"crates/capsem-mock-server"' in root_cargo
    assert "capsem-mock-server" not in cli_cargo
    assert "mock_server_impl" not in cli_main
    assert "capsem-mock-server" in cli_main


def test_serial_benchmark_release_proofs_are_not_env_gated() -> None:
    benchmark = PROJECT_ROOT / "tests" / "capsem-serial" / "test_mock_server_protocol_benchmark.py"
    source = benchmark.read_text()

    assert "CAPSEM_RUN_MOCK_SERVER_PROTOCOL_BENCH" not in source
    assert "pytest.skip(" not in source
    assert "total_requests = 10" not in source
    assert 'CAPSEM_BENCH_TOTAL_REQUESTS", "10"' not in source
    assert 'CAPSEM_BENCH_CONCURRENCY", "1"' not in source
    assert '"capsem-bench-rs",' in source
    assert '"protocol",' in source


def test_benchmark_release_path_wires_mock_server_and_forbids_http_skip() -> None:
    bench = _recipe_block("bench:")
    baseline = (
        PROJECT_ROOT / "tests" / "capsem-serial" / "test_capsem_bench_baseline.py"
    ).read_text()

    assert "tests/capsem-serial/test_capsem_bench_baseline.py" in bench
    assert '{{cli_binary}} run "capsem-bench"' not in bench
    assert "from helpers.mock_server import start_mock_server, stop_process" in baseline
    assert "CAPSEM_MOCK_SERVER_BASE_URL" in baseline
    assert "CAPSEM_MOCK_SERVER_HTTPS_BASE_URL" in baseline
    assert "CAPSEM_BENCH_TOTAL_REQUESTS" in baseline
    assert "CAPSEM_BENCH_CONCURRENCY" in baseline
    assert "RELEASE_PROTOCOL_REQUESTS = 50_000" in baseline
    assert "RELEASE_PROTOCOL_CONCURRENCY = 64" in baseline
    assert "RELEASE_PROTOCOL_REQUESTS = 10" not in baseline
    assert "RELEASE_PROTOCOL_CONCURRENCY = 1" not in baseline
    assert "validate_capsem_bench_result(data)" in baseline
    assert "capsem-bench all" in baseline
    assert "skipped" in baseline
    assert 'benchmarks" / "capsem-bench"' in baseline


def test_integration_script_has_no_live_ai_provider_escape_hatch() -> None:
    source = (PROJECT_ROOT / "scripts" / "integration_test.py").read_text()

    assert "GEMINI_API_KEY" not in source
    assert "GOOGLE_API_KEY" not in source
    assert "googleapis.com" not in source
    assert "include_gemini_probe" not in source


def test_integration_script_uses_current_tool_call_arguments_column() -> None:
    source = (PROJECT_ROOT / "scripts" / "integration_test.py").read_text()

    assert "request_preview FROM tool_calls" not in source
    assert "SELECT id, arguments FROM tool_calls WHERE origin = 'mcp'" in source


def test_builder_has_no_legacy_ai_provider_authoring_rail() -> None:
    forbidden = (
        "AiProviderConfig",
        "ApiKeyConfig",
        "add_ai_provider",
        "include_providers",
        "ai_providers",
        "config/ai",
        'config" / "ai"',
        "AI provider",
    )
    checked_roots = [
        PROJECT_ROOT / "src" / "capsem" / "builder",
        PROJECT_ROOT / "guest" / "config",
    ]
    offenders: list[str] = []
    for root in checked_roots:
        for path in sorted(root.rglob("*")):
            if not path.is_file() or "__pycache__" in path.parts:
                continue
            if path == Path(__file__) or path.name == "test_active_docs_profile_contract.py":
                continue
            rel = path.relative_to(PROJECT_ROOT)
            try:
                text = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            for marker in forbidden:
                if marker in text:
                    offenders.append(f"{rel}: contains {marker!r}")
                    break

    assert offenders == [], "legacy AI-provider builder rail still exists:\n" + "\n".join(offenders)


def test_gateway_docs_describe_explicit_routes_not_generic_forwarding() -> None:
    docs = "\n".join(
        path.read_text(encoding="utf-8")
        for path in (
            PROJECT_ROOT / "docs" / "src" / "content" / "docs" / "architecture" / "service-api.md",
            PROJECT_ROOT / "skills" / "site-architecture" / "SKILL.md",
            PROJECT_ROOT / "skills" / "frontend-design" / "SKILL.md",
        )
    )

    assert "Unknown routes must return 404" in docs
    assert "explicit route table" in docs
    assert "`*` (fallback)" not in docs
    assert "transparent fallback" not in docs
    assert "Transparent proxy" not in docs
    assert "transparently" not in docs
    assert "generic path forwarding" not in docs


def test_config_contract_has_no_admin_or_registry_authority() -> None:
    assert not (PROJECT_ROOT / "config" / "admin").exists()
    assert (PROJECT_ROOT / "config" / "settings" / "settings.toml").is_file()
    assert (PROJECT_ROOT / "config" / "settings" / "schema.generated.json").is_file()
    assert (PROJECT_ROOT / "config" / "settings" / "ui-metadata.toml").is_file()
    assert (PROJECT_ROOT / "config" / "settings" / "ui-metadata.generated.json").is_file()

    forbidden = (
        "config/admin",
        "config/guest",
        "settings registry",
        "settings-registry",
        "settings-schema.generated",
        "mcp-tools.generated",
    )
    checked_roots = [
        PROJECT_ROOT / "scripts",
        PROJECT_ROOT / "src" / "capsem" / "builder",
        PROJECT_ROOT / "crates" / "capsem-admin" / "src",
        PROJECT_ROOT / "crates" / "capsem-core" / "src" / "net" / "policy_config",
        PROJECT_ROOT / "tests",
        PROJECT_ROOT / "docs" / "src" / "content" / "docs",
        PROJECT_ROOT / "skills",
        PROJECT_ROOT / ".github" / "workflows",
    ]
    offenders: list[str] = []
    for root in checked_roots:
        for path in sorted(root.rglob("*")):
            if not path.is_file() or "__pycache__" in path.parts:
                continue
            if path == Path(__file__) or path.name == "test_active_docs_profile_contract.py":
                continue
            rel = path.relative_to(PROJECT_ROOT)
            try:
                text = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            for marker in forbidden:
                if marker in text:
                    offenders.append(f"{rel}: contains {marker!r}")
                    break
    assert offenders == [], "admin/registry config authority still exists:\n" + "\n".join(offenders)


def test_builder_has_no_guest_scaffold_authoring_rail() -> None:
    assert not (PROJECT_ROOT / "src" / "capsem" / "builder" / "scaffold.py").exists()
    assert not (PROJECT_ROOT / "tests" / "test_scaffold.py").exists()

    forbidden = (
        "capsem-builder init",
        "capsem-builder new",
        "capsem-builder add",
        "capsem-builder mcp",
        "builder.scaffold",
        "scaffold.py",
        "init_guest_dir",
        "new_image",
        "scan_base_config",
        "add_package_set",
        "add_mcp_server",
    )
    checked_roots = [
        PROJECT_ROOT / "src" / "capsem" / "builder",
        PROJECT_ROOT / "docs" / "src" / "content" / "docs",
        PROJECT_ROOT / "skills",
        PROJECT_ROOT / ".github" / "workflows",
    ]
    offenders: list[str] = []
    for root in checked_roots:
        for path in sorted(root.rglob("*")):
            if not path.is_file() or "__pycache__" in path.parts:
                continue
            rel = path.relative_to(PROJECT_ROOT)
            try:
                text = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            for marker in forbidden:
                if marker in text:
                    offenders.append(f"{rel}: contains {marker!r}")
                    break
    assert offenders == [], "builder scaffold rail still exists:\n" + "\n".join(offenders)


def test_guest_init_exports_ca_bundle_for_runtime_and_login_shells() -> None:
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()
    expected = {
        "SSL_CERT_FILE": "/etc/ssl/certs/ca-certificates.crt",
        "REQUESTS_CA_BUNDLE": "/etc/ssl/certs/ca-certificates.crt",
        "NODE_EXTRA_CA_CERTS": "/etc/ssl/certs/ca-certificates.crt",
    }

    runtime_block = init.split("cat > /newroot/etc/profile.d/capsem.sh", maxsplit=1)[0]
    profile_block = init.split("cat > /newroot/etc/profile.d/capsem.sh", maxsplit=1)[1]

    for key, value in expected.items():
        export = f"export {key}={value}"
        assert export in runtime_block
        assert export in profile_block


def test_guest_init_exports_terminal_type_for_exec_and_doctor() -> None:
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()
    runtime_block = init.split("cat > /newroot/etc/profile.d/capsem.sh", maxsplit=1)[0]
    profile_block = init.split("cat > /newroot/etc/profile.d/capsem.sh", maxsplit=1)[1]

    assert "export TERM=xterm-256color" in runtime_block
    assert "export TERM=xterm-256color" in profile_block


def test_guest_init_repairs_overlay_root_traversal_for_unprivileged_tools() -> None:
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()

    chmod_pos = init.find("chmod 755 /newroot")
    chroot_chmod_pos = init.find("chroot /newroot /bin/chmod 755 /")
    launch_pos = init.find('chroot /newroot "$AGENT_PATH"')

    assert chmod_pos != -1, "init must make / traversable for _apt and tool users"
    assert chroot_chmod_pos != -1, "init must repair root mode as seen inside chroot"
    assert launch_pos != -1
    assert chmod_pos < launch_pos
    assert chroot_chmod_pos < launch_pos


def test_guest_init_publishes_rootfs_binaries_into_run_contract() -> None:
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()

    expected_rootfs_copies = {
        "capsem-net-proxy": "/newroot/usr/local/bin/capsem-net-proxy",
        "capsem-dns-proxy": "/newroot/usr/local/bin/capsem-dns-proxy",
        "capsem-pty-agent": "/newroot/usr/local/bin/capsem-pty-agent",
        "capsem-sysutil": "/newroot/usr/local/bin/capsem-sysutil",
    }
    for binary, rootfs_path in expected_rootfs_copies.items():
        assert rootfs_path in init
        assert f"cp {rootfs_path} /newroot/run/{binary}" in init
        assert f"chmod 555 /newroot/run/{binary}" in init

    assert "ln -sf /run/capsem-sysutil /newroot/sbin/shutdown" not in init
    assert "rm -f /newroot/sbin/shutdown" in init

    for link in (
        "/newroot/sbin/halt",
        "/newroot/sbin/poweroff",
        "/newroot/sbin/reboot",
        "/newroot/usr/local/bin/suspend",
    ):
        assert f"ln -sf /run/capsem-sysutil {link}" in init


def test_guest_runtime_doctor_package_probes_are_hermetic() -> None:
    source = (PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_runtimes.py").read_text()

    forbidden_fragments = [
        "pip install six",
        "uv pip install wheel",
        "uv pip install humanize",
        "npm install -g cowsay",
        "npm install lodash",
        "apt-get install -y -qq htop",
    ]
    for fragment in forbidden_fragments:
        assert fragment not in source

    assert "--no-index" in source
    assert "file:" in source
    assert "dpkg-deb --build" in source
    assert "--python /root/.venv/bin/python" in source


def test_guest_runtime_doctor_remote_apt_https_probe_is_release_gate() -> None:
    """Doctor must catch runtime apt HTTPS/CA breakage, not just local .deb installs."""
    source = (PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_runtimes.py").read_text()

    assert "def test_remote_apt_https_install_works" in source
    assert "apt-get " in source
    assert "update 2>&1" in source
    assert "https://deb.debian.org" in source
    assert "Certificate verification failed" in source
    assert "No system certificates available" in source
    assert "apt-get install -y -qq --no-install-recommends hello" in source
    assert "Hello, world!" in source


def test_capsem_init_recreates_user_local_ai_cli_shims() -> None:
    """Curl-installed AI CLIs must keep the user-local shim expected by doctors."""
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()

    assert "for cli in claude agy; do" in init
    assert 'ln -sf "/usr/local/bin/$cli" "/newroot/root/.local/bin/$cli"' in init
    assert 'chroot /newroot /bin/chmod 555 "/root/.local/bin/$cli"' in init


def test_capsem_init_keeps_etc_traversable_for_apt_sandbox() -> None:
    """The `_apt` sandbox must be able to read the TLS trust bundle under /etc."""
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()

    profile_seed_pos = init.find("projecting profile root seed")
    final_etc_chmod_pos = init.rfind("chmod 755 /newroot/etc")
    launch_pos = init.find('chroot /newroot "$AGENT_PATH"')

    assert "chmod 755 /newroot" in init
    assert profile_seed_pos != -1
    assert final_etc_chmod_pos != -1
    assert launch_pos != -1
    assert profile_seed_pos < final_etc_chmod_pos < launch_pos
    assert "TLS trust lives under `/etc/ssl/certs`" in init


def test_profile_roots_do_not_force_local_or_mock_model_providers() -> None:
    """Checked-in profile seeds must not silently select local/test model providers."""
    forbidden_fragments = (
        "127.0.0.1:11434",
        "localhost:11434",
        "CAPSEM_MOCK_SERVER",
        '"provider": "ollama"',
        '"baseUrl": "http://127.0.0.1:11434"',
    )
    for profile_dir in sorted((PROJECT_ROOT / "config" / "profiles").iterdir()):
        if not profile_dir.is_dir():
            continue
        config_path = profile_dir / "root" / "root" / ".codex" / "config.toml"
        if not config_path.exists():
            continue
        config = tomllib.loads(config_path.read_text())
        assert config.get("model_provider") not in {"local_ollama", "ollama"}, (
            f"{config_path} must not force a local Ollama model provider"
        )
        providers = config.get("model_providers") or {}
        assert "local_ollama" not in providers, (
            f"{config_path} must not declare a hidden local_ollama provider"
        )
        assert "ollama" not in providers, f"{config_path} must not declare a hidden ollama provider"
        root_dir = profile_dir / "root"
        for payload in sorted(root_dir.rglob("*")):
            if not payload.is_file():
                continue
            text = payload.read_text(errors="ignore")
            for fragment in forbidden_fragments:
                assert fragment not in text, f"{payload} contains {fragment!r}"


def test_guest_virtiofs_pip_probe_is_hermetic() -> None:
    source = (PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_virtiofs.py").read_text()

    assert "pip install --quiet cowsay" not in source
    assert "import cowsay" not in source
    assert "pip install --no-index" in source
    assert "ZipFile" in source
