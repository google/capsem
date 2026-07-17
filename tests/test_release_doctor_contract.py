"""Release doctor contract tests."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import re
import shutil
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


def test_macos_doctor_requires_live_rosetta_registration() -> None:
    source = _source_text("scripts/doctor-macos.sh")
    asset_gate = _recipe_block("test-assets:")

    assert "/proc/sys/fs/binfmt_misc/rosetta" in source
    assert "colima rosetta configured but not registered" in source
    assert "colima restart" in source
    assert "CROSS_PLATFORM=linux/amd64" in asset_gate
    assert "CROSS_PLATFORM=linux/arm64" in asset_gate
    assert 'docker run --rm --platform "$CROSS_PLATFORM"' in asset_gate
    assert "Docker cannot execute $CROSS_PLATFORM containers" in asset_gate


def test_host_sbom_zstd_dependency_has_local_and_exact_sha_parity() -> None:
    """The canonical gate must provision the same Debian archive decoder everywhere."""
    bootstrap = _source_text("bootstrap.sh")
    doctor = _source_text("scripts/doctor-common.sh")
    macos_doctor = _source_text("scripts/doctor-macos.sh")
    linux_doctor = _source_text("scripts/doctor-linux.sh")
    qualification = _workflow_text("release-qualification.yaml")

    assert 'confirm "zstd (Debian package/SBOM archive support, via brew)"' in bootstrap
    assert "brew install zstd" in bootstrap
    assert "for tool in cargo rustup node python3 uv pnpm sqlite3 git b3sum flock zstd" in doctor
    assert 'zstd)' in macos_doctor
    assert 'echo "brew install zstd"' in macos_doctor
    assert 'zstd)' in linux_doctor

    install = qualification.index("Install Linux full-gate system dependencies")
    canonical_gate = qualification.index("run: just test")
    assert "            zstd\n" in qualification[install:canonical_gate]


def test_linux_release_qualification_enables_arm64_for_canonical_asset_gate() -> None:
    workflow = _workflow_text("release-qualification.yaml")

    qemu = workflow.index("docker/setup-qemu-action@v3")
    canonical_gate = workflow.index("run: just test")
    assert "platforms: arm64" in workflow
    assert qemu < canonical_gate


def test_parallel_asset_gate_preserves_and_names_failed_architecture_logs() -> None:
    gate = _recipe_block("test-assets:")

    assert 'ARM64_BUILD_LOG="$TEST_ROOT/build-arm64.log"' in gate
    assert 'X86_64_BUILD_LOG="$TEST_ROOT/build-x86_64.log"' in gate
    assert 'tee "$ARM64_BUILD_LOG"' in gate
    assert 'tee "$X86_64_BUILD_LOG"' in gate
    assert 'build_arch_lane arm64 2>&1 | tee "$ARM64_BUILD_LOG"' in gate
    assert 'build_arch_lane x86_64 2>&1 | tee "$X86_64_BUILD_LOG"' in gate
    assert '> >(tee "$ARM64_BUILD_LOG")' not in gate
    assert '> >(tee "$X86_64_BUILD_LOG")' not in gate
    assert 'report_asset_lane_failure "arm64"' in gate
    assert 'report_asset_lane_failure "x86_64"' in gate
    assert 'tail -n 200 "$log"' in gate


def test_asset_gate_interrupt_cleanup_only_reaps_owned_mounts(tmp_path: Path) -> None:
    gate = _recipe_block("test-assets:")
    assert 'cleanup-docker-containers-by-mount.sh" "$TEST_ROOT"' in gate

    mount_root = tmp_path / "asset-root"
    mount_root.mkdir()
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    removals = tmp_path / "removals.log"
    docker = fake_bin / "docker"
    docker.write_text(
        """#!/bin/sh
set -eu
if [ "$1" = "ps" ]; then
    printf 'owned\\nforeign\\n'
elif [ "$1" = "inspect" ]; then
    id="${4}"
    if [ "$id" = "owned" ]; then
        printf '%s/lane/arm64\\n' "$FAKE_MOUNT_ROOT"
    else
        printf '/tmp/unrelated\\n'
    fi
elif [ "$1" = "rm" ]; then
    printf '%s\\n' "${3}" >> "$FAKE_REMOVALS"
else
    exit 97
fi
"""
    )
    docker.chmod(0o755)
    env = os.environ.copy()
    env.update(
        {
            "PATH": f"{fake_bin}:{env['PATH']}",
            "FAKE_MOUNT_ROOT": str(mount_root),
            "FAKE_REMOVALS": str(removals),
        }
    )

    result = subprocess.run(
        [
            "bash",
            str(PROJECT_ROOT / "scripts/cleanup-docker-containers-by-mount.sh"),
            str(mount_root),
        ],
        cwd=PROJECT_ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    assert removals.read_text().splitlines() == ["owned"]


def test_canonical_gate_builds_both_linux_release_architectures() -> None:
    canonical_gate = _recipe_block("test:")

    arm64 = canonical_gate.index("just cross-compile arm64")
    x86_64 = canonical_gate.index("just cross-compile x86_64")
    install = canonical_gate.rindex("just test-install")
    assert arm64 < x86_64 < install
    assert "just cross-compile\n" not in canonical_gate


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
    trigger = workflow.split("permissions:", maxsplit=1)[0]
    gate = _workflow_job_block("pr-gate")
    release_site_job = _workflow_job_block("release-site-build")

    assert "pull_request:" in workflow
    assert "push:" not in trigger
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
    assert "astral-sh/setup-uv@v5" in release_site_job
    assert "uv sync --frozen" in release_site_job
    assert "bash scripts/check-web-surface.sh release-site" in release_site_job


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
    assert "bash scripts/check-web-surface.sh docs" in docs_job
    assert "pages deploy" not in docs_job

    assert "cache-dependency-path: site/pnpm-lock.yaml" in site_job
    assert "cd site && pnpm install --frozen-lockfile" in site_job
    assert "bash scripts/check-web-surface.sh site" in site_job
    assert "pages deploy" not in site_job

    assert "pull_request:" not in docs_deploy
    assert "pull_request:" not in site_deploy
    assert "push:" in docs_deploy
    assert "push:" in site_deploy
    assert "branches: [main]" in docs_deploy
    assert "branches: [main]" in site_deploy

    assert "docs-build" in docs_ci
    assert "site-build" in docs_ci
    assert (
        "`pr-gate` depends on `docs-build`, `site-build`, and `release-site-build`" in docs_ci_text
    )


def test_macos_ci_installs_release_site_dependencies_before_integration() -> None:
    job = _workflow_job_block("test")
    install = "cd release-site && pnpm install --frozen-lockfile"
    integration = "Python integration tests (non-VM suites)"

    assert "frontend/pnpm-lock.yaml" in job
    assert "release-site/pnpm-lock.yaml" in job
    assert install in job
    assert job.index(install) < job.index(integration)


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

    binary_trigger = binary_workflow.split("\npermissions:", maxsplit=1)[0]
    assert "workflow_dispatch:" in binary_trigger
    assert "push:" not in binary_trigger
    assert "tag:" in binary_trigger
    assert "channel:" in binary_trigger
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
    trigger = workflow.split("\npermissions:", maxsplit=1)[0]

    assert "workflow_dispatch:" in workflow
    assert "default: stable" not in trigger
    assert "default: code" not in trigger
    assert "push:" not in workflow
    assert "tags:" not in workflow
    assert "deployments: write" in workflow
    assert "cloudflare-release-site-preflight:" in workflow
    assert "name: Cloudflare release site preflight" in workflow
    assert "Dry run: skipping Cloudflare Pages project preflight." in workflow
    assert "RELEASE_CHANNEL_PROJECT: release" in workflow
    assert "python scripts/check-cloudflare-pages-project.py" in workflow
    assert '--project "$RELEASE_CHANNEL_PROJECT"' in workflow
    assert "needs: cloudflare-release-site-preflight" in workflow
    assert workflow.index("cloudflare-release-site-preflight:") < workflow.index("build-assets:")
    assert workflow.index("Cloudflare release site preflight") < workflow.index("Build VM assets")
    assert "just build-kernel" in workflow
    assert "just build-rootfs" in workflow
    assert "cargo run -p capsem-admin -- manifest generate assets" in workflow
    assert "binary_version:" not in workflow
    assert "BINARY_VERSION" not in workflow
    assert '--version "$BINARY_VERSION"' not in workflow
    assert "scripts/build-complete-release-channel.py" in workflow
    assert '--channel-source "$CHANNEL=file://$PWD/assets/manifest.json"' in workflow
    assert "--allow-mirror-missing" in workflow
    assert "name: Preserve binary channel metadata" in workflow
    assert "scripts/preserve-binary-channel-metadata.py" in workflow
    assert "--manifest-path assets/manifest.json" in workflow
    assert "--manifest assets/manifest.json" not in workflow
    assert workflow.index("scripts/preserve-binary-channel-metadata.py") < workflow.index(
        "scripts/check-asset-release-delta.py"
    )
    assert workflow.index("scripts/preserve-binary-channel-metadata.py") < workflow.index(
        "scripts/build-complete-release-channel.py"
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
    assert (
        "if: ${{ inputs.dry_run == false && steps.asset-delta.outputs.asset_blobs_changed == 'true' }}"
        in workflow
    )
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
        normalized_text = " ".join(text.split())
        assert "metadata-only asset release changes" in text
        assert "deploy the release channel without republishing immutable" in normalized_text
        assert "blobs" in normalized_text
        assert "skip deployment only when current" in normalized_text
        assert "asset release metadata, and manifest policy are all unchanged" in normalized_text
        assert "manifest policy" in text
        assert "refresh_policy" in text
        assert "`binaries` metadata" in text or "per-binary inventory" in text
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
    assert '--project "$RELEASE_CHANNEL_PROJECT"' in workflow
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
    assert (
        "RELEASE_SITE_URL: ${{ inputs.release_site_url || 'https://release.capsem.org' }}"
        in workflow
    )
    assert "Validate deployed asset channel content" in workflow
    assert "uv run python scripts/check-release-site-contract.py" in workflow
    assert '--base-url "$RELEASE_SITE_URL"' in workflow
    assert "--channel stable" in workflow
    assert "--channel nightly" in workflow
    assert "--attempts 30" in workflow
    assert "--delay-seconds 20" in workflow
    assert workflow.index("cloudflare/wrangler-action@v3") < workflow.index(
        "Validate deployed asset channel content"
    )


def test_release_channel_deploy_runs_python_contract_validator_after_cloudflare_deploy() -> None:
    workflow = _workflow_text("release-channel.yaml")
    validator_step = workflow.split("- name: Validate deployed asset channel content", maxsplit=1)[
        1
    ].split("\n      - name:", maxsplit=1)[0]

    assert "Validate deployed asset channel content" in workflow
    assert "uv run python scripts/check-release-site-contract.py" in validator_step
    assert '--base-url "$RELEASE_SITE_URL"' in validator_step
    assert "--channel stable" in validator_step
    assert "--channel nightly" in validator_step
    assert 'CHANNEL_ARGS=(--channel "$CHANNEL")' in validator_step
    assert '"${CHANNEL_ARGS[@]}"' in validator_step
    assert "--attempts 30" in validator_step
    assert "--delay-seconds 20" in validator_step
    assert workflow.index("cloudflare/wrangler-action@v3") < workflow.index(
        "Validate deployed asset channel content"
    )


def test_release_channel_staging_workflow_exercises_reusable_deploy_without_release_builds() -> (
    None
):
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
    assert "bash scripts/check-web-surface.sh release-site-build" in workflow
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
        assert (
            "without invoking `build-assets`" in text or "without invoking VM asset builds" in text
        )


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
                    "health asset hash mismatch for /assets/releases/2030.0101.1/arm64-vmlinuz"
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


def test_release_site_contract_cli_validates_each_requested_channel(monkeypatch, capsys) -> None:
    validator = _release_site_contract_module()
    checked_channels: list[str] = []

    class FakeChecker:
        BLAKE3_IMPORT_ERROR = None

        @staticmethod
        def check_release_site_dns(release_site: str):
            assert release_site == "https://release.capsem.org"
            return SimpleNamespace(ok=True, name="release.capsem.org DNS", detail="ok")

        @staticmethod
        def check_release_site_contract(release_site: str, channel: str):
            assert release_site == "https://release.capsem.org"
            checked_channels.append(channel)
            return SimpleNamespace(
                ok=True,
                name="release.capsem.org contract",
                detail=f"{channel} ok",
            )

    monkeypatch.setattr(validator, "load_readiness_checker", lambda: FakeChecker)
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "check-release-site-contract.py",
            "--base-url",
            "https://release.capsem.org",
            "--channel",
            "stable",
            "--channel",
            "nightly",
            "--attempts",
            "1",
            "--delay-seconds",
            "0",
        ],
    )

    exit_code = validator.main()

    captured = capsys.readouterr()
    assert exit_code == 0
    assert checked_channels == ["stable", "nightly"]
    assert "stable release-channel contract passed" in captured.out
    assert "nightly release-channel contract passed" in captured.out


def test_release_site_contract_cli_retries_requested_channels_as_a_set(monkeypatch, capsys) -> None:
    validator = _release_site_contract_module()
    checks: list[str] = []
    sleep_calls: list[float] = []

    class FakeChecker:
        BLAKE3_IMPORT_ERROR = None

        @staticmethod
        def check_release_site_dns(release_site: str):
            assert release_site == "https://release.capsem.org"
            return SimpleNamespace(ok=True, name="release.capsem.org DNS", detail="ok")

        @staticmethod
        def check_release_site_contract(release_site: str, channel: str):
            assert release_site == "https://release.capsem.org"
            checks.append(channel)
            if checks == ["stable"]:
                return SimpleNamespace(
                    ok=False,
                    name="release.capsem.org contract",
                    detail="stable package page still serving previous deploy",
                )
            return SimpleNamespace(
                ok=True,
                name="release.capsem.org contract",
                detail=f"{channel} ok",
            )

    monkeypatch.setattr(validator, "load_readiness_checker", lambda: FakeChecker)
    monkeypatch.setattr(validator.time, "sleep", sleep_calls.append)
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "check-release-site-contract.py",
            "--base-url",
            "https://release.capsem.org",
            "--channel",
            "stable",
            "--channel",
            "nightly",
            "--attempts",
            "2",
            "--delay-seconds",
            "7",
        ],
    )

    exit_code = validator.main()

    captured = capsys.readouterr()
    assert exit_code == 0
    assert checks == ["stable", "nightly", "stable", "nightly"]
    assert sleep_calls == [7]
    assert "attempt 1/2: stable" in captured.err
    assert "stable release-channel contract passed" in captured.out
    assert "nightly release-channel contract passed" in captured.out


def test_release_site_contract_retries_clear_cached_remote_fetches(monkeypatch, capsys) -> None:
    validator = _release_site_contract_module()
    cache_clears = 0
    checks: list[tuple[int, str]] = []
    sleep_calls: list[float] = []

    class CountingCache(dict):
        def clear(self) -> None:
            nonlocal cache_clears
            cache_clears += 1
            super().clear()

    class FakeChecker:
        BLAKE3_IMPORT_ERROR = None
        _FETCH_BYTES_CACHE = CountingCache({"stale-nightly-manifest": b"old"})

        @staticmethod
        def check_release_site_dns(release_site: str):
            assert release_site == "https://release.capsem.org"
            return SimpleNamespace(ok=True, name="release.capsem.org DNS", detail="ok")

        @staticmethod
        def check_release_site_contract(release_site: str, channel: str):
            assert release_site == "https://release.capsem.org"
            checks.append((cache_clears, channel))
            if channel == "nightly" and cache_clears == 1:
                FakeChecker._FETCH_BYTES_CACHE["nightly-manifest"] = b"stale"
                return SimpleNamespace(
                    ok=False,
                    name="release.capsem.org contract",
                    detail="channel manifest SHA-256 mismatch",
                )
            assert "nightly-manifest" not in FakeChecker._FETCH_BYTES_CACHE
            return SimpleNamespace(
                ok=True,
                name="release.capsem.org contract",
                detail=f"{channel} ok",
            )

    monkeypatch.setattr(validator.time, "sleep", sleep_calls.append)

    exit_code = validator.validate_release_channels(
        release_site="https://release.capsem.org",
        channels=["stable", "nightly"],
        attempts=2,
        delay_seconds=3,
        checker=FakeChecker,
    )

    captured = capsys.readouterr()
    assert exit_code == 0
    assert cache_clears == 2
    assert checks == [
        (1, "stable"),
        (1, "nightly"),
        (2, "stable"),
        (2, "nightly"),
    ]
    assert sleep_calls == [3]
    assert "attempt 1/2: nightly" in captured.err
    assert "stable release-channel contract passed" in captured.out
    assert "nightly release-channel contract passed" in captured.out


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
        assert "BLAKE3/SHA-256" in text
        assert "cache headers" in text_lower
        assert "rather than only checking that files exist" in text_lower
        assert "before running a live binary or" in text_lower
        assert "channel deploy" in text_lower


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
    script = _source_text("scripts/check-remote-release-readiness.py")
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())

    assert "astral-sh/setup-uv@v5" in workflow
    assert "uv run python scripts/check-release-site-contract.py" in workflow
    assert "import hashlib" in script
    assert "import blake3" in script
    assert "def fetch_and_verify_evidence_artifact" in script
    assert 'site, sbom, "sha256", "host SBOM evidence", "spdx"' in script
    assert 'site, obom, "blake3", "VM OBOM evidence", "cyclonedx"' in script
    assert "data = fetch_bytes(url)" in script
    assert "health evidence host_sboms missing for published binary files" in script
    assert "health evidence vm_oboms missing for published VM assets" in script
    assert "health evidence attestations missing for published artifacts" in script
    assert "attestation_predicate_evidence_urls" in script
    assert "attestation predicate_url {predicate_url} missing from {predicate_label}" in script
    assert "attestation subject {subject} missing from published file lists" in script
    assert "resolves published host SBOM and VM OBOM evidence artifacts from the graph" in docs_text
    assert "verifies their advertised hashes and sizes" in docs_text
    assert "validates their SPDX 2.3 or CycloneDX document shape" in docs_text
    assert "validates attestation subjects and predicate URLs" in docs_text
    assert "Profile image attestations are incomplete unless" in docs_text
    assert "`github_attestations_vm_assets`" in docs_text
    assert "`predicate_url` points at the published VM OBOM evidence" in docs_text


def test_docs_preserve_vm_obom_attestation_predicate_contract() -> None:
    docs_text = " ".join(_source_text("docs/src/content/docs/development/ci.md").split())

    assert "Profile image attestations are incomplete unless" in docs_text
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

    script = _source_text("scripts/check-remote-release-readiness.py")
    assert "uv run python scripts/check-release-site-contract.py" in workflow
    assert "def check_release_cache_headers" in script
    assert '("no-cache", "must-revalidate")' in script
    assert '("public", "max-age=31536000", "immutable")' in script

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


def test_linux_doctor_installs_musl_c_toolchain_before_building_assets() -> None:
    doctor = _source_text("scripts/doctor-common.sh")
    linux = _source_text("scripts/doctor-linux.sh")

    assert '_reg linux-musl-tools "_doctor_install_linux_musl_tools"' in doctor
    assert doctor.index("_reg linux-musl-tools") < doctor.index("_reg build-assets")
    assert "check_linux_musl_toolchain" in doctor
    assert 'section "C Toolchain"' in linux
    assert "linux_musl_toolchain_available" in linux
    assert "command -v musl-gcc" in linux
    assert "command -v x86_64-linux-musl-gcc" not in linux
    assert "apt-get install -y musl-tools" in linux
    assert "dnf install -y musl-gcc" in linux


def test_linux_doctor_accepts_native_musl_gcc_without_x86_cross_compiler(
    tmp_path: Path,
) -> None:
    musl_gcc = tmp_path / "musl-gcc"
    musl_gcc.write_text("#!/bin/sh\nexit 0\n")
    musl_gcc.chmod(0o755)

    result = subprocess.run(
        [
            "/bin/bash",
            "-c",
            """
            source scripts/doctor-linux.sh
            section() { :; }
            pass() { printf 'PASS:%s\\n' "$1"; }
            fixable() { printf 'FIXABLE:%s\\n' "$*"; }
            check_linux_musl_toolchain
            """,
        ],
        cwd=PROJECT_ROOT,
        env={"PATH": str(tmp_path)},
        capture_output=True,
        text=True,
        check=True,
    )

    assert result.stdout == "PASS:musl-gcc\n"


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
    assert (
        "keeps blocked profile dashboard tracks visible beside available asset tracks" in frontend
    )
    assert "Binary, VM assets available" in frontend
    assert "VM assets available for future sessions" in frontend

    assert (
        "applies binary/profile and asset update actions through typed confirmed bodies"
        in frontend_api
    )
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

    for (
        workflow_name,
        directory,
        ci_job,
        project_name,
        smoke_name,
        site_url,
        failure,
    ) in expectations:
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
        assert f"bash scripts/check-web-surface.sh {directory}" in ci_block
        assert (
            "needs: [test-linux, test, test-install, docs-build, site-build, release-site-build]"
            in ci_workflow
        )
        assert f"cd {directory} && pnpm install --frozen-lockfile" in workflow
        assert f"bash scripts/check-web-surface.sh {directory}" in workflow
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
    qualification = _workflow_text("release-qualification.yaml")
    create_release = _workflow_job_block("create-release", "release.yaml")
    assemble_channel = _workflow_job_block("assemble-release-channel", "release.yaml")
    trigger = workflow.split("\npermissions:", maxsplit=1)[0]

    assert "workflow_dispatch:" in trigger
    assert "tag:" in trigger
    assert "channel:" in trigger
    assert "type: choice" in trigger
    assert "options:" in trigger
    assert "- stable" in trigger
    assert "- nightly" in trigger
    assert "run-name: Release ${{ inputs.channel }} ${{ inputs.tag }}" in workflow
    assert "deployments: write" in workflow
    assert "push:" not in trigger
    assert "pull_request:" not in trigger
    assert "branches:" not in trigger
    assert "group: binary-release-channel" in workflow
    assert "cancel-in-progress: false" in workflow
    assert "RELEASE_TAG: ${{ inputs.tag }}" in workflow
    assert "RELEASE_CHANNEL: ${{ inputs.channel }}" in workflow
    assert (
        "ASSET_MANIFEST_URL: https://release.capsem.org/assets/${{ inputs.channel }}/manifest.json"
        in workflow
    )
    assert "Verify immutable dispatch tag and channel" in workflow
    assert 'test "$GITHUB_REF_TYPE" = tag' in workflow
    assert 'test "$GITHUB_REF_NAME" = "$RELEASE_TAG"' in workflow
    assert "BINARY_RELEASE_CHANNELS" not in workflow
    assert "  build-assets:" not in workflow
    assert "vm-assets-" not in workflow
    assert "assets/current" not in workflow
    assert """echo '{"releases":{}}'""" not in workflow
    assert "Complete canonical release gate (just test)" in qualification
    assert "run: just test" not in workflow
    assert "scripts/check-release-qualification.py" in workflow
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
    for logical_name in (
        "vmlinuz",
        "initrd.img",
        "rootfs.erofs",
        "obom.cdx.json",
        "software-inventory.json",
    ):
        assert f"release-artifacts/{logical_name}" not in create_release
        assert f"release-artifacts/*{logical_name}" not in create_release
    assert "release-artifacts/*.pkg" in create_release
    assert "release-artifacts/*.deb" in create_release
    assert "release-artifacts/capsem-sbom.spdx.json" in create_release
    assert 'gh release create "$RELEASE_TAG"' in create_release
    assert '[ -f "$deb" ] && gh release upload "$RELEASE_TAG" "$deb"' in create_release
    assert "target/binary-channel/$RELEASE_CHANNEL/manifest.json" in assemble_channel
    assert "target/binary-channel/$RELEASE_CHANNEL/manifest.before.json" in assemble_channel
    assert "https://release.capsem.org/assets/$channel/manifest.json" in assemble_channel
    assert "for channel in stable nightly" in assemble_channel
    record_step = assemble_channel.split(
        "- name: Record binary release metadata in selected channel manifest", maxsplit=1
    )[1].split("- name: Prove binary lane did not change VM assets", maxsplit=1)[0]
    assert "target/binary-channel/$RELEASE_CHANNEL/manifest.json" in record_step
    assert "for channel in" not in record_step
    build_channels = assemble_channel.split(
        "- name: Build complete release channels with existing VM assets", maxsplit=1
    )[1].split("- uses: actions/upload-artifact", maxsplit=1)[0]
    assert "generated_at=\"$(date -u +'%Y-%m-%dT%H:%M:%SZ')\"" in build_channels
    assert '--generated-at "$generated_at"' in build_channels
    assert "scripts/build-complete-release-channel.py" in build_channels
    assert '--channel-source "stable=file://$PWD/target/binary-channel/stable/manifest.json"' in build_channels
    assert '--channel-source "nightly=file://$PWD/target/binary-channel/nightly/manifest.json"' in build_channels
    assert '--primary-channel "$RELEASE_CHANNEL"' in build_channels
    assert build_channels.index('generated_at="$(date -u') < build_channels.index(
        "scripts/build-complete-release-channel.py"
    )
    assert "Prove binary lane did not change VM assets" in assemble_channel
    assert "binary release changed VM asset metadata" in assemble_channel
    assert assemble_channel.index("Fetch current asset channel manifests") < assemble_channel.index(
        "Record binary release metadata in selected channel manifest"
    )
    assert assemble_channel.index("Record binary release metadata in selected channel manifest") < (
        assemble_channel.index("Build complete release channels with existing VM assets")
    )
    assert "- name: Build release site pages" not in assemble_channel
    assert "- name: Check binary-updated release channels" not in assemble_channel


def test_binary_release_channel_assembly_preflights_canonical_artifacts() -> None:
    assemble_channel = _workflow_job_block("assemble-release-channel", "release.yaml")

    assert "Verify binary channel artifacts" in assemble_channel
    assert "release-artifacts/capsem-sbom.spdx.json" in assemble_channel
    assert "::error::release-artifacts/capsem-sbom.spdx.json missing" in assemble_channel
    assert "release-artifacts/*.pkg" in assemble_channel
    assert "release-artifacts/*.deb" in assemble_channel
    assert "::error::no installable host package artifact found" in assemble_channel
    assert assemble_channel.index("Verify binary channel artifacts") < assemble_channel.index(
        "Fetch current asset channel manifests"
    )
    assert assemble_channel.index("Verify binary channel artifacts") < assemble_channel.index(
        "Record binary release metadata in selected channel manifest"
    )


def test_binary_release_staging_dry_run_is_separate_from_tag_release() -> None:
    workflow = _workflow_text("release-binary-staging.yaml")
    real_release = _workflow_text("release.yaml")
    assemble_channel = _workflow_job_block(
        "assemble-binary-channel",
        "release-binary-staging.yaml",
    )

    real_trigger = real_release.split("\npermissions:", maxsplit=1)[0]
    assert "workflow_dispatch:" in real_trigger
    assert "push:" not in real_trigger
    assert "tag:" in real_trigger
    assert "channel:" in real_trigger

    assert "workflow_dispatch:" in workflow
    assert "asset_channel:" in workflow
    assert "description: Existing VM asset channel to use as the staging source." in workflow
    assert "default: stable" not in workflow.split("\npermissions:", maxsplit=1)[0]
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
    for logical_name in (
        "vmlinuz",
        "initrd.img",
        "rootfs.erofs",
        "obom.cdx.json",
        "software-inventory.json",
    ):
        assert f"release-artifacts/{logical_name}" not in workflow
        assert f"release-artifacts/*{logical_name}" not in workflow

    assert (
        "ASSET_MANIFEST_URL: https://release.capsem.org/assets/${{ inputs.asset_channel }}/manifest.json"
        in workflow
    )
    assert 'case "$ASSET_CHANNEL" in stable|nightly)' in assemble_channel
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
    docs_text = " ".join(docs.split())
    assert "binary freshness comes from the selected manifest in the release graph" in docs_text
    assert "releases do not rebuild or upload profile images, and they do not publish" in docs_text
    assert (
        "`latest.json`; binary freshness comes from the selected manifest in the release graph"
        in docs_text
    )
    assert "`latest.json` is absent in the current release rail" in release_skill
    assert "Do not make release creation depend on `latest.json`" in release_skill


def test_binary_release_channel_policy_supports_fast_nightly_and_weekly_stable() -> None:
    workflow = _workflow_text("release.yaml")
    docs = _source_text("docs/src/content/docs/development/ci.md")
    release_skill = _source_text("skills/release-process/SKILL.md")

    trigger = workflow.split("\npermissions:", maxsplit=1)[0]
    assert "workflow_dispatch:" in trigger
    assert "- stable" in trigger
    assert "- nightly" in trigger
    assert "RELEASE_CHANNEL: ${{ inputs.channel }}" in workflow
    assert "group: binary-release-channel" in workflow
    assert "Prove binary lane did not change VM assets" in workflow
    docs_text = " ".join(docs.split())
    release_skill_text = " ".join(release_skill.split())
    assert "nightly can move daily while stable is promoted on the weekly cadence" in docs_text
    assert "nightly is the daily binary iteration channel" in release_skill_text.lower()
    assert "stable is promoted on the weekly cadence" in release_skill_text


def test_untagged_release_candidate_runs_complete_canonical_gate_in_ci() -> None:
    workflow = _workflow_text("release.yaml")
    qualification_workflow = _workflow_text("release-qualification.yaml")
    gate = _workflow_job_block("qualification", "release-qualification.yaml")
    agents = _source_text("AGENTS.md")
    testing_skill = _source_text("skills/dev-testing/SKILL.md")
    release_skill = _source_text("skills/release-process/SKILL.md")

    assert "run-name: Qualify release ${{ inputs.channel }} ${{ inputs.sha }}" in qualification_workflow
    assert "CAPSEM_INSTALL_MANIFEST_URL: https://release.capsem.org/assets/${{ inputs.channel }}/manifest.json" in qualification_workflow
    assert "CAPSEM_INSTALL_CHANNEL: ${{ inputs.channel }}" in qualification_workflow
    assert "sha:" in qualification_workflow
    assert "ref: ${{ inputs.sha }}" in qualification_workflow
    assert '[[ "$EXPECTED_SHA" =~ ^[0-9a-f]{40}$ ]]' in gate
    assert 'test "$GITHUB_REF_TYPE" = branch' in gate
    assert 'test "$GITHUB_REF_NAME" = main' in gate
    assert 'test "$GITHUB_SHA" = "$EXPECTED_SHA"' in gate
    assert 'test "$(git rev-parse HEAD)" = "$EXPECTED_SHA"' in gate
    assert "contents: read" in qualification_workflow
    assert "contents: write" not in qualification_workflow
    assert "gh release" not in qualification_workflow
    assert 'git tag "$TAG"' not in qualification_workflow
    assert "matrix.os" not in gate
    assert "TEMPORARILY DISABLED: macOS full gate" in gate
    assert "Restore a parallel macOS `just" in gate
    assert "fromJSON(matrix.runner)" not in gate
    assert "runner: macos-14" not in gate
    assert "runs-on: ubuntu-24.04" in gate
    assert "strategy:" not in gate
    assert "extractions/setup-just@v3" in gate
    assert "Install Linux full-gate system dependencies" in gate
    assert "libglib2.0-dev" in gate
    assert "libwebkit2gtk-4.1-dev" in gate
    assert "musl-tools" in gate
    public_manifest_gate = "Validate public install manifest with candidate runtime"
    assert public_manifest_gate in gate
    assert "cargo build -p capsem" in gate
    assert 'cp target/debug/capsem "$RUNTIME_HOME/.capsem/bin/capsem"' in gate
    assert '"manifest_url": os.environ["CAPSEM_INSTALL_MANIFEST_URL"]' in gate
    assert '"channel": os.environ["CAPSEM_INSTALL_CHANNEL"]' in gate
    assert "CAPSEM_RELEASE_MANIFEST_URL=" not in gate
    assert '"$RUNTIME_HOME/.capsem/bin/capsem" update --check' in gate
    assert gate.index(public_manifest_gate) < gate.index(
        "Complete canonical release gate (just test)"
    )
    assert gate.index("Install Linux full-gate system dependencies") < gate.index(
        "Complete canonical release gate (just test)"
    )
    assert "Enable KVM" in gate
    assert "udevadm" not in gate
    assert "test -c /dev/kvm" in gate
    assert "sudo chmod 0666 /dev/kvm" in gate
    assert "test -r /dev/kvm -a -w /dev/kvm" in gate
    assert "sudo modprobe vhost_vsock" in gate
    assert "test -c /dev/vhost-vsock" in gate
    assert "sudo chmod 0666 /dev/vhost-vsock" in gate
    assert "test -r /dev/vhost-vsock -a -w /dev/vhost-vsock" in gate
    assert "Start Docker on macOS" not in gate
    assert "just test" in gate
    assert qualification_workflow.count("run: just test") == 1
    assert "run: just test" not in workflow
    assert "cargo llvm-cov --workspace --bins --no-cfg-coverage" not in gate
    assert "Create stub v2 asset manifest for unit tests" not in gate
    assert "needs: preflight" in _workflow_job_block("build-app-macos", "release.yaml")
    assert "needs: preflight" in _workflow_job_block("build-app-linux", "release.yaml")
    preflight = _workflow_job_block("preflight", "release.yaml")
    assert "Verify exact commit passed remote qualification" in preflight
    assert 'scripts/check-release-qualification.py --sha "$GITHUB_SHA" --channel "$RELEASE_CHANNEL"' in preflight
    assert "fetch-depth: 0" in preflight
    assert "git fetch origin main" in preflight
    assert 'git merge-base --is-ancestor "$GITHUB_SHA" origin/main' in preflight
    assert "temporary GitHub-hosted exception" in agents
    assert "Temporary hosted-CI exception" in testing_skill
    assert "Temporary hosted-CI exception" in release_skill


def test_clean_build_pins_sse_stream_api() -> None:
    workspace = _source_text("Cargo.toml")
    server_manager = _source_text("crates/capsem-core/src/mcp/server_manager.rs")

    assert 'sse-stream = "=0.2.4"' in workspace
    assert "SseStream::from_bytes_stream" in server_manager
    assert "SseStream::from_byte_stream" not in server_manager


def test_binary_release_installs_exact_artifacts_before_publication() -> None:
    workflow = _workflow_text("release.yaml")
    macos = _workflow_job_block("build-app-macos", "release.yaml")
    linux = _workflow_job_block("build-app-linux", "release.yaml")
    create_release = _workflow_job_block("create-release", "release.yaml")

    assert "  test-install:" not in workflow
    assert "needs: preflight" in macos
    assert "needs: preflight" in linux
    assert "continue-on-error: true" not in linux
    assert "Install exact notarized package" in macos
    assert "Verify exact notarized package identity and Gatekeeper acceptance" in macos
    assert 'pkgutil --check-signature "packages/Capsem-$VERSION.pkg"' in macos
    assert 'spctl -a -vv -t install "packages/Capsem-$VERSION.pkg"' in macos
    assert 'sudo /usr/sbin/installer -pkg "packages/Capsem-$VERSION.pkg" -target /' in macos
    assert 'test -x "$HOME/.capsem/bin/capsem"' in macos
    assert '"$HOME/.capsem/bin/capsem" --version | grep -F "$VERSION"' in macos
    assert 'test -d "/Applications/Capsem.app"' in macos
    assert (
        "for bin in capsem capsem-admin capsem-gateway capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-process capsem-service capsem-tray capsem-tui"
        in macos
    )
    assert 'grep -F "Installed: true" /tmp/capsem-status.txt' in macos
    assert 'grep -F "Running:   true" /tmp/capsem-status.txt' in macos
    assert "scripts/verify-installed-release.py" in macos
    assert (
        macos.index("Notarize and staple .pkg")
        < macos.index("Verify exact notarized package identity and Gatekeeper acceptance")
        < macos.index("Install exact notarized package")
        < macos.index("Collect macOS artifacts")
    )
    assert "Post-install full gate (just test)" not in macos
    assert "run: just test" not in macos
    assert "Install exact release deb" in linux
    assert "sudo dpkg -i target/release/bundle/deb/*.deb" in linux
    assert "test -x /usr/bin/capsem" in linux
    assert '/usr/bin/capsem --version | grep -F "$VERSION"' in linux
    assert (
        "for bin in capsem capsem-admin capsem-app capsem-gateway capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-process capsem-service capsem-tray capsem-tui"
        in linux
    )
    assert "dpkg-query -W -f='${Version}' capsem | grep -Fx \"$VERSION\"" in linux
    assert 'grep -F "Installed: true" /tmp/capsem-status.txt' in linux
    assert 'grep -F "Running:   true" /tmp/capsem-status.txt' in linux
    assert "scripts/verify-installed-release.py" in linux
    assert "Enable KVM for exact-package VM proof" in linux
    assert "test -r /dev/kvm -a -w /dev/kvm" in linux
    assert "scripts/prove-installed-shell.py" in linux
    assert "CAPSEM_EXACT_PACKAGE_SHELL_OK" in linux
    assert "/usr/bin/capsem run" not in linux
    assert (
        linux.index("Repack .deb with companion binaries")
        < linux.index("Install exact release deb")
        < linux.index("Collect Linux artifacts")
    )
    assert "Post-install full gate (just test)" not in linux
    assert "run: just test" not in linux
    assert "needs: [build-app-macos, build-app-linux]" in create_release
    assert "test-install" not in create_release
    assert "continue-on-error: true" not in create_release


def test_install_gate_rebuilds_missing_base_image_before_derived_image() -> None:
    recipe = _recipe_block("test-install:")

    base_check = "docker image inspect capsem-host-builder:latest"
    base_build = "just build-host-image"
    derived_build = 'docker build -t "$IMAGE" -f docker/Dockerfile.install-test .'
    assert base_check in recipe
    assert base_build in recipe
    assert recipe.index(base_check) < recipe.index(base_build) < recipe.index(derived_build)


def test_release_skill_requires_ci_and_local_mac_installer_outcome_proof() -> None:
    release_skill = _source_text("skills/release-process/SKILL.md")

    assert "Installer outcome gate" in release_skill
    assert "exact publishable `.pkg`" in release_skill
    assert "exact publishable `.deb`" in release_skill
    assert "Linux CI installed-product proof" in release_skill
    assert "macOS CI exact-package proof" in release_skill
    assert "Final local macOS installed-product proof" in release_skill
    assert "sudo /usr/sbin/installer -pkg" in release_skill
    assert "capsem shell" in release_skill
    assert "manual GUI click-through" in release_skill
    assert "does not count as release proof" in release_skill
    assert "forward-only release" in release_skill
    assert "Do not call the release complete" in release_skill
    assert "scripts/verify-installed-release.py" in release_skill
    assert "byte-for-byte" in release_skill
    assert "all manifest-declared profiles ready" in release_skill


def test_release_skill_requires_exact_manifest_single_metadata_and_shared_status_contract() -> None:
    release_skill = _source_text("skills/release-process/SKILL.md")
    installation_skill = _source_text("skills/dev-installation/SKILL.md")

    for source in (release_skill, installation_skill):
        normalized = " ".join(source.split())
        assert "assets/manifest.json" in source
        assert "assets/manifest-metadata.json" in source
        assert "capsem.manifest_metadata.v1" in source
        assert "GET /system/status" in source
        assert "in memory" in normalized or "in-memory" in normalized
    release_normalized = " ".join(release_skill.split())
    assert "exact verified manifest" in release_normalized
    assert "must not rewrite it into a reduced runtime schema" in release_normalized
    assert "Do not create a separate origin file" in release_normalized
    assert "the UI must not synthesize publication state" in release_normalized


def test_release_recipe_dispatches_one_parameterized_workflow() -> None:
    recipe = _recipe_block("release")

    assert 'release tag="" channel="stable":' in recipe
    assert 'case "$CHANNEL" in' in recipe
    assert "stable|nightly" in recipe
    assert 'gh workflow run release.yaml --ref "$TAG"' in recipe
    assert '-f "tag=$TAG"' in recipe
    assert '-f "channel=$CHANNEL"' in recipe
    assert 'RUN_TITLE="Release $CHANNEL $TAG"' in recipe
    assert "displayTitle" in recipe
    assert "headBranch" not in recipe
    assert 'LOCAL_TAG_SHA=$(git rev-parse "$TAG^{commit}")' in recipe
    assert 'REMOTE_TAG_SHA=$(git ls-remote --tags origin' in recipe
    assert 'test "$LOCAL_TAG_SHA" = "$REMOTE_TAG_SHA"' in recipe
    assert recipe.index('test "$LOCAL_TAG_SHA" = "$REMOTE_TAG_SHA"') < recipe.index(
        'scripts/check-release-qualification.py --sha "$LOCAL_TAG_SHA" --channel "$CHANNEL"'
    )


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
    assert "apt-get install --yes --allow-downgrades {cached}" in install_tests
    assert "and print the tested package-manager apply command (`sudo" not in install_skill
    assert (
        "downloads verified binary installers, prints the package-manager apply command,"
        not in (architecture_skill)
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
    current_binary = "1.4.0"
    current_assets = "2030.0101.1"
    profile_revision = "profiles-2030.0101.1"
    manifest_path = "/assets/stable/manifest.json"
    asset_base = "/assets/releases"

    def digest(data: bytes) -> dict[str, str]:
        return {
            "sha256": hashlib.sha256(data).hexdigest(),
            "blake3": checker.blake3.blake3(data).hexdigest(),
        }

    artifacts = {
        "/packages/Capsem-1.4.0.pkg": b"package bytes\n",
        "/packages/Capsem-1.4.0.spdx.json": b'{"spdxVersion":"SPDX-2.3","files":[]}\n',
        f"/profiles/releases/{profile_revision}/co-work/arm64/profile.toml": b'id = "co-work"\n',
        f"/profiles/releases/{profile_revision}/co-work/arm64/software-inventory.json": (
            b'{"schema":"capsem.profile_software_inventory.v1","packages":[]}\n'
        ),
        f"{asset_base}/{current_assets}/arm64-vmlinuz": b"kernel bytes\n",
        f"{asset_base}/{current_assets}/arm64-initrd.img": b"initrd bytes\n",
        f"{asset_base}/{current_assets}/arm64-rootfs.erofs": b"rootfs bytes\n",
        f"{asset_base}/{current_assets}/arm64-obom.cdx.json": (
            b'{"bomFormat":"CycloneDX","components":[]}\n'
        ),
    }

    def file_record(kind: str, name: str, url: str) -> dict[str, object]:
        data = artifacts[url]
        return {
            "kind": kind,
            "name": name,
            "url": url,
            "status": "current",
            "bytes": len(data),
            "digest": digest(data),
        }

    package_url = "/packages/Capsem-1.4.0.pkg"
    package_sbom_url = "/packages/Capsem-1.4.0.spdx.json"
    config_url = f"/profiles/releases/{profile_revision}/co-work/arm64/profile.toml"
    software_inventory_url = (
        f"/profiles/releases/{profile_revision}/co-work/arm64/software-inventory.json"
    )
    obom_url = f"{asset_base}/{current_assets}/arm64-obom.cdx.json"

    manifest = {
        "version": current_binary,
        "status": "current",
        "packages": [
            {
                "id": "capsem-1-4-0-pkg",
                "kind": "macos_pkg",
                "platform": "macos",
                "architecture": "arm64",
                "name": "Capsem-1.4.0.pkg",
                "version": current_binary,
                "url": package_url,
                "bytes": len(artifacts[package_url]),
                "digest": digest(artifacts[package_url]),
                "evidence": [
                    {
                        "kind": "sbom",
                        "url": package_sbom_url,
                        "bytes": len(artifacts[package_sbom_url]),
                        "digest": digest(artifacts[package_sbom_url]),
                    }
                ],
                "binaries": [
                    {
                        "name": "capsem-app",
                        "version": current_binary,
                        "description": "Capsem desktop application executable",
                        "installed_path": "/Applications/Capsem.app/Contents/MacOS/capsem-app",
                        "architecture": "arm64",
                        "platform": "macos",
                        "bytes": 12,
                        "digest": digest(b"capsem-app binary\n"),
                        "sbom_component_ref": "SPDXRef-File-capsem-app",
                    }
                ],
            }
        ],
        "profiles": {
            "co-work": {
                "id": "co-work",
                "name": "Co-work",
                "description": "Collaborative agent profile.",
                "revision": profile_revision,
                "min_capsem_version": current_binary,
                "architectures": [
                    {
                        "architecture": "arm64",
                        "software": [
                            {
                                "name": "@openai/codex",
                                "version": "0.142.5",
                                "source": "npm",
                                "architecture": "arm64",
                                "evidence": software_inventory_url,
                                "digest": digest(b"codex software row\n"),
                            }
                        ],
                        "config": [
                            {
                                "kind": "profile",
                                "path": "profiles/co-work/profile.toml",
                                "url": config_url,
                                "status": "current",
                                "bytes": len(artifacts[config_url]),
                                "digest": digest(artifacts[config_url]),
                            }
                        ],
                        "images": [
                            file_record(
                                "kernel",
                                "vmlinuz",
                                f"{asset_base}/{current_assets}/arm64-vmlinuz",
                            ),
                            file_record(
                                "initrd",
                                "initrd.img",
                                f"{asset_base}/{current_assets}/arm64-initrd.img",
                            ),
                            file_record(
                                "rootfs",
                                "rootfs.erofs",
                                f"{asset_base}/{current_assets}/arm64-rootfs.erofs",
                            ),
                        ],
                        "evidence": [
                            {
                                "kind": "software_inventory",
                                "url": software_inventory_url,
                                "status": "current",
                                "bytes": len(artifacts[software_inventory_url]),
                                "digest": digest(artifacts[software_inventory_url]),
                            },
                            {
                                "kind": "obom",
                                "url": obom_url,
                                "status": "current",
                                "bytes": len(artifacts[obom_url]),
                                "digest": digest(artifacts[obom_url]),
                            },
                        ],
                    }
                ],
            }
        },
    }
    if catalog_mutator is not None:
        catalog_mutator(manifest["profiles"]["co-work"])
    if manifest_mutator is not None:
        manifest_mutator(manifest)
    manifest_bytes = (json.dumps(manifest, sort_keys=True) + "\n").encode()
    manifest_digest = digest(manifest_bytes)

    channels = {
        "version": 1,
        "generated_at": "2030-01-01T00:00:00Z",
        "channels": {
            channel: {
                "label": "Stable",
                "description": "Recommended release channel.",
                "manifests": [
                    {
                        "version": current_binary,
                        "revision": current_binary,
                        "status": "current",
                        "url": manifest_path,
                        "digest": manifest_digest,
                    }
                ],
            }
        },
    }
    if channels_mutator is not None:
        channels_mutator(channels)

    payloads = {f"{site}{manifest_path}": manifest_bytes}
    payloads.update({f"{site}{path}": data for path, data in artifacts.items()})
    if payload_mutator is not None:
        payload_mutator(payloads, checker)

    if index_text is None:
        index_text = " ".join(
            [
                "Stable",
                "Recommended release channel.",
                current_binary,
                manifest_path,
            ]
        )

    package = manifest["packages"][0]
    binary = package["binaries"][0]
    profile = manifest["profiles"]["co-work"]
    architecture = profile["architectures"][0]
    config = architecture["config"][0]
    image_digest_labels = [
        label
        for image in architecture["images"]
        for label in (
            checker.hash_label(image["digest"]["sha256"]),
            checker.hash_label(image["digest"]["blake3"]),
        )
    ]
    evidence_digest_labels = [
        label
        for evidence in architecture["evidence"]
        for label in (
            checker.hash_label(evidence["digest"]["sha256"]),
            checker.hash_label(evidence["digest"]["blake3"]),
        )
    ]
    channel_page_text = " ".join(
        [
            "Stable",
            current_binary,
            manifest_path,
            package["name"],
            package["version"],
            profile["id"],
            profile["name"],
            profile["revision"],
            profile["min_capsem_version"],
        ]
    )
    package_page_text = " ".join(
        [
            package["name"],
            package["version"],
            package["kind"],
            checker.hash_label(package["digest"]["sha256"]),
            checker.hash_label(package["digest"]["blake3"]),
            binary["name"],
            binary["version"],
            binary["description"],
            binary["installed_path"],
            binary["sbom_component_ref"],
            checker.hash_label(binary["digest"]["sha256"]),
            checker.hash_label(binary["digest"]["blake3"]),
        ]
    )
    profile_page_text = " ".join(
        [
            profile["name"],
            profile["id"],
            profile["revision"],
            architecture["architecture"],
            checker.hash_label(config["digest"]["sha256"]),
            checker.hash_label(config["digest"]["blake3"]),
            *image_digest_labels,
            *evidence_digest_labels,
        ]
    )

    headers = {
        f"{site}/": "no-cache, must-revalidate",
        f"{site}/channels.json": "no-cache, must-revalidate",
        f"{site}{manifest_path}": "no-cache, must-revalidate",
    }
    for path in artifacts:
        headers[f"{site}{path}"] = "public, max-age=31536000, immutable"
    if headers_mutator is not None:
        headers_mutator(headers)

    def fake_fetch_text(url: str):
        if url == f"{site}/":
            return checker.FetchText(text=index_text)
        if url == f"{site}/channels/{channel}/":
            return checker.FetchText(text=channel_page_text)
        if url == f"{site}/channels/{channel}/packages/{package['id']}/":
            return checker.FetchText(text=package_page_text)
        if url == f"{site}/channels/{channel}/profiles/{profile['id']}/":
            return checker.FetchText(text=profile_page_text)
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
        "current_binary": current_binary,
        "current_assets": current_assets,
        "profile_revision": profile_revision,
        "manifest": manifest,
        "channels": channels,
    }


def test_remote_readiness_accepts_channels_manifest_profile_graph_contract() -> None:
    checker = _readiness_checker_module()
    fixture = _install_release_graph_contract_fixture(checker)

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert result.ok, result.detail
    assert "channels.json" in result.detail
    assert "graph manifest" in result.detail
    assert "profile artifacts" in result.detail


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
    assert "release index stable missing channel label Stable" in result.detail
    assert (
        "release index stable missing channel description Recommended release channel."
        in result.detail
    )
    assert "release index stable missing manifest URL /assets/stable/manifest.json" in result.detail


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
    assert (
        "release index stable missing manifest URL /assets/nightly/manifest.json" in result.detail
    )
    assert "channel page stable missing manifest URL /assets/nightly/manifest.json" in result.detail
    assert (
        "unexpected header fetch https://release.capsem.org/assets/nightly/manifest.json"
        in result.detail
    )


def test_remote_readiness_rejects_profile_catalog_artifact_drift() -> None:
    checker = _readiness_checker_module()

    def stale_rootfs_digest(manifest: dict[str, object]) -> None:
        profile = manifest["profiles"]["co-work"]
        architecture = profile["architectures"][0]
        rootfs = next(item for item in architecture["images"] if item["kind"] == "rootfs")
        rootfs["digest"]["blake3"] = "0" * 64

    fixture = _install_release_graph_contract_fixture(
        checker,
        manifest_mutator=stale_rootfs_digest,
    )

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert (
        "profile co-work architecture arm64 image "
        "/assets/releases/2030.0101.1/arm64-rootfs.erofs blake3 mismatch" in result.detail
    )


def test_remote_readiness_rejects_profile_catalog_content_drift() -> None:
    checker = _readiness_checker_module()
    source = "/profiles/releases/profiles-2030.0101.1/co-work/arm64/software-inventory.json"

    def stale_inventory(payloads: dict[str, bytes], _checker) -> None:
        payloads[f"https://release.capsem.org{source}"] = (
            b'{"schema":"capsem.profile_software_inventory.v0","packages":[]}\n'
        )

    fixture = _install_release_graph_contract_fixture(checker, payload_mutator=stale_inventory)

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert (
        f"profile co-work architecture arm64 evidence {source} software inventory schema mismatch"
        in result.detail
    )


def test_remote_readiness_rejects_asset_file_metadata_drift() -> None:
    checker = _readiness_checker_module()
    asset_path = "/assets/releases/2030.0101.1/arm64-rootfs.erofs"

    def stale_rootfs_size(manifest: dict[str, object]) -> None:
        profile = manifest["profiles"]["co-work"]
        architecture = profile["architectures"][0]
        rootfs = next(item for item in architecture["images"] if item["kind"] == "rootfs")
        rootfs["bytes"] = 4

    fixture = _install_release_graph_contract_fixture(
        checker,
        manifest_mutator=stale_rootfs_size,
    )

    result = checker.check_release_site_contract(fixture["site"], fixture["channel"])

    assert not result.ok
    assert f"profile co-work architecture arm64 image {asset_path} size mismatch" in result.detail


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
    assert "scripts/check-public-binary-release.py" in verify_downloads
    assert '--channel "$RELEASE_CHANNEL"' in verify_downloads
    assert (
        "--stable-manifest-url https://release.capsem.org/assets/stable/manifest.json"
    ) in verify_downloads
    assert (
        "--nightly-manifest-url https://release.capsem.org/assets/nightly/manifest.json"
    ) in verify_downloads
    assert '--manifest-url "$ASSET_MANIFEST_URL"' in verify_downloads
    assert "--install-script-url https://capsem.org/install.sh" in verify_downloads
    assert "--docker-linux-install" not in verify_downloads
    assert "Enable KVM for live public-install VM proof" in verify_downloads
    assert "Install live public Linux release and prove guest shell execution" in verify_downloads
    assert (
        'curl -fsSL https://capsem.org/install.sh | CAPSEM_CHANNEL="$RELEASE_CHANNEL" sh'
        in verify_downloads
    )
    assert "dpkg-query -W -f='${Version}' capsem" in verify_downloads
    assert 'grep -F "Running:   true" /tmp/capsem-live-status.txt' in verify_downloads
    assert 'grep -F "Service:   ok" /tmp/capsem-live-status.txt' in verify_downloads
    assert 'grep -F "Gateway:   ok" /tmp/capsem-live-status.txt' in verify_downloads
    assert "scripts/prove-installed-shell.py" in verify_downloads
    assert "scripts/verify-installed-release.py" in verify_downloads
    assert "CAPSEM_LIVE_PUBLIC_INSTALL_SHELL_OK" in verify_downloads
    assert '"$HOME/.capsem/bin/capsem" run' not in verify_downloads
    assert "skipping binary e2e" not in verify_downloads
    assert "::warning::no .deb" not in verify_downloads
    assert "::warning::no 'capsem' CLI" not in verify_downloads


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
                assert "ASSET_MANIFEST_URL: https://release.capsem.org/assets/" in workflow
                assert "/manifest.json" in workflow
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
        assert "https://release.capsem.org/assets/nightly/manifest.json" in text
        assert "https://release.capsem.org/channels.json" in text
        assert "`channels.json`" in text
        assert "host SBOM" in text
        assert "package artifacts" in text
        assert "per-binary inventory" in text
        assert "versioned manifest records" in text
        assert "`current`, `supported`, `deprecated`, or `revoked`" in normalized_text
        assert "`min_capsem_version`" in text
        assert "first channel bootstrap may have no host binary evidence yet" in normalized_text
        assert (
            "once binary files are published, missing host SBOM evidence is release-blocking"
            in normalized_text
        )
        assert "stable-to-nightly acceptance gate" in normalized_text
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
        "Profiles own their config files, profile images, ABOM/OBOM evidence" in release_skill_text
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
    assert "profile revision, image artifact URLs" in release_skill
    assert "image artifact URLs" in release_skill
    assert "evidence URLs" in release_skill
    assert "Host SBOM evidence is incomplete unless" in release_skill
    assert "per-binary metadata" in release_skill
    assert "fetch profile-owned artifacts" in release_skill
    assert "attestation subjects and predicate URLs" in release_skill
    assert "curl -fsSL https://release.capsem.org/channels.json" in release_skill
    assert "curl -fsSL https://release.capsem.org/assets/stable/manifest.json" in release_skill
    assert "gh release download vX.Y.Z --pattern manifest.json" not in release_skill
    assert "VM asset manifests" in release_skill
    assert "root channel catalog live on" in release_skill
    assert (
        "`ci.yaml` runs `docs-build`, `site-build`, and `release-site-build` under `pr-gate`"
        in release_skill_text
    )
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
        "Binary releases are explicitly dispatched",
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


def test_docs_describe_multi_channel_release_graph() -> None:
    docs_paths = [
        PROJECT_ROOT / "docs/src/content/docs/security/build-verification.md",
        PROJECT_ROOT / "docs/src/content/docs/development/ci.md",
        PROJECT_ROOT / "docs/src/content/docs/architecture/build-system.md",
    ]
    combined = "\n".join(path.read_text() for path in docs_paths)
    combined_text = " ".join(combined.split())

    for required in [
        "`channels.json`",
        "stable and nightly",
        "versioned manifest records",
        "exactly one `status` enum value",
        "`current`, `supported`, `deprecated`, or `revoked`",
        "package artifacts",
        "per-binary inventory",
        "Every executable inside each package must be listed",
        "SHA-256, BLAKE3, HMAC",
        "`min_capsem_version`",
        "Profiles own profile images, config files, software inventory, and ABOM/OBOM",
        "profile-owned config, image, ABOM, and OBOM files",
        "https://release.capsem.org/assets/stable/manifest.json",
        "https://release.capsem.org/assets/nightly/manifest.json",
        "stable-to-nightly acceptance gate",
        "absence from the channel list",
        "--manifest file:///path/to/assets/manifest.json",
    ]:
        assert required in combined_text, required

    assert "health.json" not in combined
    assert "capsem.assets_channel.health.v1" not in combined
    assert "current binary" not in combined_text
    assert "VM artifact" not in combined
    assert "schema_version" not in combined


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
        "single metadata file records the installed manifest URL separately",
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
        "| `release.yaml` | Manual `{tag, channel}` dispatch | Run one globally serialized stable or nightly release: build apps, install-test the exact packages, publish them, update only the selected channel, and run public glow-up checks |"
        in docs
    )
    assert (
        "| `release-assets.yaml` | Manual | Build profile images/config/evidence, generate `assets/manifest.json`, and optionally deploy the asset channel |"
        in docs
    )
    assert (
        "| `release-channel-staging.yaml` | Manual | Build a deterministic staging asset channel fixture, deploy it to a Cloudflare Pages preview branch, and validate the same release-channel contract without invoking `build-assets`, `build-app-macos`, or `build-app-linux` |"
        in docs
    )
    assert (
        "| `release-binary-staging.yaml` | Manual | Build a deterministic binary-channel dry-run bundle from fake host packages and the live asset manifest, then prove profile image metadata is unchanged without creating a GitHub release or deploying release.capsem.org |"
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
    assert "`/channels.json`, and" in docs
    assert "`/assets/<channel>/manifest.json` before the workflow can pass" in docs
    assert (
        "`docs.yaml` and `site.yaml` are independent from binary and profile image release" in docs
    )
    assert "`https://docs.capsem.org/`, content type `text/html`" in docs
    assert "`https://capsem.org/`, content type `text/html`" in docs


def test_ci_docs_compare_pr_gate_to_just_test_with_named_substitutions() -> None:
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    just_test = _recipe_block("test:")

    for stage in [
        "Audits + lint + web surfaces",
        "Cross-compile agent (both arches)",
        "Rust: test suite with coverage",
        "Python: non-serial tests (n=4 parallel)",
        "Python: serial timing and benchmark tests",
        "Python: Build chain and release tests (serial)",
        "Injection test",
        "Integration test",
        "Benchmarks",
        "Cross-compile Linux releases (Docker, both arches)",
        "Install e2e tests (Docker + systemd)",
    ]:
        assert stage in just_test

    assert "## PR gate compared with `just test`" in docs
    assert (
        "| Audits, lint, and all web surfaces | `test`, `docs-build`, `site-build`, and `release-site-build` reuse `scripts/check-web-surface.sh` | Same checked-in entrypoint; `just test` remains the canonical owner |"
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
        "| Docs, marketing, and release-channel site builds | `docs-build`, `site-build`, and `release-site-build` call the same web-surface entrypoint as `just test` before `pr-gate` can pass | Merge-blocking duplicate execution of the canonical local gate; deploy happens only after merge or explicit release-channel publication |"
        in docs
    )
    assert "`pr-gate` is the only status that should be required by branch protection" in docs
    assert "`pr-gate` depends on `docs-build`, `site-build`, and `release-site-build`" in docs_text
    assert (
        "needs: [test-linux, test, test-install, docs-build, site-build, release-site-build]"
        in workflow
    )


def test_release_skills_require_local_ci_execution_parity_and_record_native_musl_lesson() -> None:
    testing = (PROJECT_ROOT / "skills/dev-testing/SKILL.md").read_text()
    skills = (PROJECT_ROOT / "skills/dev-skills/SKILL.md").read_text()
    debugging = (PROJECT_ROOT / "skills/dev-debugging/SKILL.md").read_text()
    release = (PROJECT_ROOT / "skills/release-process/SKILL.md").read_text()

    for document in (testing, debugging, release):
        assert "Local/CI execution parity" in document
        assert "same production entrypoint" in document
        assert "Docker" in document

    assert "local/CI parity" in skills
    assert "native `musl-gcc`" in skills
    assert "`x86_64-linux-musl-gcc`" in skills
    assert "unavoidable platform boundary" in testing
    assert "physical Mac" in testing
    assert "exact-SHA CI" in testing
    assert "release-assets.yaml" in release
    assert "linux_musl_toolchain_available" in release


def test_release_critical_workflows_share_local_entrypoints_or_name_platform_boundaries() -> None:
    just = (PROJECT_ROOT / "justfile").read_text()
    qualification = _workflow_text("release-qualification.yaml")
    assets = _workflow_text("release-assets.yaml")
    ci = _workflow_text("ci.yaml")
    release = _workflow_text("release.yaml")
    release_skill = (PROJECT_ROOT / "skills/release-process/SKILL.md").read_text()

    assert "run: just test" in qualification
    assert "test:" in just

    for command in ("just build-kernel", "just build-rootfs"):
        assert command in assets
    assert "build-kernel arch" in just
    assert "build-rootfs arch" in just

    assert "run: just test-install" in ci
    assert "test-install:" in just

    for shared_script in (
        "scripts/build-pkg.sh",
        "scripts/repack-deb.sh",
        "scripts/verify-installed-release.py",
        "scripts/prove-installed-shell.py",
    ):
        assert shared_script in release
        assert shared_script in just

    for unavoidable_boundary in (
        "Apple signing and notarization",
        "hosted-runner KVM",
        "Cloudflare publication",
        "physical-Mac VZ shell proof",
    ):
        assert unavoidable_boundary in release_skill


def test_web_surfaces_share_one_local_and_ci_entrypoint() -> None:
    script = _source_text("scripts/check-web-surface.sh")
    just = (PROJECT_ROOT / "justfile").read_text()
    ci = _workflow_text("ci.yaml")
    docs = _workflow_text("docs.yaml")
    site = _workflow_text("site.yaml")
    release = _workflow_text("release.yaml")
    release_assets = _workflow_text("release-assets.yaml")
    binary_staging = _workflow_text("release-binary-staging.yaml")
    channel_staging = _workflow_text("release-channel-staging.yaml")

    for surface in (
        "frontend",
        "frontend-build",
        "docs",
        "site",
        "release-site",
        "release-site-build",
    ):
        assert f"{surface})" in script

    assert "bash scripts/check-web-surface.sh frontend" in just
    assert "bash scripts/check-web-surface.sh docs" in just
    assert "bash scripts/check-web-surface.sh site" in just
    assert "bash scripts/check-web-surface.sh release-site" in just

    assert "bash scripts/check-web-surface.sh frontend" in ci
    assert "bash scripts/check-web-surface.sh docs" in ci
    assert "bash scripts/check-web-surface.sh site" in ci
    assert "bash scripts/check-web-surface.sh release-site" in ci
    assert "bash scripts/check-web-surface.sh docs" in docs
    assert "bash scripts/check-web-surface.sh site" in site
    assert release.count("bash scripts/check-web-surface.sh frontend-build") == 2
    for workflow in (binary_staging, channel_staging):
        assert "bash scripts/check-web-surface.sh release-site-build" in workflow
    assert "scripts/build-complete-release-channel.py" in release
    assert "scripts/build-complete-release-channel.py" in release_assets

    bypasses = (
        "cd frontend && pnpm run build",
        "cd frontend && pnpm build",
        "cd docs && pnpm run build",
        "cd site && pnpm run build",
        "cd release-site && pnpm run build:channel",
    )
    for text in (
        just,
        ci,
        docs,
        site,
        release,
        release_assets,
        binary_staging,
        channel_staging,
    ):
        for bypass in bypasses:
            assert bypass not in text

    assert "write-release-site-ci-fixture.py" in script
    assert "build-complete-release-channel.py" in script
    assert "pnpm --dir release-site run build:channel" in script
    assert 'test -s "$CAPSEM_RELEASE_CHANNEL_DIST/404.html"' in script
    assert 'grep -q "Artifact not found"' in script
    complete_builder = _source_text("scripts/build-complete-release-channel.py")
    assert '"assets",\n                "channel",\n                "check"' in complete_builder
    assert "CAPSEM_FRONTEND_JUNIT" in script


def test_ironbank_release_rule_is_the_complete_local_and_ci_just_test() -> None:
    just = (PROJECT_ROOT / "justfile").read_text()
    qualification = _workflow_text("release-qualification.yaml")
    testing = (PROJECT_ROOT / "skills/dev-testing/SKILL.md").read_text()
    ironbank = (PROJECT_ROOT / "skills/ironbank/SKILL.md").read_text()
    release = (PROJECT_ROOT / "skills/release-process/SKILL.md").read_text()

    for document in (testing, ironbank, release):
        assert "Ironbank parity rule" in document
        assert "every portable release gate" in document
        assert "`just test`" in document

    assert "run: just test" in qualification
    assert "cargo llvm-cov --workspace --bins --lib --tests" in just
    assert "--fail-under-lines 65" in just
    assert "--cov-fail-under=90" in just
    assert "CAPSEM_REQUIRE_ARTIFACTS=1" in just
    assert "tests/ironbank/test_route_health.py" in just
    assert "scripts/integration_test.py" in just
    assert "=== Benchmarks ===" in just
    assert "tests/capsem-serial/test_capsem_bench_baseline.py" in just
    assert "just test-install" in just
    for surface in ("frontend", "docs", "site", "release-site"):
        assert f"bash scripts/check-web-surface.sh {surface}" in just


def test_release_channel_deploy_validates_the_deployed_channel_shape() -> None:
    deploy = _workflow_text("release-channel.yaml")
    staging = _workflow_text("release-channel-staging.yaml")

    assert "validate_complete_public_channels:" in deploy
    assert 'CHANNEL_ARGS=(--channel "$CHANNEL")' in deploy
    assert "CHANNEL_ARGS=(--channel stable --channel nightly)" in deploy
    assert '"${CHANNEL_ARGS[@]}"' in deploy
    assert "validate_complete_public_channels: false" in staging


def test_remote_release_readiness_checker_is_read_only_and_covers_live_gates() -> None:
    script = (PROJECT_ROOT / "scripts/check-remote-release-readiness.py").read_text()
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())

    assert "Read-only remote release readiness checks" in script
    assert 'git", "rev-list", "--left-right", "--count"' in script
    assert 'gh", "workflow", "view", "ci.yaml"' in script
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
    assert (
        "aggregates `test-linux`, `test`, `test-install`, `docs-build`, `site-build`, and `release-site-build`"
        in (docs_text)
    )
    assert "runs with `if: ${{ always() }}` and asserts every dependency result" in docs_text
    assert "branch protection or active branch rulesets require `pr-gate`" in docs_text
    assert "`release.capsem.org` resolves and serves the generated release graph" in docs_text


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


def test_remote_release_readiness_fetch_retries_ipv4_on_network_unreachable(monkeypatch) -> None:
    checker = _readiness_checker_module()
    calls: list[tuple[str, str]] = []
    failures_left = {
        ("GET", "https://release.capsem.org/ipv6-body"): 1,
        ("HEAD", "https://release.capsem.org/ipv6-headers"): 1,
    }

    class FakeResponse:
        headers = {"Cache-Control": "no-cache, must-revalidate"}

        def __enter__(self):
            return self

        def __exit__(self, *_args):
            return False

        def read(self) -> bytes:
            return b"ok"

    def fake_urlopen(request, *, timeout: int):
        method = request.get_method()
        key = (method, request.full_url)
        calls.append(key)
        assert timeout == 20
        if failures_left.get(key, 0) > 0:
            failures_left[key] -= 1
            raise checker.urllib.error.URLError(
                OSError(checker.errno.ENETUNREACH, "Network is unreachable")
            )
        return FakeResponse()

    monkeypatch.setattr(checker.urllib.request, "urlopen", fake_urlopen)

    body = checker.fetch_bytes("https://release.capsem.org/ipv6-body")
    headers = checker.fetch_headers("https://release.capsem.org/ipv6-headers")

    assert body == checker.FetchBytes(b"ok")
    assert headers == checker.FetchHeaders({"cache-control": "no-cache, must-revalidate"})
    assert calls.count(("GET", "https://release.capsem.org/ipv6-body")) == 2
    assert calls.count(("HEAD", "https://release.capsem.org/ipv6-headers")) == 2


def test_live_release_activation_order_is_documented() -> None:
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    release_skill = (PROJECT_ROOT / "skills/release-process/SKILL.md").read_text()
    asset_skill = (PROJECT_ROOT / "skills/asset-pipeline/SKILL.md").read_text()

    for text in (docs, release_skill, asset_skill):
        normalized = " ".join(text.split())
        normalized_lower = normalized.lower()
        assert "Live release activation order" in text
        assert "merge the release-rail commits to `main` only after" in normalized_lower
        assert "expanded `pr-gate` passes" in normalized_lower
        assert "require only `pr-gate` in branch protection or active rulesets" in normalized_lower
        assert "fail-closed `pr-gate` shape" in normalized_lower
        assert (
            "provision the `release.capsem.org` cloudflare pages project and dns"
            in normalized_lower
        )
        assert "run `uv run python scripts/check-remote-release-readiness.py`" in normalized_lower
        assert (
            "manual VM asset workflow as a dry run" in normalized
            or "manual profile image workflow as a dry run" in normalized
        )
        assert "release-binary-staging.yaml" in normalized
        assert "binary-channel-dry-run-bundle" in normalized
        assert "proof.json" in normalized
        assert (
            "vm asset metadata was not changed" in normalized_lower
            or "vm asset metadata did not change" in normalized_lower
            or "profile image metadata was not changed" in normalized_lower
            or "profile image metadata did not change" in normalized_lower
        )
        assert "explicitly dispatch" in normalized_lower
        assert "exactly one `stable` or `nightly` channel" in normalized_lower
        assert "globally serialized" in normalized_lower
        assert (
            "run the manual vm asset workflow live only after reviewing `asset-release-plan`"
            in normalized_lower
            or "run the manual profile image workflow live only after reviewing `asset-release-plan`"
            in normalized_lower
        )
        assert "installed update smokes" in normalized
        assert normalized_lower.index(
            "merge the release-rail commits to `main` only after"
        ) < normalized_lower.index("require only `pr-gate` in branch protection or active rulesets")
        assert normalized_lower.index(
            "provision the `release.capsem.org` cloudflare pages project and dns"
        ) < min(
            index
            for index in [
                normalized.find("manual VM asset workflow as a dry run"),
                normalized.find("manual profile image workflow as a dry run"),
            ]
            if index >= 0
        )
        assert normalized.index("release-binary-staging.yaml") < normalized_lower.index(
            "push a new immutable `vx.y.z` tag"
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
    fail_closed = (
        inline
        + """
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
    )

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
    assert module.pr_gate_contract_failures(module.workflow_job_block(fail_closed, "pr-gate")) == []
    assert module.pr_gate_contract_failures(module.workflow_job_block(non_failing, "pr-gate")) == [
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
    assert "repos/{repo}/rules/branches/{branch}" in script
    assert "repos/{repo}/rulesets/{ruleset_id}" not in script
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
    assert (
        "VM OBOM evidence /assets/releases/2030.0101.1/arm64-obom.cdx.json blake3 mismatch"
        in failures
    )

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
        "https://github.com/google/capsem/releases/download/assets-v2030.0101.1/arm64-rootfs.erofs"
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
    assert "fetch_and_verify_evidence_artifact(" in script
    assert '"VM asset file"' in script
    assert "uv run python scripts/check-release-site-contract.py" in workflow


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


def test_release_channel_smoke_and_remote_readiness_validate_matching_attestation_predicate_evidence() -> (
    None
):
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
    assert "uv run python scripts/check-release-site-contract.py" in workflow


def test_remote_readiness_rejects_attestation_rail_drift() -> None:
    module = _readiness_checker_module()
    script = _source_text("scripts/check-remote-release-readiness.py")
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
    assert "attestation_expected_rails" in script
    assert "health evidence {attestation_name} scope mismatch" in script
    assert "health evidence {attestation_name} workflow mismatch" in script


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
    script = _source_text("scripts/check-remote-release-readiness.py")
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
    assert "host SBOM evidence {url} name mismatch" in script
    assert "host SBOM attestation predicate_url missing" in script


def test_release_channel_smoke_host_sbom_attestation_subjects_cover_packages() -> None:
    script = _source_text("scripts/check-remote-release-readiness.py")

    assert "host_sbom_attestation_subjects" in script
    assert "github_attestations_host_sbom" in script
    assert "host SBOM attestation subjects missing" in script


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
    assert (
        module.check_release_cache_headers("https://release.capsem.test", "stable", asset_files)
        == []
    )
    assert calls == list(headers)

    headers["https://release.capsem.test/assets/stable/manifest.json"] = (
        "public, max-age=31536000, immutable"
    )
    failures = module.check_release_cache_headers(
        "https://release.capsem.test", "stable", asset_files
    )
    assert (
        "channel manifest https://release.capsem.test/assets/stable/manifest.json "
        "Cache-Control must contain no-cache"
    ) in failures

    assert "def check_release_cache_headers" in script
    assert 'release_site_request(url, method="HEAD")' in script
    assert "RELEASE_VALIDATOR_USER_AGENT" in script
    assert "Cache-Control must contain {directive}" in script
    assert "max-age=31536000" in script
    assert "Cache-Control" in docs
    assert "mutable release-channel pointers" in docs_text
    assert (
        "immutable asset and profile artifacts" in docs_text
        or "immutable profile release artifacts" in docs_text
    )


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
    assert '"create",' in source
    assert '"exec",' in source
    assert '"-e",' in source
    assert 'f"{MOCK_SERVER_ENV}={mock_base_url}"' in source
    assert "PERSISTENT_DIR" in source
    assert '"capsem-doctor"' in source


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
    build_pos = workflow.find("bash scripts/check-web-surface.sh frontend-build")
    capsem_app_pos = workflow.find("-p capsem-app")
    coverage_pos = workflow.rfind("cargo llvm-cov nextest --no-cfg-coverage", 0, capsem_app_pos)

    assert build_pos != -1, "Tauri frontendDist must exist before capsem-app tests compile"
    assert coverage_pos != -1
    assert capsem_app_pos != -1
    assert build_pos < coverage_pos < capsem_app_pos


def test_frontend_generated_settings_use_one_shared_rail() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    release_qualification = (
        PROJECT_ROOT / ".github" / "workflows" / "release-qualification.yaml"
    ).read_text()
    just = (PROJECT_ROOT / "justfile").read_text()
    web_gate = _source_text("scripts/check-web-surface.sh")

    generate_pos = workflow.find("bash scripts/generate-settings.sh")
    first_frontend_build_pos = workflow.find(
        "bash scripts/check-web-surface.sh frontend-build"
    )
    frontend_check_pos = workflow.find("bash scripts/check-web-surface.sh frontend")
    release_gate_pos = release_qualification.find("run: just test")

    assert generate_pos != -1
    assert first_frontend_build_pos != -1
    assert frontend_check_pos != -1
    assert release_gate_pos != -1
    assert "test: _bootstrap _install-tools _clean-stale _pnpm-install _check-generated-settings" in just
    assert "bash scripts/check-web-surface.sh frontend" in just
    assert "pnpm --dir frontend run check" in web_gate
    assert generate_pos < first_frontend_build_pos
    assert generate_pos < frontend_check_pos
    assert "bash scripts/generate-settings.sh" in just
    generated_gate = _recipe_block("_check-generated-settings:")
    assert "bash \"$ROOT/scripts/generate-settings.sh\"" in generated_gate
    assert "cmp -s" in generated_gate
    assert "Generated files were refreshed" in generated_gate
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
    assert 'capsem install" 2>/dev/null || true' not in deb_postinst
    assert "event=service_registration_failed" in deb_postinst
    assert "event=readiness_poll" in deb_postinst
    assert 'grep -q "Service:   ok"' in deb_postinst
    assert 'grep -q "Gateway:   ok"' in deb_postinst
    assert "event=service_not_ready" in deb_postinst


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
    runner = _source_text("scripts/test-linux-rust.sh")
    nextest = tomllib.loads((PROJECT_ROOT / ".config" / "nextest.toml").read_text())

    coverage_step = workflow.split("- name: Unit tests (KVM backend) with coverage", maxsplit=1)[
        1
    ].split("- name: Upload Linux coverage", maxsplit=1)[0]
    slow_timeout = nextest["profile"]["ci"]["slow-timeout"]

    assert "timeout-minutes:" in coverage_step
    assert "run: just test-linux-rust" in coverage_step
    assert "cargo llvm-cov nextest" in runner
    report_block = runner.split("cargo llvm-cov report", maxsplit=1)[1]
    assert "--bins" not in report_block
    assert "--fail-under-lines" not in runner
    assert "--fail-under-lines 65" in _recipe_block("test:")
    assert "--profile ci" in runner
    assert slow_timeout == {
        "period": "120s",
        "terminate-after": 3,
        "grace-period": "10s",
        "on-timeout": "fail",
    }


def test_just_test_owns_linux_rust_platform_coverage_through_docker() -> None:
    canonical_gate = _recipe_block("test:")
    linux_rust_gate = _recipe_block("test-linux-rust:")
    linux_ci = _workflow_job_block("test-linux")
    runner = _source_text("scripts/test-linux-rust.sh")
    host_builder = _source_text("docker/Dockerfile.host-builder")

    assert "just test-linux-rust" in canonical_gate
    assert "test-linux-rust: _generate-settings" not in _source_text("justfile")
    assert "scripts/test-linux-rust.sh" in linux_rust_gate
    assert "capsem-host-builder:latest" in linux_rust_gate
    assert "docker run --rm" in linux_rust_gate
    assert '--user "$HOST_UID:$HOST_GID"' in linux_rust_gate
    assert '-v "$ROOT:/src:ro"' in linux_rust_gate
    assert '"$OUTPUT_DIR/nextest:/src/target/nextest"' in linux_rust_gate
    assert "capsem-linux-rust-cargo-registry" in linux_rust_gate
    assert "capsem-linux-rust-rustup" in linux_rust_gate
    assert 'if [ "$(uname -s)" = "Linux" ]' in linux_rust_gate
    assert linux_rust_gate.index("exit 0") < linux_rust_gate.index(
        "just build-host-image"
    )
    assert "run: just test-linux-rust" in linux_ci
    assert "cargo llvm-cov nextest" not in linux_ci
    assert "cargo llvm-cov nextest" in runner
    assert "capsem-service" in runner
    assert 'package_args+=( -p "$package" )' in runner
    assert "--profile ci" in runner
    assert "cargo install cargo-nextest" in host_builder
    assert "cargo install cargo-llvm-cov" in host_builder
    assert host_builder.count("for attempt in 1 2 3") >= 4
    assert "CARGO_NET_RETRY=10" in host_builder


def test_just_test_builds_real_host_packages_and_runs_production_sbom() -> None:
    canonical_gate = _recipe_block("test:")
    mac_package = _recipe_block("test-macos-release-package:")
    host_sbom = _recipe_block("test-host-package-sbom:")
    release = _source_text(".github/workflows/release.yaml")

    assert "just cross-compile arm64" in canonical_gate
    assert "just cross-compile x86_64" in canonical_gate
    assert "just test-macos-release-package" in canonical_gate
    assert "just test-host-package-sbom" in canonical_gate
    assert "pytest tests/capsem-recipes/" in canonical_gate
    assert "cargo tauri build --bundles app" in mac_package
    assert "cargo build --release" in mac_package
    assert "scripts/build-pkg.sh" in mac_package
    assert "scripts/generate-host-binary-sbom.py" in mac_package
    assert "scripts/generate-host-binary-sbom.py" in host_sbom
    assert 'DEBS=("$ROOT"/dist/*"$VERSION"*.deb)' in host_sbom
    assert "scripts/build-pkg.sh" in release
    assert "scripts/generate-host-binary-sbom.py" in release


def test_release_packages_use_the_shared_all_profile_materialization_rail() -> None:
    release = _source_text(".github/workflows/release.yaml")
    mac_job = _workflow_job_block("build-app-macos", "release.yaml")
    linux_job = _workflow_job_block("build-app-linux", "release.yaml")
    materializer = _source_text("scripts/materialize-config.sh")

    assert 'profile_paths=("$ROOT"/config/profiles/*/profile.toml)' in materializer
    assert 'for profile_path in "${profile_paths[@]}"' in materializer
    assert 'CAPSEM_ASSET_MANIFEST="$ASSET_MANIFEST_URL"' in mac_job
    assert "CAPSEM_ARCH=arm64" in mac_job
    assert "bash scripts/materialize-config.sh" in mac_job
    assert 'CAPSEM_ASSET_MANIFEST="$ASSET_MANIFEST_URL"' in linux_job
    assert 'CAPSEM_ARCH="${{ matrix.arch }}"' in linux_job
    assert "bash scripts/materialize-config.sh" in linux_job
    assert "--profile config/profiles/code/profile.toml" not in release
    for assembler in ("scripts/build-pkg.sh", "scripts/repack-deb.sh"):
        source = _source_text(assembler)
        assert 'for profile_path in "$CONFIG_ROOT"/profiles/*/profile.toml' in source
        assert 'profile validate "$profile_path"' in source
        assert '--config-root "$CONFIG_ROOT" --materialized' in source


def test_all_quick_session_entrypoints_preserve_profile_selection() -> None:
    app = _source_text("frontend/src/lib/components/shell/App.svelte")
    tray_main = _source_text("crates/capsem-tray/src/main.rs")
    tray_gateway = _source_text("crates/capsem-tray/src/gateway.rs")
    cli = _source_text("crates/capsem/src/main.rs")
    mcp = _source_text("crates/capsem-mcp/src/main.rs")

    assert "vmStore.openCreateModal()" in app
    assert "profile_id: 'code'" not in app
    new_session = tray_main.split("Action::NewSession =>", maxsplit=1)[1].split(
        "Action::Save", maxsplit=1
    )[0]
    assert "launch_ui(None)" in new_session
    assert "provision_temp" not in new_session
    assert "provision_temp" not in tray_gateway
    assert "profile_id\":\"code" not in tray_gateway
    assert "profile_id: profile.clone()" in cli
    assert 'params.profile.as_deref().unwrap_or(DEFAULT_PROFILE_ID)' in mcp


def test_just_test_runs_grep_guardrails_for_hardcoded_release_selections() -> None:
    canonical_gate = _recipe_block("test:")
    guard = _source_text("scripts/check-hardcoded-release-selections.sh")
    reusable_channel = _workflow_text("release-channel.yaml")

    assert "bash scripts/check-hardcoded-release-selections.sh" in canonical_gate
    for term in ("code", "co-work", "cowork", "terminal", "termional", "gui"):
        assert term in guard
    assert "rg" in guard
    assert "profile_id" in guard
    assert "--profile" in guard
    assert "stable" in guard
    assert "nightly" in guard
    assert "ASSET_MANIFEST_URL" in guard
    assert ".github/workflows" in guard
    assert "rg --files config/profiles" in guard
    assert "builtin_profile_configs" in guard
    assert "unwrap_or" in guard
    assert "DEFAULT_RELEASE_MANIFEST_URL" in guard
    assert "channel:\n        type: string\n        required: true" in reusable_channel
    assert "inputs.channel || 'stable'" not in reusable_channel
    assert "CHANNEL: ${{ inputs.channel }}" in reusable_channel


def test_hardcoded_release_selection_guard_rejects_each_regression(tmp_path: Path) -> None:
    fixture_paths = (
        ".github/workflows",
        "config/profiles",
        "frontend/src/lib/components",
        "crates/capsem-tray/src",
        "crates/capsem-mcp/src/main.rs",
        "crates/capsem/src/main.rs",
        "crates/capsem/src/update.rs",
        "crates/capsem-service/src/main.rs",
        "crates/capsem-core/src/net/policy_config/profile_contract.rs",
        "scripts/build-pkg.sh",
        "scripts/repack-deb.sh",
        "scripts/deb-postinst.sh",
        "scripts/pkg-scripts/postinstall",
        "tests/capsem-install",
        "justfile",
    )
    for relative in fixture_paths:
        source = PROJECT_ROOT / relative
        target = tmp_path / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        if source.is_dir():
            shutil.copytree(source, target)
        else:
            shutil.copy2(source, target)

    guard = PROJECT_ROOT / "scripts/check-hardcoded-release-selections.sh"
    env = {**os.environ, "CAPSEM_GUARD_ROOT": str(tmp_path)}

    def run_guard() -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(guard)],
            cwd=PROJECT_ROOT,
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )

    baseline = run_guard()
    assert baseline.returncode == 0, baseline.stderr

    dialog = tmp_path / "frontend/src/lib/components/shell/CreateSandboxDialog.svelte"
    for profile in ("code", "co-work", "cowork", "terminal", "termional", "gui"):
        original = dialog.read_text()
        dialog.write_text(original + f"\n<!-- profile_id: '{profile}' -->\n")
        rejected = run_guard()
        dialog.write_text(original)
        assert rejected.returncode != 0, f"guard accepted hardcoded profile {profile}"
        assert "hardcodes a named profile" in rejected.stderr

    workflow = tmp_path / ".github/workflows/release-binary-staging.yaml"
    for channel in ("stable", "nightly"):
        original = workflow.read_text()
        workflow.write_text(
            original
            + f"\n# ASSET_MANIFEST_URL: https://release.capsem.org/assets/{channel}/manifest.json\n"
        )
        rejected = run_guard()
        workflow.write_text(original)
        assert rejected.returncode != 0, f"guard accepted hardcoded channel {channel}"
        assert "hardcodes a stable/nightly ASSET_MANIFEST_URL" in rejected.stderr

    postinstall = tmp_path / "scripts/deb-postinst.sh"
    original = postinstall.read_text()
    postinstall.write_text(
        original
        + "\n# MANIFEST_SOURCE='https://release.capsem.org/assets/nightly/manifest.json'\n"
    )
    rejected = run_guard()
    postinstall.write_text(original)
    assert rejected.returncode != 0
    assert "postinstall silently falls back" in rejected.stderr

    update = tmp_path / "crates/capsem/src/update.rs"
    original = update.read_text()
    update.write_text(original + "\n// value.unwrap_or(DEFAULT_RELEASE_MANIFEST_URL)\n")
    rejected = run_guard()
    update.write_text(original)
    assert rejected.returncode != 0
    assert "installed update flow silently substitutes" in rejected.stderr

    future_profile = tmp_path / "config/profiles/terminal/profile.toml"
    future_profile.parent.mkdir(parents=True)
    shutil.copy2(tmp_path / "config/profiles/code/profile.toml", future_profile)
    rejected = run_guard()
    assert rejected.returncode != 0
    assert "builtin_profile_configs does not exactly mirror" in rejected.stderr
    future_profile.unlink()

    for future_name in ("terminal", "gui"):
        future_profile = tmp_path / f"config/profiles/{future_name}/profile.toml"
        future_profile.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(tmp_path / "config/profiles/code/profile.toml", future_profile)
        rejected = run_guard()
        future_profile.unlink()
        assert rejected.returncode != 0, f"guard accepted unembedded profile {future_name}"
        assert "builtin_profile_configs does not exactly mirror" in rejected.stderr

    profile_contract = (
        tmp_path / "crates/capsem-core/src/net/policy_config/profile_contract.rs"
    )
    original = profile_contract.read_text()
    profile_contract.write_text(
        original + '\n// include_str!("../../../../../config/profiles/gui/profile.toml")\n'
    )
    rejected = run_guard()
    profile_contract.write_text(original)
    assert rejected.returncode != 0
    assert "builtin_profile_configs does not exactly mirror" in rejected.stderr

    original = dialog.read_text()
    for regression in ("profileId = 'terminal'", "<option value='gui'>GUI</option>"):
        dialog.write_text(original + f"\n<!-- {regression} -->\n")
        rejected = run_guard()
        assert rejected.returncode != 0, f"guard accepted picker regression {regression}"
        assert "profile picker fabricates" in rejected.stderr
    dialog.write_text(original)

    mcp = tmp_path / "crates/capsem-mcp/src/main.rs"
    original = mcp.read_text()
    for regression, message in (
        ('// "profile_id": DEFAULT_PROFILE_ID\n', "MCP request bypasses"),
        ('// "/profiles/{}/mcp/servers", DEFAULT_PROFILE_ID\n', "silently uses the default"),
    ):
        mcp.write_text(original + "\n" + regression)
        rejected = run_guard()
        assert rejected.returncode != 0, f"guard accepted {regression.strip()}"
        assert message in rejected.stderr
    mcp.write_text(original)

    release_workflow = tmp_path / ".github/workflows/release.yaml"
    original = release_workflow.read_text()
    release_workflow.write_text(original + "\n# --profile config/profiles/gui/profile.toml\n")
    rejected = run_guard()
    release_workflow.write_text(original)
    assert rejected.returncode != 0
    assert "materializes one named profile" in rejected.stderr

    for selection in (
        "stable",
        "nightly",
        "code",
        "co-work",
        "cowork",
        "terminal",
        "termional",
        "gui",
    ):
        original = workflow.read_text()
        input_name = "channel" if selection in {"stable", "nightly"} else "profile"
        workflow.write_text(original + f"\nregression:\n  {input_name}:\n    default: {selection}\n")
        rejected = run_guard()
        workflow.write_text(original)
        assert rejected.returncode != 0, f"guard accepted workflow default {selection}"
        assert "silently defaults" in rejected.stderr

    qualification = tmp_path / ".github/workflows/release-qualification.yaml"
    original = qualification.read_text()
    for channel in ("stable", "nightly"):
        qualification.write_text(original + f"\n# CAPSEM_INSTALL_CHANNEL: {channel}\n")
        rejected = run_guard()
        assert rejected.returncode != 0, f"guard accepted qualification channel {channel}"
        assert "qualification hardcodes" in rejected.stderr
    qualification.write_text(original)
    qualification.write_text(
        original
        + "\n# CAPSEM_RELEASE_MANIFEST_URL=https://release.capsem.org/assets/stable/manifest.json\n"
    )
    rejected = run_guard()
    qualification.write_text(original)
    assert rejected.returncode != 0
    assert "qualification bypasses installed manifest-metadata" in rejected.stderr

    installed_update_test = tmp_path / "tests/capsem-install/test_update.py"
    original_installed_update_test = installed_update_test.read_text()
    for override in ("CAPSEM_RELEASE_MANIFEST_URL", "CAPSEM_RELEASE_HEALTH_URL"):
        installed_update_test.write_text(
            original_installed_update_test + f'\n# "{override}": manifest_url\n'
        )
        rejected = run_guard()
        assert rejected.returncode != 0, f"guard accepted installed test override {override}"
        assert "installed update test bypasses manifest-metadata" in rejected.stderr
    installed_update_test.write_text(original_installed_update_test)

    reusable = tmp_path / ".github/workflows/release-channel.yaml"
    original = reusable.read_text()
    reusable.write_text(original + "\nchannel:\n  type: string\n  required: false\n")
    rejected = run_guard()
    assert rejected.returncode != 0
    assert "makes its channel optional" in rejected.stderr
    reusable.write_text(original + "\n# ${{ inputs.channel || 'stable' }}\n")
    rejected = run_guard()
    reusable.write_text(original)
    assert rejected.returncode != 0
    assert "silently substitutes stable" in rejected.stderr

    justfile = tmp_path / "justfile"
    original = justfile.read_text()
    justfile.write_text(original + "\n# scripts/check-release-qualification.py --sha deadbeef\n")
    rejected = run_guard()
    justfile.write_text(original)
    assert rejected.returncode != 0
    assert "not bound to an explicit channel" in rejected.stderr

    for relative in ("scripts/deb-postinst.sh", "scripts/pkg-scripts/postinstall"):
        postinstall = tmp_path / relative
        original = postinstall.read_text()
        for channel in ("stable", "nightly"):
            postinstall.write_text(
                original
                + f"\n# MANIFEST_SOURCE='https://release.capsem.org/assets/{channel}/manifest.json'\n"
            )
            rejected = run_guard()
            assert rejected.returncode != 0, f"guard accepted {relative} fallback {channel}"
            assert "postinstall silently falls back" in rejected.stderr
        postinstall.write_text(
            original
            + "\n# CAPSEM_RELEASE_MANIFEST_URL=https://release.capsem.org/assets/stable/manifest.json\n"
        )
        rejected = run_guard()
        assert rejected.returncode != 0
        assert "postinstall bypasses installed manifest-metadata" in rejected.stderr
        postinstall.write_text(original)

    original = update.read_text()
    for fallback in (
        "value.unwrap_or(DEFAULT_RELEASE_MANIFEST_URL)",
        "value.unwrap_or_else(|| DEFAULT_RELEASE_MANIFEST_URL)",
    ):
        update.write_text(original + f"\n// {fallback}\n")
        rejected = run_guard()
        assert rejected.returncode != 0, f"guard accepted update fallback {fallback}"
        assert "installed update flow silently substitutes" in rejected.stderr
    update.write_text(original)

    for legacy_sidecar in ("update-check.json", "manifest-origin.json"):
        update.write_text(original + f'\n// "{legacy_sidecar}"\n')
        rejected = run_guard()
        assert rejected.returncode != 0, f"guard accepted legacy {legacy_sidecar}"
        assert "legacy split manifest/update sidecar" in rejected.stderr
    update.write_text(original)


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


def test_guest_init_console_redirection_cannot_kill_pid_one() -> None:
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()

    mknod_pos = init.find("mknod -m 600 /dev/console c 5 1")
    probe_pos = init.find('( : <"$candidate" >"$candidate" )')
    guarded_exec_pos = init.find('if [ -n "$CONSOLE_DEV" ]; then')
    fatal_exec = "exec 0</dev/console 1>/dev/console 2>/dev/console"

    assert mknod_pos != -1, "init must create /dev/console when devtmpfs omits it"
    assert probe_pos != -1, "init must preflight console opens before redirecting PID 1"
    assert guarded_exec_pos != -1, "init must guard console redirection with a usable device check"
    assert fatal_exec not in init, "hard /dev/console redirection exits PID 1 on KVM boot races"
    assert mknod_pos < probe_pos < guarded_exec_pos


def test_guest_init_persists_boot_diagnostics_before_agent_launch() -> None:
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()

    helper_pos = init.find("init_log()")
    workspace_log_pos = init.find("/mnt/shared/workspace/.capsem-boot.log")
    agent_stdio_pos = init.find("/mnt/shared/workspace/.capsem-agent-stdio.log")
    backtrace_pos = init.find("export RUST_BACKTRACE=1")
    kmsg_pos = init.find("> /dev/kmsg")
    launch_log_pos = init.find('init_log "starting PTY agent (vsock mode): $AGENT_PATH"')
    chroot_pos = init.find('chroot /newroot "$AGENT_PATH" >> "$AGENT_STDIO_LOG" 2>&1')
    launch_pos = init.find('chroot /newroot "$AGENT_PATH"')
    exit_status_pos = init.find('init_log "PTY agent exited with status $AGENT_STATUS"')

    assert helper_pos != -1, "init must centralize durable boot diagnostics"
    assert workspace_log_pos != -1, "init diagnostics must survive in host-preserved workspace"
    assert agent_stdio_pos != -1, "agent stderr must survive when it exits before opening its log"
    assert backtrace_pos != -1, "early agent panics must include enough context to fix"
    assert kmsg_pos != -1, "init diagnostics must reach serial-visible kernel log on quiet boots"
    assert launch_log_pos != -1, "init must mark the exact agent launch boundary"
    assert chroot_pos != -1
    assert launch_pos != -1
    assert exit_status_pos != -1, (
        "init must report early agent exits instead of silently idling PID 1"
    )
    assert (
        helper_pos
        < workspace_log_pos
        < agent_stdio_pos
        < launch_log_pos
        < launch_pos
        < exit_status_pos
    )


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


def test_capsem_init_keeps_default_venv_out_of_workspace() -> None:
    """The boot venv must not become forked /root workspace state."""
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()

    assert "ln -sfn /run/capsem-venv /newroot/root/.venv" in init
    assert "uv venv --system-site-packages /run/capsem-venv" in init
    assert "python3 -m venv --system-site-packages /run/capsem-venv" in init
    assert "uv venv --system-site-packages /root/.venv" not in init
    assert "python3 -m venv --system-site-packages /root/.venv" not in init


def test_capsem_agent_repairs_missing_default_venv() -> None:
    """The guest agent must not leave VIRTUAL_ENV unset if init venv races."""
    source = (PROJECT_ROOT / "crates" / "capsem-agent" / "src" / "main.rs").read_text()

    assert 'const VENV_TARGET: &str = "/run/capsem-venv"' in source
    assert "std::thread::spawn(move ||" in source
    assert "std::fs::remove_dir_all(VENV_TARGET)" in source
    assert '.args(["venv", "--system-site-packages", VENV_TARGET])' in source
    assert '.args(["-m", "venv", "--system-site-packages", VENV_TARGET])' in source
    assert 'boot_env.push(("VIRTUAL_ENV".into(), VENV_DIR.into()))' in source
    assert "venv missing after init wait; creating fallback" in source
    assert "venv activated in boot_env" not in source


def test_fork_route_flushes_without_thaw_before_clone() -> None:
    """Pre-fork quiescence must not pay fsfreeze cost and thaw before clone."""
    source = (PROJECT_ROOT / "crates" / "capsem-service" / "src" / "main.rs").read_text()
    fork_block = source.split("async fn handle_fork", maxsplit=1)[1].split(
        "Ok(Json(ForkResponse", maxsplit=1
    )[0]

    assert 'command: "sync; true".to_string()' in fork_block
    assert 'command: "fsfreeze' not in fork_block


def test_linux_vm_launch_preformats_system_overlay_before_boot() -> None:
    """Doctor boot must not pay first-boot mke2fs cost inside the guest."""
    core = (PROJECT_ROOT / "crates" / "capsem-core" / "src" / "lib.rs").read_text()
    process = (PROJECT_ROOT / "crates" / "capsem-process" / "src" / "main.rs").read_text()
    service = (PROJECT_ROOT / "crates" / "capsem-service" / "src" / "main.rs").read_text()
    init = (PROJECT_ROOT / "guest" / "artifacts" / "capsem-init").read_text()

    assert "pub fn preformat_system_overlay_image_if_needed" in core
    assert "pub fn ensure_preformatted_system_overlay_template" in core
    assert "pub fn preformat_system_overlay_image_from_template_if_needed" in core
    assert "auto_snapshot::clone_file(template_path, &tmp_path)" in core
    assert "system_overlay_has_ext4_magic(path)?" in core
    assert "lazy_itable_init=1,lazy_journal_init=1" in core
    assert '.arg("size=4")' in core
    assert "preformat_system_overlay_image_from_template_if_needed" in process
    assert "system_overlay_template_path_for_session" in process
    assert "session_dir, scratch_disk_size_gb" in process
    assert "fn prewarm_system_overlay_templates" in service
    assert "ensure_preformatted_system_overlay_template(&template_path, size_gb)" in service
    assert "prewarm_system_overlay_templates(&run_dir, &profile_cache)" in service
    assert "fn prewarm_vm_asset_hash_cache" in service
    assert "capsem_core::VmConfig::verify_hash(path, hash)" in service
    assert (
        "prewarm_vm_asset_hash_cache(&assets_base_dir, manifest.as_deref(), &current_version)"
        in service
    )
    assert "mke2fs unavailable; guest will format system overlay at first boot" in process
    assert "lazy_itable_init=1,lazy_journal_init=1" in init
    assert "-J size=4" in init


def test_raw_guest_vsock_probes_resolve_kvm_port_offset() -> None:
    """Raw guest vsock probes must connect to logical ports on KVM."""
    source = (PROJECT_ROOT / "tests" / "capsem-e2e" / "test_framed_mcp_mitm.py").read_text()

    assert "def capsem_vsock_port(logical_port):" in source
    assert "capsem.vsock_port_offset=" in source
    assert "VMADDR_CID_HOST, capsem_vsock_port(5002)" in source
    assert "VMADDR_CID_HOST, capsem_vsock_port(5003)" in source
    assert "VMADDR_CID_HOST, 5002" not in source
    assert "VMADDR_CID_HOST, 5003" not in source


def test_create_route_does_not_wait_for_full_guest_readiness() -> None:
    """Create catches immediate boot crashes; exec/file routes own readiness waits."""
    source = (PROJECT_ROOT / "crates" / "capsem-service" / "src" / "main.rs").read_text()
    provision_attempt = source.split("async fn provision_attempt", maxsplit=1)[1].split(
        "\n#[cfg(unix)]", maxsplit=1
    )[0]

    assert "std::time::Duration::from_millis(500)" in provision_attempt
    assert "exec/file routes own the full readiness wait" in provision_attempt
    assert "std::time::Duration::from_secs(5)" not in provision_attempt


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
    assert 'ln -sf "/opt/ai-clis/bin/$cli" "/newroot/usr/local/bin/$cli"' in init
    assert 'rm -f "/newroot/root/.local/bin/$cli"' in init
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
