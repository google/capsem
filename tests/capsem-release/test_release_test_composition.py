from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys


PROJECT_ROOT = Path(__file__).resolve().parents[2]
JUSTFILE = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")


def _source_contract_tests() -> tuple[str, ...]:
    match = re.search(
        r"(?ms)^    SOURCE_CONTRACT_TESTS=\(\n(?P<body>.*?)^    \)\n",
        JUSTFILE,
    )
    assert match is not None, "Justfile must define SOURCE_CONTRACT_TESTS"
    tests = tuple(
        line.strip()
        for line in match.group("body").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    )
    assert tests
    assert all(test.startswith("tests/") for test in tests)
    return tests


SOURCE_CONTRACT_TESTS = _source_contract_tests()


def _recipe(name: str) -> str:
    lines = JUSTFILE.splitlines()
    start = next(
        index
        for index, line in enumerate(lines)
        if line.startswith(f"{name}:") or line.startswith(f"{name} ")
    )
    end = len(lines)
    for index in range(start + 1, len(lines)):
        line = lines[index]
        if line and not line.startswith((" ", "\t", "#")):
            end = index
            break
    return "\n".join(lines[start:end])


def _workflow_job(path: str, name: str) -> str:
    lines = (PROJECT_ROOT / path).read_text(encoding="utf-8").splitlines()
    start = lines.index(f"  {name}:")
    end = next(
        (
            index
            for index in range(start + 1, len(lines))
            if lines[index].startswith("  ")
            and not lines[index].startswith("    ")
            and lines[index].endswith(":")
        ),
        len(lines),
    )
    return "\n".join(lines[start:end])


def _source_digest_module():
    script = PROJECT_ROOT / "scripts" / "source-state-digest.py"
    spec = importlib.util.spec_from_file_location("source_state_digest", script)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_local_test_composes_all_checked_in_modules_after_rebuilding_assets() -> None:
    public = _recipe("test")
    local = _recipe("_test-candidate")

    assert "just _test-fast" in public
    assert public.index("just _test-fast") < public.index("scripts/with-gate-colima.sh")
    expected = (
        "just _test-static",
        "just _test-artifacts",
        "just _test-functional",
        "just _test-glowup",
        "just _test-recipes",
    )
    positions = [local.index(command) for command in expected]
    assert positions == sorted(positions)
    assert "CAPSEM_TEST_MODULE=all" not in local
    assert "scripts/source-state-digest.py" in _recipe("test")


def test_private_release_modules_select_one_shared_runner() -> None:
    expected = {
        "_test-fast": "fast",
        "_test-static": "static",
        "_test-artifacts": "artifacts",
        "_test-functional": "functional",
        "_test-glowup": "glowup",
        "_test-release-contracts": "release-contracts",
    }

    for recipe, module in expected.items():
        assert f"CAPSEM_TEST_MODULE={module} just _test-candidate-run" in _recipe(recipe)

    runner = _recipe("_test-candidate-run")
    assert "fast|static|artifacts|functional|glowup|release-contracts" in runner
    assert '"all"' not in runner
    for module in expected.values():
        assert f"module_enabled {module}" in runner


def test_fast_module_owns_every_cheap_failure_before_colima_or_artifact_work() -> None:
    public = _recipe("test")
    fast = _recipe("_test-fast")
    runner = _recipe("_test-candidate-run")

    for required in (
        "scripts/check-source-syntax.py",
        "just _test-release-contracts",
        "scripts/check-cargo-audit.py",
        "scripts/audit-pnpm-bulk.py",
        "scripts/audit-python-lock.sh",
        "uv run ruff check .",
        "uv run ty check src/capsem",
        "cargo clippy --workspace --all-targets -- -D warnings",
        "bash scripts/check-web-surface.sh frontend",
        "bash scripts/check-web-surface.sh release-site",
    ):
        assert required in fast or required in runner

    assert public.index("just _test-fast") < public.index("scripts/with-gate-colima.sh")
    assert "_bootstrap" not in fast
    assert "_check-assets" not in fast
    assert "_pack-initrd" not in fast
    assert "module_enabled fast" in runner


def test_release_static_module_never_bootstraps_or_builds_profile_assets() -> None:
    static = _recipe("_test-static")

    assert "_bootstrap" not in static.splitlines()[0]
    assert "uv sync" in static
    assert "just _bound-docker-test-storage" in static
    assert "_check-generated-settings" in static.splitlines()[0]
    for forbidden in (
        "_build-assets",
        "_build-kernel",
        "_build-rootfs",
        "_check-assets",
        "_pack-initrd",
    ):
        assert forbidden not in static


def test_functional_module_materializes_its_gitignored_settings_fixture() -> None:
    functional = _recipe("_test-functional")

    assert "_generate-settings" in functional.splitlines()[0]
    assert 'if [ -z "${CAPSEM_RELEASE_INPUT_DIR:-}" ]; then' in functional
    assert "just _sign" in functional
    assert functional.index("just _sign") < functional.index(
        "CAPSEM_TEST_MODULE=functional"
    )
    for forbidden in (
        "_build-assets",
        "_build-kernel",
        "_build-rootfs",
        "_cross-compile",
    ):
        assert forbidden not in functional.splitlines()[0]


def test_modules_retain_complete_named_quality_gates() -> None:
    runner = _recipe("_test-candidate-run")

    for required in (
        "scripts/check-cargo-audit.py",
        "scripts/audit-pnpm-bulk.py",
        "cargo clippy --workspace --all-targets -- -D warnings",
        "bash scripts/check-web-surface.sh frontend",
        "cargo llvm-cov --workspace --bins --lib --tests",
        "tests/capsem-mcp/test_state_transitions.py",
        "tests/ironbank/test_route_health.py",
        "scripts/injection_test.py",
        "scripts/integration_test.py",
        "test_capsem_bench_baseline.py",
        "scripts/local-release-glowup.py",
        "just _gate-install",
        "tests/capsem-build-chain/",
        "tests/capsem-release/",
    ):
        assert required in runner


def test_release_contract_module_does_not_reenter_source_build_suites() -> None:
    runner = _recipe("_test-candidate-run")
    release_contracts = runner[runner.index("if module_enabled release-contracts;") :]
    functional = runner[
        runner.index("if module_enabled functional;") :
        runner.index("if module_enabled glowup;")
    ]

    assert "tests/capsem-build-chain/" in release_contracts
    assert "tests/capsem-release/" in release_contracts
    for artifact_test in (
        "test_cargo_build.py",
        "test_codesign.py",
        "test_full_chain.py",
        "test_manifest_regen.py",
        "test_pack_initrd.py",
    ):
        assert f"--ignore=tests/capsem-build-chain/{artifact_test}" in release_contracts
        assert f"tests/capsem-build-chain/{artifact_test}" in runner
    assert "tests/test_*contract.py" in release_contracts
    for source_test in SOURCE_CONTRACT_TESTS:
        assert source_test in runner
    assert '"${SOURCE_CONTRACT_TESTS[@]}"' in release_contracts
    assert "tests/capsem-recipes/" not in release_contracts
    assert "tests/capsem-recipes/" in _recipe("_test-recipes")
    assert "--ignore-glob=tests/test_*contract.py" in functional
    assert '"${SOURCE_CONTRACT_IGNORE_ARGS[@]}"' in functional
    assert "Python: release site shared-dist tests" not in functional


def test_every_root_workflow_or_just_source_test_is_owned_by_the_fast_gate() -> None:
    inventory = set(SOURCE_CONTRACT_TESTS)
    inspected_source_contracts = set()

    for path in (PROJECT_ROOT / "tests").glob("test_*.py"):
        source = path.read_text(encoding="utf-8")
        if not any(
            needle in source
            for needle in (".github/workflows", '"Justfile"', '"justfile"')
        ):
            continue
        relative = path.relative_to(PROJECT_ROOT).as_posix()
        if path.name.endswith("_contract.py"):
            continue
        inspected_source_contracts.add(relative)

    assert inspected_source_contracts <= inventory


def test_parallel_coverage_state_is_kept_out_of_the_source_tree() -> None:
    runner = _recipe("_test-candidate-run")

    assert (
        'export COVERAGE_FILE="{{justfile_directory()}}/target/coverage/.coverage"'
        in runner
    )
    assert 'mkdir -p "$(dirname "$COVERAGE_FILE")"' in runner


def test_functional_coverage_replays_cheap_contracts_after_the_early_gate() -> None:
    runner = _recipe("_test-candidate-run")
    functional = runner[
        runner.index("if module_enabled functional;") :
        runner.index("if module_enabled glowup;")
    ]
    coverage = functional[
        functional.index('echo "=== Python: non-serial tests (n=4 parallel) ==="') :
        functional.index('echo "=== Python: host snapshot tests (serial) ==="')
    ]

    assert "--cov=src/capsem" in coverage
    assert "--cov-fail-under=90" in coverage
    assert '"${SOURCE_CONTRACT_IGNORE_ARGS[@]}"' not in coverage
    assert "--ignore-glob=tests/test_*contract.py" not in coverage


def test_release_contract_module_owns_release_site_dependencies(tmp_path: Path) -> None:
    contracts = _recipe("_test-release-contracts")
    install = _recipe("_release-site-pnpm-install")

    assert "_release-site-pnpm-install" in contracts.splitlines()[0]
    assert "release-site" in install
    assert "pnpm install --frozen-lockfile" in install
    for workflow_path, job in (
        (".github/workflows/release.yaml", "test-binary-pairing"),
        (".github/workflows/release-assets.yaml", "test-profile-pairing"),
    ):
        pairing = _workflow_job(workflow_path, job)
        assert "cache: pnpm" in pairing
        assert "release-site/pnpm-lock.yaml" in pairing

    real_just = shutil.which("just")
    assert real_just is not None
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    trace = tmp_path / "trace"
    for command, body in (
        ("pnpm", 'printf "pnpm:%s:%s\\n" "$PWD" "$*" >> "$TRACE"'),
        (
            "just",
            'printf "just:%s:%s\\n" "${CAPSEM_TEST_MODULE:-}" "$*" >> "$TRACE"',
        ),
    ):
        executable = fake_bin / command
        executable.write_text(f"#!/bin/sh\n{body}\n", encoding="utf-8")
        executable.chmod(0o755)

    result = subprocess.run(
        [real_just, "_test-release-contracts"],
        cwd=PROJECT_ROOT,
        env={
            **os.environ,
            "PATH": f"{fake_bin}{os.pathsep}{os.environ['PATH']}",
            "TRACE": str(trace),
        },
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )

    assert result.returncode == 0, result.stdout + result.stderr
    assert trace.read_text(encoding="utf-8").splitlines() == [
        f"pnpm:{PROJECT_ROOT / 'release-site'}:install --frozen-lockfile",
        "just:release-contracts:_test-candidate-run",
    ]


def test_static_module_orders_fast_checks_before_docker_preflight() -> None:
    runner = _recipe("_test-candidate-run")

    audit = runner.index("scripts/check-cargo-audit.py")
    frontend = runner.index("bash scripts/check-web-surface.sh frontend")
    clippy = runner.index("cargo clippy --workspace --all-targets -- -D warnings")
    install_preflight = runner.index("just _test-install-harness-preflight")

    assert audit < install_preflight
    assert frontend < clippy < install_preflight


def test_static_module_audits_the_locked_python_graph_fail_closed() -> None:
    runner = _recipe("_test-candidate-run")
    pyproject = (PROJECT_ROOT / "pyproject.toml").read_text(encoding="utf-8")
    audit_script = (PROJECT_ROOT / "scripts/audit-python-lock.sh").read_text(
        encoding="utf-8"
    )

    launch = runner.index("bash scripts/audit-python-lock.sh & PID_PYTHON_AUDIT=$!")
    wait = runner.index(
        'wait $PID_PYTHON_AUDIT || { echo "Python dependency audit failed"; FAIL=1; }'
    )
    install_preflight = runner.index("just _test-install-harness-preflight")

    assert launch < wait < install_preflight
    assert '"pip-audit>=' in pyproject
    for required in (
        "uv export",
        "--locked",
        "--no-emit-project",
        "uv run pip-audit",
        "-s osv",
        "--require-hashes",
        "--disable-pip",
    ):
        assert required in audit_script


def test_reusable_fast_gate_installs_workspace_static_prerequisites() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/fast-gate.yaml").read_text(encoding="utf-8")
    prerequisites = workflow.index("Install Linux workspace lint prerequisites")
    shared_module = workflow.index("Run shared static module")

    assert prerequisites < shared_module
    for package in (
        "musl-tools",
        "pkg-config",
        "libssl-dev",
        "libgtk-3-dev",
        "libwebkit2gtk-4.1-dev",
        "libayatana-appindicator3-dev",
        "libxdo-dev",
    ):
        assert package in workflow[prerequisites:shared_module]
    shared_block = workflow[shared_module:]
    assert "CC_x86_64_unknown_linux_musl: musl-gcc" in shared_block
    assert "CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER: musl-gcc" in shared_block


def test_standalone_functional_scripts_use_the_project_python() -> None:
    for recipe in ("_test-candidate-run", "smoke"):
        body = _recipe(recipe)
        for script in ("scripts/injection_test.py", "scripts/integration_test.py"):
            assert f"python3 {script}" not in body
            assert f"uv run python {script}" in body


def test_release_glowup_consumes_the_exact_pairing_environment() -> None:
    runner = _recipe("_test-candidate-run")
    adapter = (PROJECT_ROOT / "scripts" / "local-release-glowup.py").read_text(encoding="utf-8")

    assert "scripts/local-release-glowup.py" in runner
    for variable in (
        "CAPSEM_RELEASE_CHANNEL",
        "CAPSEM_RELEASE_TRANSITION",
        "CAPSEM_RELEASE_BEFORE_MANIFEST",
        "CAPSEM_RELEASE_AFTER_MANIFEST",
        "CAPSEM_RELEASE_BEFORE_PACKAGE",
        "CAPSEM_RELEASE_BEFORE_PROFILE_INPUTS",
        "CAPSEM_RELEASE_AFTER_PROFILE_INPUTS",
    ):
        assert variable in adapter
    assert "validate_exact_release_pairing(args)" in adapter


def test_release_glowup_also_runs_pre_activation_channel_switches() -> None:
    runner = _recipe("_test-candidate-run")
    glowup = runner.split(
        'if [ -n "${CAPSEM_RELEASE_PACKAGE:-}" ]; then', maxsplit=1
    )[1].split("else", maxsplit=1)[0]

    assert glowup.count("scripts/local-release-glowup.py") == 2
    assert "--work-dir target/release-module-glowup" in glowup
    assert "--work-dir target/release-module-channel-switch" in glowup
    for variable in (
        "CAPSEM_RELEASE_CHANNEL",
        "CAPSEM_RELEASE_TRANSITION",
        "CAPSEM_RELEASE_BEFORE_MANIFEST",
        "CAPSEM_RELEASE_AFTER_MANIFEST",
        "CAPSEM_RELEASE_BEFORE_PACKAGE",
        "CAPSEM_RELEASE_BEFORE_PROFILE_INPUTS",
        "CAPSEM_RELEASE_AFTER_PROFILE_INPUTS",
        "CAPSEM_RELEASE_PROFILE",
        "CAPSEM_RELEASE_CANDIDATE_PROFILE_PUBLICATION",
        "CAPSEM_RELEASE_PUBLICATION_BASE",
    ):
        assert f"-u {variable}" in glowup


def test_standalone_local_glowup_materializes_config_without_release_builders() -> None:
    runner = _recipe("_test-candidate-run")

    release_branch = runner.index('if [ -n "${CAPSEM_RELEASE_PACKAGE:-}" ]; then')
    local_branch = runner.index("else", release_branch)
    local_materialize = runner.index("just _materialize-config", local_branch)
    first_cross_compile = runner.index("just _cross-compile arm64", local_branch)

    assert 'LOCAL_CONFIG_ROOT="target/config"' in runner
    assert 'find "$LOCAL_CONFIG_ROOT/profiles"' in runner
    assert local_branch < local_materialize < first_cross_compile
    assert "just _build-kernel" not in runner
    assert "just _build-rootfs" not in runner
    assert "just _build-images" not in runner


def test_release_artifact_module_boots_manifest_selected_profile_bytes_without_builders() -> None:
    runner = _recipe("_test-candidate-run")
    artifact_branch = runner[
        runner.index("if module_enabled artifacts; then") :
        runner.index("# ---- Stage 5: Python pytest")
    ]

    assert "scripts/prove-release-profile-assets.py" in artifact_branch
    assert '--input-dir "$CAPSEM_RELEASE_INPUT_DIR"' in artifact_branch
    assert '--profile "$CAPSEM_RELEASE_PROFILE"' in artifact_branch
    for forbidden in (
        "just _build-assets",
        "just _build-kernel",
        "just _build-rootfs",
        "just _build-images",
        "just _cross-compile",
    ):
        assert forbidden not in artifact_branch


def test_functional_module_runs_every_selected_profile_without_rebuilding() -> None:
    runner = _recipe("_test-candidate-run")

    assert "scripts/release-test-profiles.py" in runner
    assert '--manifest "$TEST_ASSETS/manifest.json"' in runner
    assert 'for TEST_PROFILE in "${TEST_PROFILES[@]:1}"' in runner
    assert 'CAPSEM_TEST_PROFILE="$BASE_PROFILE"' in runner
    assert 'CAPSEM_TEST_PROFILE="$TEST_PROFILE"' in runner
    assert '-m "(integration or mcp or e2e) and not serial"' in runner
    assert '--profile "$BASE_PROFILE"' in runner
    assert '--profile "$TEST_PROFILE"' in runner
    assert "tests/capsem-mcp/test_state_transitions.py" in runner
    assert "tests/ironbank/test_route_health.py" in runner
    assert "tests/capsem-serial/test_capsem_bench_baseline.py" in runner
    assert "build-assets" not in runner


def test_release_functional_helpers_never_hide_host_binary_builds() -> None:
    helper_paths = (
        "scripts/mock_server.py",
        "tests/helpers/gateway.py",
        "tests/capsem-service/test_profile_assets.py",
        "tests/capsem-admin/test_profile_materialization.py",
        "tests/ironbank/test_profile_asset_readiness.py",
        "tests/test_capsem_bench_rust.py",
    )

    for path in helper_paths:
        source = (PROJECT_ROOT / path).read_text(encoding="utf-8")
        assert "ensure_host_test_binary" in source, path
        assert '["cargo", "build"' not in source, path


def test_pulled_binary_functional_preflight_requires_release_inputs_not_build_tree(
    tmp_path: Path,
) -> None:
    from tests.conftest import (
        _missing_required_artifacts,
        _required_artifacts_for_run,
    )

    source_agent = tmp_path / "target/linux-agent/x86_64"
    release_inputs = tmp_path / "verified-profile-inputs"
    release_package = tmp_path / "Capsem_1.5_amd64.deb"
    release_binary = tmp_path / "target/debug/capsem"
    required = _required_artifacts_for_run(
        {
            "CAPSEM_RELEASE_INPUT_DIR": str(release_inputs),
            "CAPSEM_RELEASE_PACKAGE": str(release_package),
            "CAPSEM_TEST_BINARY": str(release_binary),
        },
        {
            "assets/manifest.json": tmp_path / "assets/manifest.json",
            "target/linux-agent/<arch>": source_agent,
        },
    )

    assert "target/linux-agent/<arch>" not in required
    assert (
        required["verified release input report"]
        == release_inputs / "release-inputs.json"
    )
    assert required["manifest-selected release package"] == release_package
    assert required["manifest-selected test binary"] == release_binary
    assert _missing_required_artifacts(
        {"CAPSEM_REQUIRE_ARTIFACTS": "1"},
        required,
    ) == [
        "assets/manifest.json",
        "verified release input report",
        "manifest-selected release package",
        "manifest-selected test binary",
    ]

    source_required = _required_artifacts_for_run(
        {},
        {"target/linux-agent/<arch>": source_agent},
    )
    assert source_required == {"target/linux-agent/<arch>": source_agent}


def test_pulled_binary_static_gate_owns_source_agent_assertions() -> None:
    runner = _recipe("_test-candidate-run")
    cross_compile = runner[
        runner.index("# ---- Stage 2: cross-arch agent cross-compile") :
        runner.index("# ---- Stage 2b: Linux Rust platform parity")
    ]

    assert 'if [ "$TEST_MODULE" = "static" ]; then' in cross_compile
    assert "tests/capsem-bootstrap/test_cross_compile.py" in cross_compile
    assert "tests/capsem-security/test_binary_perms.py" in cross_compile
    assert "target/linux-agent/$HOST_AGENT_ARCH" in cross_compile


def test_source_state_digest_covers_dirty_and_untracked_nonignored_files(
    tmp_path: Path,
) -> None:
    subprocess.run(("git", "init", "-q"), cwd=tmp_path, check=True)
    tracked = tmp_path / "tracked.txt"
    tracked.write_text("one\n", encoding="utf-8")
    (tmp_path / ".gitignore").write_text("ignored.txt\n", encoding="utf-8")
    subprocess.run(("git", "add", "tracked.txt", ".gitignore"), cwd=tmp_path, check=True)
    module = _source_digest_module()

    initial = module.source_state_digest(tmp_path)
    tracked.write_text("two\n", encoding="utf-8")
    dirty = module.source_state_digest(tmp_path)
    assert dirty != initial

    untracked = tmp_path / "untracked.txt"
    untracked.write_text("present\n", encoding="utf-8")
    with_untracked = module.source_state_digest(tmp_path)
    assert with_untracked != dirty

    (tmp_path / "ignored.txt").write_text("ignored change\n", encoding="utf-8")
    assert module.source_state_digest(tmp_path) == with_untracked

    if os.name != "nt":
        untracked.chmod(0o755)
        assert module.source_state_digest(tmp_path) != with_untracked
