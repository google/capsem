"""Keep package-owned tests with the build system and product acceptance at root."""

from __future__ import annotations

import subprocess
import tomllib
from collections.abc import Iterable, Mapping
from pathlib import Path
from typing import Any

BUILD_SYSTEM_ROOT = Path(__file__).resolve().parents[1]
REPOSITORY_ROOT = BUILD_SYSTEM_ROOT.parent
POLICY = Path(__file__).with_name("test_ownership.toml")

OWNER_PREFIXES = {
    "gate": "build_system/tests/gate/",
    "image": "build_system/tests/image/",
    "packaging": "build_system/tests/packaging/",
    "policy": "build_system/tests/policy/",
    "release": "build_system/tests/release/",
    "release_site": "build_system/tests/release_site/",
    "scripts": "build_system/tests/scripts/",
}
PACKAGE_LOCAL_TEST_SOURCES = frozenset({"build_system/release_site/src/lib/release-data.test.ts"})

# Boundary guards and support files created during the migration, or moved from
# non-test roots, were not part of the starting root-test ledger. Declare them
# exactly instead of weakening reconciliation for the 176 inventoried files.
BOUNDARY_FILES = frozenset(
    {
        "build_system/tests/gate/test_ci_tool_module_boundary.py",
        "build_system/tests/cache/test_admission.py",
        "build_system/tests/cache/test_admission_state.py",
        "build_system/tests/cache/test_cli.py",
        "build_system/tests/cache/test_direct_environment.py",
        "build_system/tests/cache/test_docker_anchor_inventory.py",
        "build_system/tests/cache/test_failure_artifacts.py",
        "build_system/tests/cache/test_gitimpact.py",
        "build_system/tests/cache/test_inventory.py",
        "build_system/tests/cache/test_objects.py",
        "build_system/tests/cache/test_operations.py",
        "build_system/tests/cache/test_paths.py",
        "build_system/tests/cache/test_planner.py",
        "build_system/tests/cache/test_policy.py",
        "build_system/tests/cache/test_pythonenv.py",
        "build_system/tests/cache/test_runtime_adapters.py",
        "build_system/tests/cache/test_runtime_control.py",
        "build_system/tests/cache/test_registry.py",
        "build_system/tests/cache/test_stats.py",
        "build_system/tests/cache/test_telemetry.py",
        "build_system/tests/cache/test_tool_adapters.py",
        "build_system/tests/cache/test_tools.py",
        "build_system/tests/cache/test_verdicts.py",
        "build_system/tests/cache/test_views.py",
        "build_system/tests/gate/test_doctor_tool_commands.py",
        "build_system/tests/gate/test_doctor_tool_module_boundary.py",
        "build_system/tests/gate/test_gate_module_boundary.py",
        "build_system/tests/gate/test_gate_cache_layout.py",
        "build_system/tests/gate/test_gate_cache_tooling.py",
        "build_system/tests/gate/test_gate_qualification_reuse.py",
        "build_system/tests/gate/test_gate_pycache_authority.py",
        "build_system/tests/gate/test_gate_run_transfer.py",
        "build_system/tests/gate/test_gate_test_admission.py",
        "build_system/tests/gate/test_bounded_process_tree.py",
        "build_system/tests/gate/test_candidate_profile_content.py",
        "build_system/tests/gate/test_host_docker_ownership.py",
        "build_system/tests/gate/test_rust_coverage_ratchet.py",
        "build_system/tests/gate/test_web_tool_module_boundary.py",
        "build_system/tests/helpers/__init__.py",
        "build_system/tests/helpers/injection_test.py",
        "build_system/tests/helpers/integration_test.py",
        "build_system/tests/helpers/mock_server.py",
        "build_system/tests/helpers/protocol_fixture_recorder.py",
        "build_system/tests/helpers/prove_installed_shell.py",
        "build_system/tests/image/test_image_module_boundary.py",
        "build_system/tests/image/test_componentcache.py",
        "build_system/tests/image/test_guest_binary_source_contract.py",
        "build_system/tests/packaging/test_linux_packaging_boundary.py",
        "build_system/tests/packaging/test_macos_packaging_boundary.py",
        "build_system/tests/packaging/test_shared_packaging_boundary.py",
        "build_system/tests/policy/test_policy_modules.py",
        "build_system/tests/release/test_release_module_boundary.py",
        "build_system/tests/release/test_release_foundation_tool_boundary.py",
        "build_system/tests/release/test_release_foundation_tools.py",
        "build_system/tests/release/test_installed_failure_logs.py",
        "build_system/tests/release/test_release_orchestration_tool_boundary.py",
        "build_system/tests/release/test_release_staging_tool_boundary.py",
        "build_system/tests/release/test_release_verification_tool_boundary.py",
        "build_system/tests/release/test_remote_ci_verdict_contract.py",
        "build_system/tests/scripts/test_dependency_audit.py",
        "build_system/tests/scripts/test_rust_affected.py",
        "build_system/tests/conftest.py",
        "build_system/tests/test_ownership.toml",
        "build_system/tests/test_project_boundary.py",
        "build_system/tests/test_reproducible_sdist.py",
        "build_system/tests/test_test_ownership.py",
    }
)


def _tracked_paths() -> frozenset[str]:
    return frozenset(
        subprocess.run(
            ("git", "ls-files"),
            cwd=REPOSITORY_ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.splitlines()
    )


def _rows() -> list[dict[str, Any]]:
    document = tomllib.loads(POLICY.read_text(encoding="utf-8"))
    assert document.get("version") == 1
    rows = document.get("owned")
    assert isinstance(rows, list) and rows
    return rows


def _problems(rows: Iterable[Mapping[str, Any]], tracked: frozenset[str]) -> list[str]:
    problems: list[str] = []
    sources: list[str] = []
    targets: list[str] = []

    for index, row in enumerate(rows):
        owner = row.get("owner")
        source = row.get("source")
        target = row.get("target")
        if not all(isinstance(value, str) and value for value in (owner, source, target)):
            problems.append(f"row {index}: owner, source, and target must be non-empty strings")
            continue

        assert isinstance(owner, str)
        assert isinstance(source, str)
        assert isinstance(target, str)
        sources.append(source)
        targets.append(target)

        prefix = OWNER_PREFIXES.get(owner)
        if prefix is None:
            problems.append(f"{source}: unknown build-system test owner {owner!r}")
        elif not target.startswith(prefix):
            problems.append(f"{source}: {owner} test targets the wrong owner path {target}")
        if not source.startswith("tests/") and source not in PACKAGE_LOCAL_TEST_SOURCES:
            problems.append(
                f"{source}: migration source must be under root tests/ or be an "
                "explicit package-local test"
            )
        if not target.startswith("build_system/tests/"):
            problems.append(f"{source}: package-owned target escapes build_system/tests/: {target}")
        if target.startswith("config/") or "/config/fixtures/" in target:
            problems.append(f"{source}: test fixture targets product config: {target}")
        if source == target:
            problems.append(f"{source}: migration source and target are identical")
        if source in tracked:
            problems.append(f"{source}: package-owned test/support file remains under root tests/")
        if target not in tracked:
            problems.append(f"{target}: declared build-system test/support target is missing")

    for label, paths in (("source", sources), ("target", targets)):
        duplicates = sorted({path for path in paths if paths.count(path) > 1})
        if duplicates:
            problems.append(f"duplicate {label} paths: {duplicates}")

    owned_targets = set(targets) | set(BOUNDARY_FILES)
    actual_build_tests = {path for path in tracked if path.startswith("build_system/tests/")}
    unowned = sorted(actual_build_tests - owned_targets)
    missing_boundaries = sorted(BOUNDARY_FILES - tracked)
    if unowned:
        problems.append(f"unclassified build-system test/support files: {unowned}")
    if missing_boundaries:
        problems.append(f"declared build-system boundary files are missing: {missing_boundaries}")
    return sorted(set(problems))


def test_package_owned_test_inventory_is_fully_migrated() -> None:
    problems = _problems(_rows(), _tracked_paths())
    assert not problems, "test ownership is incomplete:\n" + "\n".join(problems[:40])


def test_test_ownership_rejects_wrong_roots_owners_and_unclassified_files() -> None:
    target = "build_system/tests/gate/test_probe.py"
    row = {"owner": "gate", "source": "tests/test_probe.py", "target": target}
    tracked = frozenset({target, *BOUNDARY_FILES})
    assert _problems((row,), tracked) == []

    wrong_owner = {**row, "target": "build_system/tests/release/test_probe.py"}
    wrong_root = {**row, "target": "config/fixtures/test_probe.py"}
    stale_root = tracked | {"tests/test_probe.py"}
    rogue = tracked | {"build_system/tests/gate/test_rogue.py"}

    assert any("wrong owner path" in problem for problem in _problems((wrong_owner,), tracked))
    assert any("escapes build_system" in problem for problem in _problems((wrong_root,), tracked))
    assert any("remains under root" in problem for problem in _problems((row,), stale_root))
    assert any("unclassified" in problem for problem in _problems((row,), rogue))
