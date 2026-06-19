"""Release doctor contract tests."""

from __future__ import annotations

import json
import re
import subprocess
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parent.parent
FAST_DOCTOR_FLAG = "doctor " + "--" + "fast"
OLD_DEBUG_CRATE = "capsem-debug" + "-upstream"


def _recipe_block(name: str) -> str:
    lines = (PROJECT_ROOT / "justfile").read_text().splitlines()
    start = next(
        i for i, line in enumerate(lines) if line == name or line.startswith(f"{name} ")
    )
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line.startswith((" ", "\t", "#")):
            end = i
            break
    return "\n".join(lines[start:end])


def _workflow_job_block(name: str) -> str:
    lines = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text().splitlines()
    start = next(i for i, line in enumerate(lines) if line == f"  {name}:")
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line.startswith("  ") and not line.startswith("    ") and line.endswith(":"):
            end = i
            break
    return "\n".join(lines[start:end])


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


def test_install_e2e_generates_manifest_through_admin_rail() -> None:
    script = (PROJECT_ROOT / "scripts" / "prepare-install-test-assets.sh").read_text()

    assert "cargo run -p capsem-admin -- manifest generate" in script
    assert 'arm64|aarch64)' in script
    assert 'write_if_missing "$ASSETS_DIR/$arch/vmlinuz"' in script
    assert 'create_minimal_initrd_if_missing "$ASSETS_DIR/$arch/initrd.img"' in script
    assert 'write_if_missing "$ASSETS_DIR/$arch/initrd.img"' not in script
    assert "cpio -o -H newc" not in script
    assert "gzip.open" in script
    assert "TRAILER!!!" in script
    assert 'write_if_missing "$ASSETS_DIR/$arch/rootfs.erofs"' in script
    assert "scripts/gen_manifest.py" not in script


def test_ci_installs_b3sum_before_bootstrap_asset_hash_checks() -> None:
    workflow = _workflow_job_block("test")

    install_tools_pos = workflow.find("- name: Install tools")
    b3sum_pos = workflow.find("cargo install b3sum --locked")
    bootstrap_pos = workflow.find("uv run python -m pytest tests/capsem-bootstrap/")

    assert install_tools_pos != -1
    assert b3sum_pos != -1
    assert bootstrap_pos != -1
    assert install_tools_pos < b3sum_pos < bootstrap_pos


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
    assert "[binary, \"run\", \"capsem-doctor\"]" in source


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
        PROJECT_ROOT / "scripts" / "mock_server_impl.py",
        PROJECT_ROOT / "tests" / "helpers" / "mock_server.py",
        PROJECT_ROOT / "guest" / "artifacts" / "capsem_bench" / "__main__.py",
        PROJECT_ROOT / "guest" / "artifacts" / "capsem_bench" / "helpers.py",
    ]

    for path in current_files:
        text = path.read_text()
        assert OLD_DEBUG_CRATE not in text
        assert "debug_upstream" not in text
        assert "CAPSEM_BENCH_MOCK_SERVER_PROTOCOL_BASE_URL" not in text

    assert (PROJECT_ROOT / "crates" / OLD_DEBUG_CRATE).exists() is False
    assert (PROJECT_ROOT / "crates" / "capsem-mock-server").exists() is False
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
    just = (PROJECT_ROOT / "justfile").read_text()

    generate_pos = workflow.find("bash scripts/generate-settings.sh")
    first_frontend_build_pos = workflow.find("cd frontend && pnpm run build")
    frontend_check_pos = workflow.find("pnpm run check")

    assert generate_pos != -1
    assert first_frontend_build_pos != -1
    assert frontend_check_pos != -1
    assert generate_pos < first_frontend_build_pos
    assert generate_pos < frontend_check_pos
    assert "bash scripts/generate-settings.sh" in just
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
        assert "codesign --sign - --identifier \"$identifier\"" in script
        for identifier in expected:
            assert identifier in script


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
    reqwest_line = next(
        line for line in manifest.splitlines() if line.startswith("reqwest = ")
    )
    assert 'version = "0.12"' in reqwest_line
    assert "rustls-tls-webpki-roots" in reqwest_line
    assert '"rustls"' not in reqwest_line

    service_manifest = (PROJECT_ROOT / "crates" / "capsem-service" / "Cargo.toml").read_text()
    ort_line = next(
        line for line in service_manifest.splitlines() if line.startswith("ort = ")
    )
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
    assert "println!(\"Service stopped.\");" in body
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
    unreleased = changelog.split("## [Unreleased]", maxsplit=1)[1].split(
        "\n## [", maxsplit=1
    )[0]

    assert "Disabled the macOS Keychain-backed credential broker store" in unreleased
    assert "file-backed durable storage" in unreleased
    assert "Added credential broker plugin support with Keychain-backed storage" not in unreleased
    assert "single `org.capsem.credentials` Keychain vault item" not in unreleased
    assert "credential store/keychain" not in unreleased


def test_release_docs_identify_body_blobs_as_forensic_truth() -> None:
    telemetry = (
        PROJECT_ROOT
        / "docs"
        / "src"
        / "content"
        / "docs"
        / "architecture"
        / "session-telemetry.md"
    ).read_text()
    network = (
        PROJECT_ROOT
        / "docs"
        / "src"
        / "content"
        / "docs"
        / "security"
        / "network-isolation.md"
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


def test_pr_ci_python_coverage_is_not_a_monolithic_vm_tree_rerun() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    coverage_step = workflow.split("- name: Python schema tests with coverage", maxsplit=1)[1].split(
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
    block = workflow.split("- name: Python integration tests (non-VM suites)", maxsplit=1)[
        1
    ].split("# Verify all integration test suites", maxsplit=1)[0]

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
    source = (PROJECT_ROOT / "crates" / "capsem-core" / "src" / "hypervisor" / "kvm" / "checkpoint.rs").read_text()
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


def test_mock_server_has_no_rust_fixture_crate() -> None:
    root_cargo = (PROJECT_ROOT / "Cargo.toml").read_text()
    cli_cargo = (PROJECT_ROOT / "crates" / "capsem" / "Cargo.toml").read_text()

    assert "crates/capsem-mock-server" not in root_cargo
    assert "capsem-mock-server" not in cli_cargo
    assert "capsem_mock_server" not in (PROJECT_ROOT / "crates" / "capsem" / "src" / "main.rs").read_text()


def test_serial_benchmark_release_proofs_are_not_env_gated() -> None:
    benchmark = PROJECT_ROOT / "tests" / "capsem-serial" / "test_mock_server_protocol_benchmark.py"
    source = benchmark.read_text()

    assert "CAPSEM_RUN_MOCK_SERVER_PROTOCOL_BENCH" not in source
    assert "pytest.skip(" not in source
    assert "total_requests = 10" not in source
    assert 'CAPSEM_BENCH_TOTAL_REQUESTS", "10"' not in source
    assert 'CAPSEM_BENCH_CONCURRENCY", "1"' not in source
    assert '"capsem-bench",' in source
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
    assert "benchmarks\" / \"capsem-bench\"" in baseline


def test_integration_script_has_no_live_ai_provider_escape_hatch() -> None:
    source = (PROJECT_ROOT / "scripts" / "integration_test.py").read_text()

    assert "GEMINI_API_KEY" not in source
    assert "GOOGLE_API_KEY" not in source
    assert "googleapis.com" not in source
    assert "include_gemini_probe" not in source


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

    assert offenders == [], "legacy AI-provider builder rail still exists:\n" + "\n".join(
        offenders
    )


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
    assert offenders == [], "admin/registry config authority still exists:\n" + "\n".join(
        offenders
    )


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
    launch_pos = init.find("chroot /newroot \"$AGENT_PATH\"")

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
    source = (
        PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_runtimes.py"
    ).read_text()

    forbidden_fragments = [
        "pip install six",
        "uv pip install wheel",
        "uv pip install humanize",
        "npm install -g cowsay",
        "npm install lodash",
        "apt-get update",
        "apt-get install -y -qq htop",
    ]
    for fragment in forbidden_fragments:
        assert fragment not in source

    assert "--no-index" in source
    assert "file:" in source
    assert "dpkg-deb --build" in source
    assert "--python /root/.venv/bin/python" in source


def test_guest_virtiofs_pip_probe_is_hermetic() -> None:
    source = (
        PROJECT_ROOT / "guest" / "artifacts" / "diagnostics" / "test_virtiofs.py"
    ).read_text()

    assert "pip install --quiet cowsay" not in source
    assert "import cowsay" not in source
    assert "pip install --no-index" in source
    assert "ZipFile" in source
