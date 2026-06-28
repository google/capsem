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

    assert "pull_request:" in workflow
    assert "push:" in workflow
    assert "needs: [test-linux, test, test-install]" in gate
    assert "if: ${{ always() }}" in gate
    assert "TEST_LINUX_RESULT: ${{ needs.test-linux.result }}" in gate
    assert "TEST_MACOS_RESULT: ${{ needs.test.result }}" in gate
    assert "TEST_INSTALL_RESULT: ${{ needs.test-install.result }}" in gate
    assert 'test "$TEST_LINUX_RESULT" = success' in gate
    assert 'test "$TEST_MACOS_RESULT" = success' in gate
    assert 'test "$TEST_INSTALL_RESULT" = success' in gate


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
    ]:
        assert f"- name: {step_name}" in workflow
        step = workflow.split(f"- name: {step_name}", maxsplit=1)[1].split(
            "\n      - name:", maxsplit=1
        )[0]
        assert "|| true" not in step, step_name
        assert "continue-on-error: true" not in step, step_name


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
    assert "just build-kernel" in workflow
    assert "just build-rootfs" in workflow
    assert "cargo run -p capsem-admin -- manifest generate assets" in workflow
    assert "cargo run -p capsem-admin -- assets channel build" in workflow
    assert '--manifest "file://$PWD/assets/manifest.json"' in workflow
    assert "cargo run -p capsem-admin -- assets channel check" in workflow
    assert "name: asset-release-plan" in workflow
    assert "path: target/asset-release/" in workflow
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


def test_asset_channel_deploy_consumes_generated_dist_artifact() -> None:
    workflow = _workflow_text("release-channel.yaml")

    assert "workflow_call:" in workflow
    assert "workflow_dispatch:" not in workflow
    assert "dist_artifact:" in workflow
    assert "actions/download-artifact@v8" in workflow
    assert "DIST_DIR: target/release-channel" in workflow
    assert 'test -f "$DIST_DIR/index.html"' in workflow
    assert 'test -f "$DIST_DIR/health.json"' in workflow
    assert 'test -f "$DIST_DIR/assets/$CHANNEL/manifest.json"' in workflow
    assert 'test -d "$DIST_DIR/assets/releases"' in workflow
    assert "cargo run -p capsem-admin -- assets channel build" not in workflow
    assert "Require Cloudflare credentials" in workflow
    assert "CLOUDFLARE_ACCOUNT_ID secret is required to deploy release.capsem.org" in workflow
    assert "CLOUDFLARE_API_TOKEN secret is required to deploy release.capsem.org" in workflow
    assert workflow.index("Require Cloudflare credentials") < workflow.index(
        "cloudflare/wrangler-action@v3"
    )
    assert (
        "pages deploy target/release-channel/ --project-name=capsem-release --branch=main"
        in workflow
    )
    assert "assets/stable/manifest.json" not in workflow
    assert "Smoke public asset channel" in workflow
    assert "RELEASE_SITE_URL: https://release.capsem.org" in workflow
    assert 'curl -fsSL "$RELEASE_SITE_URL/" -o /tmp/release-index.html' in workflow
    assert 'curl -fsSL "$RELEASE_SITE_URL/health.json" -o /tmp/release-health.json' in workflow
    assert (
        'curl -fsSL "$RELEASE_SITE_URL/assets/$CHANNEL/manifest.json" -o /tmp/release-manifest.json'
        in workflow
    )
    assert 'health.get("schema") != "capsem.assets_channel.health.v1"' in workflow
    assert 'health.get("urls", {}).get("manifest") != expected_manifest' in workflow
    assert 'health.get("urls", {}).get("asset_base") != "/assets/releases"' in workflow
    assert 'for key in ("binary", "assets")' in workflow
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
    assert "index page missing {label} {value}" in workflow
    assert 'health.get("asset_releases", [])' in workflow
    assert 'release.get("version") == current_assets' in workflow
    assert "health current asset release date missing" in workflow
    assert "index page missing current asset release date" in workflow
    assert "release.capsem.org smoke failed after deploy." in workflow


def test_asset_channel_deploy_smoke_verifies_public_evidence_artifacts() -> None:
    workflow = _workflow_text("release-channel.yaml")
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    docs_text = " ".join(docs.split())

    assert "astral-sh/setup-uv@v5" in workflow
    assert "uv run python3 - <<'PY'" in workflow
    assert "import hashlib" in workflow
    assert "import blake3" in workflow
    assert "def check_evidence_artifact" in workflow
    assert 'check_evidence_artifact(sbom, "sha256", "sha256", "host SBOM evidence")' in workflow
    assert 'check_evidence_artifact(obom, "hash", "blake3", "VM OBOM evidence")' in workflow
    assert 'data = fetch_bytes(resolve_release_url(url))' in workflow
    assert "health evidence host_sboms missing for published binary files" in workflow
    assert "health evidence vm_oboms missing for published VM assets" in workflow
    assert "health evidence attestations missing for published artifacts" in workflow
    assert "attestation predicate_url {predicate_url} missing from host SBOM evidence" in workflow
    assert "attestation subject {subject} missing from published file lists" in workflow
    assert "resolves published host SBOM and VM OBOM evidence artifacts from `health.json`" in docs_text
    assert "verifies their advertised hashes and sizes" in docs_text
    assert "validates attestation subjects and predicate URLs" in docs_text


def test_docs_and_marketing_sites_build_on_pr_and_deploy_on_main_only() -> None:
    expectations = [
        (
            "docs.yaml",
            "docs",
            "capsem-docs",
            "Smoke public docs site",
            "https://docs.capsem.org",
            "docs.capsem.org smoke failed after deploy.",
        ),
        (
            "site.yaml",
            "site",
            "capsem",
            "Smoke public marketing site",
            "https://capsem.org",
            "capsem.org smoke failed after deploy.",
        ),
    ]

    for workflow_name, directory, project_name, smoke_name, site_url, failure in expectations:
        workflow = _workflow_text(workflow_name)

        assert "pull_request:" in workflow, workflow_name
        assert "push:" in workflow, workflow_name
        assert "branches: [main]" in workflow, workflow_name
        assert f"'{directory}/**'" in workflow, workflow_name
        assert f"'.github/workflows/{workflow_name}'" in workflow, workflow_name
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
    assert "assets-v" not in workflow
    assert '--manifest "$ASSET_MANIFEST_URL"' in workflow
    assert "release.capsem.org" in workflow
    assert "assets channel record-binary" in workflow
    assert '--asset-source-base "https://release.capsem.org/assets/releases"' in workflow
    assert "uses: ./.github/workflows/release-channel.yaml" in workflow
    assert "dist_artifact: binary-channel-preview" in workflow
    assert "needs: [deploy-release-channel]" in workflow
    assert "cloudflare/wrangler-action" not in workflow
    assert "pages deploy" not in workflow
    assert "capsem-release" not in workflow
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


def test_binary_release_verifies_packages_hydrate_vm_assets_from_public_channel() -> None:
    verify_downloads = _workflow_job_block("verify-release-downloads", "release.yaml")

    assert "needs: [deploy-release-channel]" in verify_downloads
    assert 'curl -fsSL "$ASSET_MANIFEST_URL" -o /tmp/verify/manifest.json' in verify_downloads
    assert 'BASE="${ASSET_MANIFEST_URL%/stable/manifest.json}/releases"' in verify_downloads
    assert 'url="$BASE/$asset_version/$arch-$name"' in verify_downloads
    assert 'code=$(curl -sIL -o /dev/null -w "%{http_code}" "$url")' in verify_downloads
    assert 'gh release download "${{ github.ref_name }}"' in verify_downloads
    assert '--pattern "Capsem_*_${deb_arch}.deb" -D /tmp/deb' in verify_downloads
    assert 'ar x "$deb"' in verify_downloads
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

    for text in (docs, asset_skill, release_skill):
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
        assert "channels/stable/index.json" not in text


def test_release_skill_keeps_binary_and_asset_verification_decoupled() -> None:
    release_skill = (PROJECT_ROOT / "skills/release-process/SKILL.md").read_text()

    assert "asset-channel-preview" in release_skill
    assert "generated dist artifact" in release_skill
    assert "smoke-check `https://release.capsem.org/`, `/health.json`, and" in release_skill
    assert "`/assets/<channel>/manifest.json`" in release_skill
    assert "reject stale public HTML" in release_skill
    assert "current binary, current VM asset version, and asset release date" in release_skill
    assert "resolve published host" in release_skill
    assert "SBOM and VM OBOM evidence artifacts from `health.json`" in release_skill
    assert "verify their advertised" in release_skill
    assert "attestation subjects and predicate URLs" in release_skill
    assert "curl -fsSL https://release.capsem.org/health.json" in release_skill
    assert "curl -fsSL https://release.capsem.org/assets/stable/manifest.json" in release_skill
    assert "gh release download vX.Y.Z --pattern manifest.json" not in release_skill
    assert "VM asset manifests" in release_skill
    assert "channel health live on `release.capsem.org`" in release_skill
    assert "`docs.yaml` and `site.yaml` build on pull requests" in release_skill
    assert "deploy only on pushes to `main`" in release_skill
    assert "`https://docs.capsem.org/` plus `/getting-started/`" in release_skill
    assert "`https://capsem.org/` for marketing" in release_skill
    assert "must not depend on release tags or VM asset publication" in release_skill


def test_capsem_update_checks_release_channel_health_not_github_latest() -> None:
    update_rs = (PROJECT_ROOT / "crates/capsem/src/update.rs").read_text()

    assert "https://release.capsem.org/health.json" in update_rs
    assert "capsem.assets_channel.health.v1" in update_rs
    assert "CAPSEM_RELEASE_HEALTH_URL" in update_rs
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
        "| `docs.yaml` | Pull requests and push to main (docs changes) | Build docs on PRs; deploy docs.capsem.org on main, then smoke the live docs site |"
        in docs
    )
    assert (
        "| `site.yaml` | Pull requests and push to main (site changes) | Build marketing site on PRs; deploy capsem.org on main, then smoke the live marketing site |"
        in docs
    )
    assert (
        "| `release-channel.yaml` | Called by binary or asset release | Deploy release.capsem.org from the generated release-channel site artifact |"
        in docs
    )
    assert "release.yaml` | Tag push (`v*`) | Build assets" not in docs
    assert "generated asset manifest artifact" not in docs
    assert "### pr-gate (ubuntu-latest)" in docs
    assert "`test-linux`, `test`, and `test-install`" in docs
    assert "fails unless all three dependency jobs report `success`" in docs
    assert "After Cloudflare deploys, `release-channel.yaml` smoke" in docs
    assert "`https://release.capsem.org/` index" in docs
    assert "`/health.json`, and" in docs
    assert "`/assets/<channel>/manifest.json` before the workflow can pass" in docs
    assert "`docs.yaml` and `site.yaml` are independent from binary and VM asset release" in docs
    assert "`https://docs.capsem.org/`, content type `text/html`" in docs
    assert "`https://capsem.org/`, content type `text/html`" in docs


def test_ci_docs_compare_pr_gate_to_just_test_with_named_substitutions() -> None:
    docs = (PROJECT_ROOT / "docs/src/content/docs/development/ci.md").read_text()
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    just_test = _recipe_block("test:")

    for stage in [
        "Audits + lint + frontend",
        "Cross-compile agent (both arches)",
        "Rust: test suite with coverage",
        "Python: non-serial tests (n=4 parallel)",
        "Python: serial timing and benchmark tests",
        "Python: Build chain tests (serial)",
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
    assert "`pr-gate` is the only status that should be required by branch protection" in docs
    assert "needs: [test-linux, test, test-install]" in workflow


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
    assert "capsem.assets_channel.health.v1" in script
    assert "pr-gate" in script
    assert "current asset release date" in script
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
    assert "branch protection or active branch rulesets require `pr-gate`" in docs_text
    assert "`release.capsem.org` resolves and serves the asset channel" in docs_text


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
                    "predicate_type": "https://slsa.dev/provenance/v1",
                    "predicate_url": None,
                    "verify_command": "gh attestation verify <subject-url> --owner google",
                    "subjects": [obom_path],
                },
                {
                    "name": "github_attestations_host_sbom",
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
    assert "fetch_and_verify_evidence_artifact(site, sbom, \"sha256\", \"host SBOM evidence\")" in script
    assert "fetch_and_verify_evidence_artifact(site, obom, \"blake3\", \"VM OBOM evidence\")" in script
    assert "hashlib.sha256" in script
    assert "blake3.blake3" in script
    assert "attestation subject {subject} missing from published file lists" in script
    assert "attestation predicate_url {predicate_url} missing from host SBOM evidence" in script
    assert "resolves published host SBOM and VM OBOM evidence artifacts" in docs_text
    assert "verifies their advertised hashes and sizes" in docs_text
    assert "validates attestation subjects and predicate URLs" in docs_text


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
    deb_postinst = (PROJECT_ROOT / "scripts" / "deb-postinst.sh").read_text()
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
    assert "pkill -9 -x capsem-app" in preinstall
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
    sign_pos = block.find("codesign --sign - --entitlements entitlements.plist --force")
    pytest_pos = block.find("uv run python -m pytest tests/capsem-bootstrap/")

    assert asset_pos != -1
    assert build_pos != -1
    assert sign_pos != -1
    assert pytest_pos != -1
    assert asset_pos < pytest_pos
    assert build_pos < pytest_pos
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
