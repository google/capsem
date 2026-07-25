from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import subprocess
import sys


PROJECT_ROOT = Path(__file__).resolve().parents[2]
JUSTFILE = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")


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


def _source_digest_module():
    script = PROJECT_ROOT / "scripts" / "source-state-digest.py"
    spec = importlib.util.spec_from_file_location("source_state_digest", script)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_local_test_composes_all_checked_in_modules_after_rebuilding_assets() -> None:
    local = _recipe("_test-candidate")

    assert "_check-assets _pack-initrd _materialize-config" in local.splitlines()[0]
    assert "CAPSEM_TEST_MODULE=all just _test-candidate-run" in local
    assert "scripts/source-state-digest.py" in _recipe("test")


def test_private_release_modules_select_one_shared_runner() -> None:
    expected = {
        "_test-static": "static",
        "_test-artifacts": "artifacts",
        "_test-functional": "functional",
        "_test-glowup": "glowup",
        "_test-release-contracts": "release-contracts",
    }

    for recipe, module in expected.items():
        assert f"CAPSEM_TEST_MODULE={module} just _test-candidate-run" in _recipe(recipe)

    runner = _recipe("_test-candidate-run")
    assert "all|static|artifacts|functional|glowup|release-contracts" in runner
    for module in expected.values():
        assert f"module_enabled {module}" in runner


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


def test_modules_retain_complete_named_quality_gates() -> None:
    runner = _recipe("_test-candidate-run")

    for required in (
        "cargo audit",
        "scripts/audit-pnpm-bulk.py --project-dir frontend",
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
        "tests/capsem-build-chain/ tests/capsem-release/",
    ):
        assert required in runner


def test_static_module_orders_fast_checks_before_docker_preflight() -> None:
    runner = _recipe("_test-candidate-run")

    audit = runner.index("cargo audit")
    frontend = runner.index("bash scripts/check-web-surface.sh frontend")
    clippy = runner.index("cargo clippy --workspace --all-targets -- -D warnings")
    install_preflight = runner.index("just _test-install-harness-preflight")

    assert audit < install_preflight
    assert frontend < install_preflight
    assert clippy < install_preflight


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
