"""One spelling of "run pytest", and what may not share a machine with what.

Sixteen call sites assembled their own flags and agreed by hand: the same
`--tb=short`, the same four `--ignore` directories, the same
`CAPSEM_REQUIRE_ARTIFACTS=1`. Sixteen copies of an agreement are sixteen
chances for one to differ, with nothing to notice which.

The contention is the part worth testing. In shell it was achieved by
placement -- run after the `wait`, and hope nobody adds a job below. Here it is
declared, and the plan honours it however the steps are written.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

from capsem.gate import config as gate_config
from capsem.gate import pytestsuite

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)


def _argv(suite) -> list[str]:
    return suite.argv(CONFIG)


# ---------------------------------------------------------------------------
# Contention
# ---------------------------------------------------------------------------


def test_host_snapshot_tests_claim_the_one_service() -> None:
    """Production has one service and one service-scoped save/restore lock.
    An xdist worker per service does not reproduce that."""
    suite = pytestsuite.host_snapshot(CONFIG, profile="code")

    assert [e.name for e in suite.contends] == ["host_service"]
    assert not suite.parallel


def test_benchmarks_claim_the_vz_launch_budget() -> None:
    """Two files launching VMs at once measure each other, not Capsem."""
    for suite in (
        pytestsuite.timing(CONFIG, profile="code"),
        pytestsuite.benchmark(CONFIG, profile="code"),
    ):
        assert [e.name for e in suite.contends] == ["apple_vz"]
        assert not suite.parallel


def test_timing_rail_owns_the_route_health_probe_once() -> None:
    """A focused wrapper must not make one noisy measurement count twice."""
    route_owners = tuple(path for path in CONFIG.suites.pytest.serial_paths if "route_" in path)

    assert route_owners == ("tests/ironbank/test_route_health.py",)
    wrapper = PROJECT_ROOT / "tests/ironbank/test_route_latency.py"
    assert "def test_" not in wrapper.read_text(encoding="utf-8")


def test_the_broad_suite_claims_the_binaries_it_runs_against() -> None:
    """`cargo build --workspace` atomically replaces the codesigned binaries a
    concurrent VM test is using, so anything that rebuilds must not overlap."""
    suite = pytestsuite.broad(CONFIG, profile="code")

    assert [e.name for e in suite.contends] == ["workspace_binaries"]


def test_every_exclusive_a_suite_claims_is_declared() -> None:
    """A step that invents its own contends with nothing."""
    for build in (
        pytestsuite.broad,
        pytestsuite.host_snapshot,
        pytestsuite.timing,
        pytestsuite.benchmark,
        pytestsuite.compatibility,
    ):
        for exclusive in build(CONFIG, profile="code").contends:
            assert CONFIG.exclusive(exclusive.name) is exclusive


# ---------------------------------------------------------------------------
# What each invocation actually is
# ---------------------------------------------------------------------------


def test_the_broad_suite_runs_four_at_a_time_by_file() -> None:
    """`--dist=loadfile` keeps per-file fixtures on one worker, which matters
    when the fixtures build VMs."""
    argv = _argv(pytestsuite.broad(CONFIG, profile="code"))

    assert "-n" in argv
    assert "--dist=loadfile" in argv


def test_the_broad_suite_skips_what_rebuilds_the_binaries_under_it() -> None:
    """`capsem-recipes` invokes `cargo build --workspace` from inside pytest,
    which replaces the binaries the concurrent VM tests are running."""
    argv = _argv(pytestsuite.broad(CONFIG, profile="code"))

    assert "--ignore=tests/capsem-recipes" in argv
    assert "--ignore=tests/capsem-install" in argv


def test_the_serial_snapshot_files_are_excluded_from_the_parallel_run() -> None:
    """Otherwise they run twice, once in the way that does not reproduce
    production."""
    broad = _argv(pytestsuite.broad(CONFIG, profile="code"))

    for path in CONFIG.suites.pytest.host_snapshot_serial:
        assert f"--ignore={path}" in broad


def test_the_snapshot_suite_runs_exactly_those_files() -> None:
    argv = _argv(pytestsuite.host_snapshot(CONFIG, profile="code"))

    for path in CONFIG.suites.pytest.host_snapshot_serial:
        assert path in argv


def test_every_suite_fails_closed_without_artifacts() -> None:
    """Before collection, rather than passing vacuously against a tree whose
    assets were never built."""
    variable = CONFIG.suites.pytest.require_artifacts

    for build in (pytestsuite.broad, pytestsuite.host_snapshot, pytestsuite.timing):
        assert build(CONFIG, profile="code").environment(CONFIG)[variable] == "1"


def test_every_suite_carries_the_profile_it_is_proving() -> None:
    variable = CONFIG.suites.pytest.profile_variable

    for build in (pytestsuite.broad, pytestsuite.compatibility):
        assert build(CONFIG, profile="co-work").environment(CONFIG)[variable] == "co-work"


def test_vm_suites_do_not_bypass_the_manifest_content_selector() -> None:
    """Every VM fixture runs in a subprocess with CAPSEM_ASSETS_DIR and
    CAPSEM_PROFILES_DIR. A module-level checkout literal silently opts that
    fixture out and makes a profile lane boot the stale canonical tree."""
    roots = (
        "capsem-bootstrap",
        "capsem-e2e",
        "capsem-mcp",
        "capsem-security",
        "capsem-service",
        "ironbank",
    )
    forbidden = (
        'ASSETS_DIR = PROJECT_ROOT / "assets"',
        'PROFILES_DIR = PROJECT_ROOT / "target" / "config" / "profiles"',
    )
    offenders = [
        f"{path.relative_to(PROJECT_ROOT)}: {needle}"
        for root in roots
        for path in (PROJECT_ROOT / "tests" / root).rglob("*.py")
        for needle in forbidden
        if needle in path.read_text(encoding="utf-8")
    ]

    assert not offenders, "VM suites bypass the selected manifest input:\n  " + "\n  ".join(
        offenders
    )


def test_only_the_broad_suite_measures_coverage() -> None:
    """Four suites all writing `codecov-python.xml` would each overwrite the
    last, and the file would report whichever finished last."""
    measured = [
        build
        for build in (
            pytestsuite.broad,
            pytestsuite.host_snapshot,
            pytestsuite.timing,
            pytestsuite.benchmark,
            pytestsuite.compatibility,
        )
        if build(CONFIG, profile="code").coverage
    ]

    assert measured == [pytestsuite.broad]


def test_the_compatibility_run_skips_the_contracts_already_proved() -> None:
    """The broad suite proves every source contract once. Repeating them per
    profile would triple the slowest part of the gate to re-prove a constant."""
    argv = _argv(pytestsuite.compatibility(CONFIG, profile="co-work"))

    assert "--ignore=tests/test_gate_plan.py" in argv
    assert "--ignore-glob=tests/test_*contract.py" in argv


def test_the_compatibility_run_keeps_the_vm_owned_markers() -> None:
    argv = _argv(pytestsuite.compatibility(CONFIG, profile="co-work"))

    assert "(integration or mcp or e2e) and not serial" in argv


def test_the_timing_suite_leaves_the_recorded_baseline_to_its_own_step() -> None:
    """It is the one whose numbers get published, so it runs by itself."""
    timing = _argv(pytestsuite.timing(CONFIG, profile="code"))
    baseline = _argv(pytestsuite.benchmark(CONFIG, profile="code"))

    assert CONFIG.suites.pytest.benchmark_deselect in timing
    assert CONFIG.suites.pytest.benchmark_baseline in baseline


def test_a_timing_suite_does_not_stop_at_the_first_failure() -> None:
    """One slow probe should not hide the other five."""
    budget = CONFIG.suites.pytest.stop_at_first
    assert budget not in _argv(pytestsuite.timing(CONFIG, profile="code"))
    assert budget in _argv(pytestsuite.broad(CONFIG, profile="code"))


def test_collection_is_cache_free_strict_and_artifact_independent() -> None:
    """Collection is a source-shape proof, not a VM or built-output proof."""
    collection = pytestsuite.collection(CONFIG)
    rendered = " ".join(collection.render())

    assert "uv run python -m pytest tests/" in rendered
    for flag in (
        "--collect-only",
        "-qq",
        "-p no:cacheprovider",
        "--strict-config",
        "--strict-markers",
    ):
        assert flag in rendered
    assert CONFIG.suites.pytest.require_artifacts not in rendered
    assert "--cov" not in rendered
    assert "-n" not in rendered


def test_every_serial_node_has_a_non_broad_execution_rail() -> None:
    """Broad excludes ``serial``; every such node needs another owner."""
    settings = CONFIG.suites.pytest
    result = subprocess.run(
        [
            "uv",
            "run",
            "python",
            "-m",
            "pytest",
            settings.root,
            *settings.collection_flags,
            "-q",
            "-m",
            "serial",
        ],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        check=False,
        # Collection reads source, not artifacts, so it must not demand them.
        # The autouse fixture strips `CAPSEM_RELEASE_*` from this process so a
        # unit test cannot mistake itself for a release -- and the child then
        # inherits an environment that still says artifacts are required while
        # no longer saying which lane is running. It concluded it was a local
        # build and asked for `target/linux-agent`, which a pulled lane
        # correctly does not have, fifteen minutes into a release gate.
        env={k: v for k, v in os.environ.items() if k != "CAPSEM_REQUIRE_ARTIFACTS"},
    )
    # Reported rather than raised bare. `check=True` throws the child's stderr
    # away, so a collection failure in a release lane said only that a command
    # exited non-zero -- after fourteen minutes, on a machine nobody can attach
    # to, with the reason in the output it had just discarded.
    assert result.returncode == 0, (
        f"collecting serial nodes failed ({result.returncode}):\n"
        f"stdout:\n{result.stdout[-2000:]}\nstderr:\n{result.stderr[-2000:]}"
    )
    node_paths = {
        line.split("::", 1)[0].split(": ", 1)[0]
        for line in result.stdout.splitlines()
        if line.startswith(settings.root)
    }
    release_contracts = {
        str(path.relative_to(PROJECT_ROOT))
        for path in PROJECT_ROOT.glob(CONFIG.modules.contract_glob)
    }
    file_owners = {
        *CONFIG.suites.source_contract,
        *CONFIG.modules.build_chain_artifact_tests,
        *release_contracts,
    }
    directory_owners = (
        *settings.serial_paths,
        *CONFIG.modules.release_suites,
        CONFIG.install.suite.path,
        *(part for part in CONFIG.candidate.recipe_suite if part.startswith(settings.root)),
    )
    unowned = sorted(
        path
        for path in node_paths
        if path not in file_owners
        and not any(
            path == owner.rstrip("/") or path.startswith(owner.rstrip("/") + "/")
            for owner in directory_owners
        )
    )

    assert not unowned, (
        "serial nodes are excluded from the broad suite and have no configured "
        f"execution rail: {unowned}"
    )


def test_every_suite_is_labelled_by_what_it_proves_and_for_which_profile() -> None:
    """The label is what the run log and the timing report show, so
    `pytest` five times would make the summary useless."""
    labels = {
        build(CONFIG, profile="code").label
        for build in (
            pytestsuite.broad,
            pytestsuite.host_snapshot,
            pytestsuite.timing,
            pytestsuite.benchmark,
            pytestsuite.compatibility,
        )
    }

    assert len(labels) == 5
    assert all(label.endswith(".code") for label in labels)
