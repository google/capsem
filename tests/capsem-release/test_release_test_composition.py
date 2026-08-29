from __future__ import annotations

import importlib.util
import os
import shutil
import subprocess
import sys
from pathlib import Path

import yaml
from capsem_builder.gate import config as _gate_config
from capsem_builder.gate.tools.ci import justfile_graph as GRAPH
from capsem_builder.release.tools import local_release_glowup
from helpers.workflow_contract import (
    parsed_commands,
    workflow_job_source,
    workflow_jobs,
)

PROJECT_ROOT = Path(__file__).resolve().parents[2]
JUSTFILE = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")


CONFIG = _gate_config.load(PROJECT_ROOT)

#: The inventory, from the one place that owns it. It was a 47-line array in
#: the justfile until `_test-candidate-run` was ported out of shell, and the
#: duplication is how eleven gate test files ended up in neither the fast
#: module nor the exclusion that keeps them out of the VM matrix.
SOURCE_CONTRACT_TESTS = tuple(CONFIG.suites.source_contract)


def test_the_source_contract_inventory_has_one_authority() -> None:
    """`config/gate.toml` owns it, and nothing else may keep a copy.

    The justfile carried a second array until the modules were ported, and
    that duplication is how eleven gate test files ended up in neither the
    fast module nor the exclusion that keeps them out of the VM matrix.
    """
    assert "SOURCE_CONTRACT_TESTS" not in JUSTFILE
    assert SOURCE_CONTRACT_TESTS


def _command(module: str):
    """The command object, for asking what it holds as well as what it does."""
    import argparse

    from capsem_builder.gate import cli  # noqa: F401 - imports every command module
    from capsem_builder.gate.command import GateCommand
    from helpers.gate import RecordingRunner

    return GateCommand.registry[module](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
    )


def _qualification(**overrides):
    """A complete release state, or the local one.

    Handed to the command rather than exported into `os.environ`, because a
    release state is indivisible: the modules no longer each read a variable,
    and a test that set one of the three was reproducing a hybrid the gate now
    refuses outright.
    """
    from capsem_builder.gate.qualification import from_environment as qualification_for

    return qualification_for(CONFIG, overrides)


#: The two release lanes, spelled once. A binary lane resolves every profile
#: the manifest names; a profile lane is publishing exactly one.
BINARY_LANE = _qualification(
    CAPSEM_RELEASE_INPUT_DIR="target/release-inputs",
    CAPSEM_RELEASE_PACKAGE="dist/capsem_0.0.0_arm64.deb",
)
PROFILE_LANE = _qualification(
    CAPSEM_RELEASE_INPUT_DIR="target/release-inputs",
    CAPSEM_RELEASE_PACKAGE="dist/capsem_0.0.0_arm64.deb",
    CAPSEM_RELEASE_PROFILE="code",
)


def _planned(module: str, qualification=None) -> str:
    """What a module's plan would run, rendered.

    Replaces grepping `_test-candidate-run`, which no longer exists. This is
    the stronger question: the text search noticed a line that stopped being
    written, while this notices a step that stopped running.
    """
    import argparse

    from capsem_builder.gate import cli  # noqa: F401 - imports every command module
    from capsem_builder.gate.command import GateCommand
    from helpers.gate import RecordingRunner

    command = GateCommand.registry[module](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
        qualification=qualification,
    )
    return command.plan().describe()


def _planned_labels(module: str) -> tuple[str, ...]:
    """Every step a module's plan contains, in an order the graph permits."""
    import argparse

    from capsem_builder.gate import cli  # noqa: F401 - registers every command
    from capsem_builder.gate.command import GateCommand
    from helpers.gate import RecordingRunner

    return (
        GateCommand.registry[module](
            RecordingRunner(PROJECT_ROOT),
            argparse.Namespace(dry_run=False, graph=False, timing=False),
        )
        .plan()
        .labels
    )


def _all_modules() -> str:
    """Every module's plan, in both of the shapes a module can take.

    A recipe body carried every branch as text, so grepping it found gates
    that only a release lane reaches. A plan carries the branch it took, so
    the release-lane shapes are rendered explicitly rather than assumed.
    """

    plans = [
        _planned(name)
        for name in (
            "test-fast",
            "test-static",
            "test-artifacts",
            "test-functional",
            "test-glowup",
            "test-release-contracts",
        )
    ]

    plans += [
        _planned("test-glowup", PROFILE_LANE),
        _planned("test-artifacts", PROFILE_LANE),
    ]
    return "\n".join(plans)


def _recipe(name: str) -> str:
    lines = JUSTFILE.splitlines()
    start = next(
        index for index, line in enumerate(lines) if line.startswith((f"{name}:", f"{name} "))
    )
    end = len(lines)
    for index in range(start + 1, len(lines)):
        line = lines[index]
        if line and not line.startswith((" ", "\t", "#")):
            end = index
            break
    return "\n".join(lines[start:end])


def _workflow_job(path: str, name: str) -> str:
    return workflow_job_source(
        (PROJECT_ROOT / path).read_text(encoding="utf-8"), name
    )


SETUP_JUST = "extractions/setup-just"
SETUP_PNPM = "pnpm/action-setup"
SETUP_NODE = "actions/setup-node"
SETUP_UV = "astral-sh/setup-uv"
PROVISIONED_WORKFLOWS = (
    ".github/workflows/ci.yaml",
    ".github/workflows/release.yaml",
    ".github/workflows/release-assets.yaml",
)

def _tests_requiring_just() -> tuple[str, ...]:
    return tuple(
        sorted(
            str(path.relative_to(PROJECT_ROOT))
            for path in PROJECT_ROOT.glob("tests/**/*.py")
            if 'shutil.which("just")' in path.read_text(encoding="utf-8")
        )
    )


def _job_shell(job: str) -> str:
    """The shell a job runs, without `uses:` action references."""
    document = yaml.safe_load(job) or {}
    definition = document
    return "\n".join(
        step["run"]
        for step in definition.get("steps") or ()
        if isinstance(step, dict) and isinstance(step.get("run"), str)
    )


def _selects_a_just_dependent_test(shell: str, just_tests: tuple[str, ...]) -> bool:
    """A directory argument selects everything under it; a file selects itself."""
    selected_paths = (
        argument
        for command in parsed_commands(shell, origin="CI job")
        for argument in command.argv
        if argument.startswith("tests/")
    )
    for selected in selected_paths:
        if selected.endswith("/"):
            if any(test.startswith(selected) for test in just_tests):
                return True
        elif selected in just_tests:
            return True
    return False


def _workflow_job_names(path: str) -> tuple[str, ...]:
    return tuple(workflow_jobs(PROJECT_ROOT / path))


def test_every_ci_job_provisions_the_tools_its_own_steps_invoke() -> None:
    """Local `just test-clean` runs where every tool is already on PATH, so it cannot
    observe that a CI job never installed one. That blind spot is what let
    `test` lose `just` and `test-install` lose `pnpm` while every local gate
    stayed green. This test moves CI tool provisioning into the checked-in
    contract so the fast local gate fails first.

    Recipe reachability deliberately over-approximates: it follows the justfile
    dependency graph without modelling shell branches, so a job may be required
    to install a tool one of its conditional paths would skip. That bias is the
    safe one -- a spare tool costs seconds, a missing one is a red gate.

    It only covers tools whose need is unconditional once reached (`just`,
    `uv`, `pnpm`, `node`). System packages like musl-tools are deliberately excluded:
    `_gate-linux-rust` reaches `doctor` statically but exits before it on
    Linux, so requiring them here would fail jobs that are already correct."""
    just_tests = _tests_requiring_just()
    assert just_tests, "the just-dependent test scan must find the release contracts"

    missing: list[str] = []
    recipes_reaching_uv = GRAPH.recipes_reaching(JUSTFILE, "uv")
    for path in PROVISIONED_WORKFLOWS:
        for name in _workflow_job_names(path):
            job = _workflow_job(path, name)
            shell = _job_shell(job)
            needs_just = bool(GRAPH.just_recipes(shell)) or _selects_a_just_dependent_test(
                shell, just_tests
            )
            needs_pnpm = GRAPH.shell_reaches_pnpm(shell, JUSTFILE)
            needs_uv = GRAPH.invokes(shell, "uv") or any(
                recipe in recipes_reaching_uv for recipe in GRAPH.just_recipes(shell)
            )
            for required, needed in (
                (SETUP_JUST, needs_just),
                (SETUP_PNPM, needs_pnpm),
                (SETUP_NODE, needs_pnpm),
                (SETUP_UV, needs_uv),
            ):
                if needed and required not in job:
                    missing.append(f"{path}::{name} invokes it but never installs {required}")

    assert not missing, "CI jobs missing tool provisioning:\n" + "\n".join(missing)


def _source_digest_module():
    script = PROJECT_ROOT / "build_system" / "scripts" / "build" / "source-state-digest.py"
    spec = importlib.util.spec_from_file_location("source_state_digest", script)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_local_test_composes_all_checked_in_modules_after_rebuilding_assets() -> None:
    """The fast module first, then the expensive one under the Colima wrapper.

    Read out of the recipe when `test:` was shell, and out of the plan now
    that `_test-candidate` is a command. The module order is edges, so it
    holds however the source is arranged.
    """
    assert "capsem-gate candidate" in _recipe("test-clean")

    # The fast phase precedes every expensive one, and Colima is given back on
    # every path. Both used to be read out of `candidate.py` as source text --
    # the module order as two variable names in the right sequence, and the
    # Colima lifecycle as a wrapper script name. They are an edge and a
    # resource now, and the resource covers the *whole* gate rather than the
    # half that happened to sit inside the wrapper.
    gate = list(_planned_labels("candidate"))
    assert next(i for i, s in enumerate(gate) if s.startswith("fast.")) < next(
        i for i, s in enumerate(gate) if s.startswith("static.")
    )
    from helpers.gate import RecordingRunner

    assert "colima" in {
        r.name for r in _command("candidate").resources(RecordingRunner(PROJECT_ROOT))
    }

    order = list(_planned_labels("test-candidate"))
    # The modules are phases of one plan now rather than four child processes,
    # so each is a namespace rather than a step name.
    expected = ("prepare.", "static.", "artifacts.", "functional.", "glowup.", "recipes")
    positions = [
        next(i for i, label in enumerate(order) if label.startswith(prefix)) for prefix in expected
    ]
    assert positions == sorted(positions)

    assert CONFIG.candidate.source_digest_script.endswith("source-state-digest.py")


def test_private_release_modules_select_one_shared_runner() -> None:
    """Each module recipe reaches exactly one runner and contains no logic.

    Two runners are legal while the port is in progress: the shell one for
    modules still inside `_test-candidate-run`, and `capsem-gate` for those
    already extracted. Neither may be a recipe that does the work itself.
    """
    expected = {
        "_test-source-checks": "fast",
        "_test-compiled-checks": "static",
        "_test-artifacts": "artifacts",
        "_test-functional": "functional",
        "_test-glowup": "glowup",
        "_test-release-contracts": "release-contracts",
    }

    for recipe, module in expected.items():
        body = _recipe(recipe)
        shell = f"CAPSEM_TEST_MODULE={module} just _test-candidate-run" in body
        ported = f"capsem-gate test-{module}" in body
        assert shell != ported, (
            f"{recipe} must reach exactly one runner; it has shell={shell} ported={ported}"
        )

    runner = _all_modules()
    assert '"all"' not in runner
    for recipe, module in expected.items():
        if f"capsem-gate test-{module}" in _recipe(recipe):
            continue
        assert f"module_enabled {module}" in runner


def test_fast_module_owns_every_cheap_failure_before_colima_or_artifact_work() -> None:
    public = _recipe("test-clean")
    fast = _recipe("_test-source-checks")
    planned = _planned("test-fast")

    for required in (
        "build_system/scripts/audit/check-source-syntax.py",
        "build_system/scripts/audit/check-cargo-audit.py",
        "build_system/scripts/audit/audit-pnpm-bulk.py",
        "build_system/scripts/audit/audit-python-lock.sh",
        # Ruff over the whole tree, and Ty over the strict builder package -- as
        # three steps, so a ruff failure no longer hides what ty would have
        # said. The explicit all-platform surface keeps the exact diagnostic
        # ratchet identical on Linux and macOS. The project/config flags keep
        # the nested build-system project explicit after the root facade is gone.
        "ruff check --config build_system/pyproject.toml .",
        (
            "ty check --project build_system --error-on-warning --python-platform all "
            "build_system/builder"
        ),
        "cargo clippy --workspace --all-targets -- -D warnings",
        "check-web-surface.sh frontend",
        "check-web-surface.sh release-site",
    ):
        assert required in planned, f"the fast plan does not run {required}"

    assert "just _test-release-contracts" in fast

    # The order lives in capsem_builder.gate.candidate now; see
    # test_local_test_composes_all_checked_in_modules_after_rebuilding_assets.
    assert "capsem-gate candidate" in public
    assert "_bootstrap" not in fast
    assert "_check-assets" not in fast
    assert "_pack-initrd" not in fast


def test_release_static_module_never_bootstraps_or_builds_profile_assets() -> None:
    static = _recipe("_test-compiled-checks")

    assert "_bootstrap" not in static.splitlines()[0]
    assert "just _bound-docker-test-storage" in static
    assert "_check-generated-settings" in static.splitlines()[0]
    assert "uv sync" in _planned("test-static") or "uv sync" in _planned("test-fast")
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

    # Signing moved into the module, where it is conditional on the same
    # release-input variable and ordered by an edge rather than by position.
    labels = _planned_labels("test-functional")
    # Composed rather than dispatched: one platform-shaped signing step stays
    # in the graph before the broad suite. Linux keeps the edge with no action;
    # the synthetic macOS contract separately proves the codesign actions.
    assert labels.index("functional.sign") < labels.index("functional.pytest.broad.code")
    for forbidden in (
        "_build-assets",
        "_build-kernel",
        "_build-rootfs",
        "_cross-compile",
    ):
        assert forbidden not in functional.splitlines()[0]


def test_modules_retain_complete_named_quality_gates() -> None:
    runner = _all_modules()

    for required in (
        "build_system/scripts/audit/check-cargo-audit.py",
        "build_system/scripts/audit/audit-pnpm-bulk.py",
        "cargo clippy --workspace --all-targets -- -D warnings",
        "bash scripts/check-web-surface.sh frontend",
        "cargo llvm-cov nextest --workspace --bins --lib --tests",
        "cargo test --workspace --doc",
        "tests/capsem-mcp/test_state_transitions.py",
        "tests/ironbank/test_route_health.py",
        "scripts/injection_test.py",
        "scripts/integration_test.py",
        "test_capsem_bench_baseline.py",
        "build_system/scripts/release/local-release-glowup.py",
        "install the exact package and prove the installed product",
        "tests/capsem-build-chain/",
        "tests/capsem-release/",
    ):
        assert required in runner


def test_release_contract_module_does_not_reenter_source_build_suites() -> None:
    """The cheap composition proof runs the suites that need no artifacts, and
    the artifacts module runs exactly the ones that do."""
    release_contracts = _planned("test-release-contracts")
    functional = _planned("test-functional")
    artifacts = _planned("test-artifacts")

    assert "tests/capsem-build-chain/" in release_contracts
    assert "tests/capsem-release/" in release_contracts
    for artifact_test in CONFIG.modules.build_chain_artifact_tests:
        assert f"--ignore={artifact_test}" in release_contracts
        assert artifact_test in artifacts

    # The glob is expanded before pytest sees it: pytest does not expand path
    # arguments, so passing the pattern through would collect nothing and the
    # module would pass vacuously.
    for pattern in CONFIG.modules.contract_globs:
        assert pattern not in release_contracts
    assert "build_system/tests/" in release_contracts

    for source_test in SOURCE_CONTRACT_TESTS:
        assert (PROJECT_ROOT / source_test).is_file()
        if not source_test.startswith("build_system/tests/"):
            assert source_test in release_contracts
    assert "tests/capsem-recipes" not in release_contracts
    assert "tests/capsem-recipes/" in _recipe("_test-recipes")

    for pattern in CONFIG.modules.contract_globs:
        assert f"--ignore-glob={pattern}" in functional
    for source_test in SOURCE_CONTRACT_TESTS[:3]:
        assert f"--ignore={source_test}" in functional


def test_every_root_workflow_or_just_source_test_is_owned_by_the_fast_gate() -> None:
    inventory = set(SOURCE_CONTRACT_TESTS)
    inspected_source_contracts = set()

    for path in (PROJECT_ROOT / "tests").glob("test_*.py"):
        source = path.read_text(encoding="utf-8")
        if not any(
            needle in source for needle in (".github/workflows", '"Justfile"', '"justfile"')
        ):
            continue
        relative = path.relative_to(PROJECT_ROOT).as_posix()
        if path.name.endswith("_contract.py"):
            continue
        inspected_source_contracts.add(relative)

    assert inspected_source_contracts <= inventory


def test_parallel_coverage_state_is_kept_out_of_the_source_tree() -> None:
    """Coverage state under `target/`, so the candidate tree stays
    byte-for-byte identical to the commit that was tested."""
    workspace = CONFIG.workspace

    assert workspace.coverage_file.startswith("target/")
    assert workspace.benchmark_root.startswith("target/")
    assert workspace.home.startswith("target/")

    from capsem_builder.gate.workspace import Workspace

    environment = Workspace(CONFIG).environment()
    assert environment["COVERAGE_FILE"].endswith(workspace.coverage_file)


def test_functional_coverage_replays_cheap_contracts_after_the_early_gate() -> None:
    """The broad suite measures coverage and does not skip the contracts.

    The compatibility runs skip them, because the broad run already proved
    them once and repeating a constant per profile triples the slowest part of
    the gate.
    """
    from capsem_builder.gate import pytestsuite

    broad = pytestsuite.broad(CONFIG, profile=CONFIG.suites.pytest.base_profile)
    argv = broad.argv(CONFIG)

    assert "--cov=build_system/builder" in argv
    # The floor is `fail_under` in pyproject's [tool.coverage.report], so any
    # run that reports inherits it. What this module must still do is measure.
    assert any(flag.startswith("--cov-report=") for flag in argv)

    for source_test in SOURCE_CONTRACT_TESTS[:3]:
        assert f"--ignore={source_test}" not in argv
    for pattern in CONFIG.modules.contract_globs:
        assert f"--ignore-glob={pattern}" not in argv


def test_release_contract_module_owns_release_site_dependencies(tmp_path: Path) -> None:
    contracts = _recipe("_test-release-contracts")
    install = _recipe("_release-site-pnpm-install")

    assert "_release-site-pnpm-install" in contracts.splitlines()[0]
    assert "build_system/release_site" in install
    assert "pnpm install --frozen-lockfile" in install
    for workflow_path, job in (
        (".github/workflows/release.yaml", "test-binary-pairing"),
        (".github/workflows/release-assets.yaml", "test-profile-pairing"),
    ):
        pairing = _workflow_job(workflow_path, job)
        assert "cache: pnpm" in pairing
        assert "build_system/release_site/pnpm-lock.yaml" in pairing
        assert "cd web/app && pnpm install --frozen-lockfile" in pairing
        assert "cd build_system/release_site && pnpm install --frozen-lockfile" in pairing

    real_just = shutil.which("just")
    assert real_just is not None
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    trace = tmp_path / "trace"
    for command, body in (
        ("pnpm", 'printf "pnpm:%s:%s\\n" "$PWD" "$*" >> "$TRACE"'),
        ("uv", 'printf "uv:%s:%s\\n" "$PWD" "$*" >> "$TRACE"'),
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
        capture_output=True,
        check=False,
    )

    assert result.returncode == 0, result.stdout + result.stderr
    trace_lines = trace.read_text(encoding="utf-8").splitlines()
    pnpm_command, pnpm_cwd, pnpm_args = trace_lines[0].split(":", maxsplit=2)
    assert pnpm_command == "pnpm"
    assert Path(pnpm_cwd).resolve() == (PROJECT_ROOT / "build_system" / "release_site").resolve()
    assert pnpm_args == "install --frozen-lockfile"
    # The recipe dispatches rather than implementing: one install, then the
    # gate command. Nothing between them, and no nested `just`.
    assert len(trace_lines) == 2
    gate_command, _cwd, gate_args = trace_lines[1].split(":", maxsplit=2)
    assert gate_command == "uv"
    assert gate_args == (
        "run --project build_system --frozen capsem-gate test-release-contracts"
    )


def test_static_module_orders_fast_checks_before_docker_preflight() -> None:
    """Cheap failures come back before anything starts a container.

    Asserted across modules now: the audits and clippy live in the fast plan,
    the Docker preflight lives in the static one, and `just test-clean` runs fast
    before static. In shell all three were regions of one file and the order
    was where the lines sat.
    """
    fast = _planned("test-fast")
    static = _planned("test-static")

    assert "build_system/scripts/audit/check-cargo-audit.py" in fast
    assert "check-web-surface.sh frontend" in fast
    assert fast.index("check-web-surface.sh frontend") < fast.index("cargo clippy")

    assert "build the network-denied install qualification image" in static
    assert "cargo clippy" not in static, "the lint gate belongs to the fast module"


def test_static_module_audits_the_locked_python_graph_fail_closed() -> None:
    """The Python dependency audit runs, and runs early.

    It used to be a backgrounded job whose exit status came back through a
    `wait` into a FAIL bit; now it is a step, so "did it run" and "did it
    pass" are the same question.
    """
    fast = _planned("test-fast")
    static = _planned("test-static")
    pyproject = (PROJECT_ROOT / "build_system/pyproject.toml").read_text(encoding="utf-8")
    audit_script = (
        PROJECT_ROOT / "build_system/scripts/audit/audit-python-lock.sh"
    ).read_text(encoding="utf-8")

    assert "build_system/scripts/audit/audit-python-lock.sh" in fast
    assert "build the network-denied install qualification image" in static
    assert '"pip-audit>=' in pyproject
    for required in (
        "uv export",
        "--locked",
        "--no-emit-project",
        "uv run --project build_system --frozen pip-audit",
        "-s osv",
        "--require-hashes",
        "--disable-pip",
    ):
        assert required in audit_script


def test_reusable_fast_gate_installs_workspace_static_prerequisites() -> None:
    ci = (PROJECT_ROOT / ".github/workflows/ci.yaml").read_text(encoding="utf-8")
    workflow = (PROJECT_ROOT / ".github/workflows/fast-gate.yaml").read_text(encoding="utf-8")
    prerequisites = workflow.index("Install Linux workspace lint prerequisites")
    shared_module = workflow.index("Run the complete fast gate")

    assert prerequisites < shared_module
    provision = "sudo python3 build_system/scripts/bootstrap/provision-linux-workspace.py --install apt"
    assert provision in workflow[prerequisites:shared_module]
    linux_coverage = ci.index("Unit tests (KVM backend) with coverage")
    assert provision in ci[:linux_coverage]
    shared_block = workflow[shared_module:]
    assert "CC_x86_64_unknown_linux_musl: musl-gcc" in shared_block
    assert "CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER: musl-gcc" in shared_block


def test_standalone_functional_scripts_use_the_project_python() -> None:
    """`python3` is whatever the machine has; on a release runner that is not
    the interpreter the lockfile pins."""
    for module in ("test-functional", "smoke"):
        planned = _planned(module)
        for script in ("scripts/injection_test.py", "scripts/integration_test.py"):
            assert f"uv run --project build_system --frozen python {script}" in planned
            assert f"python3 {script}" not in planned


def test_release_glowup_consumes_the_exact_pairing_environment() -> None:
    runner = _all_modules()
    owner = Path(local_release_glowup.__file__).read_text(encoding="utf-8")

    assert "build_system/scripts/release/local-release-glowup.py" in runner
    for variable in (
        "CAPSEM_RELEASE_CHANNEL",
        "CAPSEM_RELEASE_TRANSITION",
        "CAPSEM_RELEASE_BEFORE_MANIFEST",
        "CAPSEM_RELEASE_AFTER_MANIFEST",
        "CAPSEM_RELEASE_BEFORE_PACKAGE",
        "CAPSEM_RELEASE_BEFORE_PROFILE_INPUTS",
        "CAPSEM_RELEASE_AFTER_PROFILE_INPUTS",
    ):
        assert variable in owner
    assert "validate_exact_release_pairing(args)" in owner


def test_release_glowup_runs_one_exact_candidate_transition() -> None:
    """The exact pairing is the release transition; do not prove a second,
    synthetic candidate-to-itself channel switch in the hosted lane."""
    glowup = _planned("test-glowup", BINARY_LANE)

    assert glowup.count("build_system/scripts/release/local-release-glowup.py") == 1
    assert "--work-dir target/release-module-glowup" in glowup
    assert "glowup.channel-switch" not in glowup
    assert "target/release-module-channel-switch" not in glowup


def test_standalone_local_glowup_materializes_config_without_release_builders() -> None:
    """The package rail owns materializing the catalog `repack-deb.sh` reads.

    Asserted as a dependency rather than as a call before each cross-compile,
    which is what stops a new caller silently dropping it -- ordinary CI's
    install gate hit exactly that and failed with "no materialized profiles
    found".
    """
    runner = _all_modules()

    assert "_materialize-config" in _recipe("_cross-compile").splitlines()[0]
    assert "build the Linux release package for arm64" in _planned("test-glowup")
    for forbidden in ("_build-kernel", "_build-rootfs", "_build-images"):
        assert forbidden not in runner


def test_release_artifact_module_boots_manifest_selected_profile_bytes_without_builders() -> None:
    """A release lane verifies the bytes it pulled rather than rebuilding
    them; rebuilding would prove something about the source instead."""
    artifacts = _planned("test-artifacts", PROFILE_LANE)

    assert "build_system/scripts/release/prove-release-profile-assets.py" in artifacts
    assert "--input-dir target/release-inputs" in artifacts
    assert "--profile code" in artifacts
    for forbidden in ("capsem-gate assets", "_build-kernel", "_build-rootfs", "cross-compile"):
        assert forbidden not in artifacts


def test_functional_module_runs_every_selected_profile_without_rebuilding() -> None:
    """Every selected profile gets the VM-owned suites; the base profile also
    gets the broad one. That is the compatibility axis, not a reduced
    release-only substitute."""
    from capsem_builder.gate import profiles

    functional = _planned("test-functional")
    axis = profiles.selected(CONFIG)
    assert len(axis) >= 2, "the compatibility axis needs more than one profile"

    for profile in axis:
        assert f"CAPSEM_TEST_PROFILE={profile}" in functional
        assert f"--profile {profile}" in functional

    assert "(integration or mcp or e2e) and not serial" in functional
    assert "tests/capsem-mcp/test_state_transitions.py" in functional
    assert "tests/ironbank/test_route_health.py" in functional
    assert "tests/capsem-serial/test_capsem_bench_baseline.py" in functional
    assert "build-assets" not in functional


def test_release_functional_keeps_the_manifest_staged_input_selector() -> None:
    """The private IronBank tree is local-build output, never a release input."""
    for lane in (BINARY_LANE, PROFILE_LANE):
        functional = _planned("test-functional", lane)
        assert CONFIG.assets.test_root not in functional


def test_release_integration_follows_the_declared_staged_config_root(
    monkeypatch,
) -> None:
    """The legacy integration driver gets the same pulled catalog as pytest."""
    from capsem_builder.gate import vmproofs

    config_root = PROJECT_ROOT / "target" / "synthetic-release-config"
    monkeypatch.setenv(CONFIG.functional.config_root_variable, str(config_root))

    rendered = "\n".join(
        vmproofs.integration(CONFIG, profile=CONFIG.suites.pytest.base_profile).render()
    )

    expected = config_root / CONFIG.functional.profiles_subdir
    assert f"{CONFIG.environment.profiles_dir}={expected}" in rendered


def test_standalone_local_functional_uses_its_declared_canonical_inputs() -> None:
    """Only the composed candidate owns the private IronBank build fragment."""
    functional = _planned("test-functional")

    assert CONFIG.assets.test_root not in functional


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
    from conftest import (
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
    assert required["verified release input report"] == release_inputs / "release-inputs.json"
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
    """The static module owns the source-build assertions, once, right after
    it produces both guest-binary architectures."""
    static = _planned("test-static")

    assert "capsem-builder agent config/docker/image" in static
    assert "tests/capsem-bootstrap/test_cross_compile.py" in static
    assert "tests/capsem-security/test_binary_perms.py" in static
    assert f"{CONFIG.initrd.staging}/" in static
    for binary in CONFIG.initrd.binaries:
        assert binary in static


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


def test_source_state_digest_accepts_a_tracked_deletion(tmp_path: Path) -> None:
    subprocess.run(("git", "init", "-q"), cwd=tmp_path, check=True)
    tracked = tmp_path / "tracked.txt"
    tracked.write_text("present\n", encoding="utf-8")
    subprocess.run(("git", "add", "tracked.txt"), cwd=tmp_path, check=True)
    module = _source_digest_module()

    present = module.source_state_digest(tmp_path)
    tracked.unlink()

    assert module.source_state_digest(tmp_path) != present


def test_source_state_digest_ignores_the_generated_asset_selector(tmp_path: Path) -> None:
    """Asset selection is build output, not a mid-gate source mutation.

    ``AssetGate`` creates selectors below the target-owned output root only
    after the sealed install image has been built.  The shared source subject
    must exclude those selectors, while a retired checkout-root selector must
    remain visible as unexpected source state.
    """
    subprocess.run(("git", "init", "-q"), cwd=tmp_path, check=True)
    (tmp_path / ".gitignore").write_bytes((PROJECT_ROOT / ".gitignore").read_bytes())
    tracked = tmp_path / "tracked.txt"
    tracked.write_text("source\n", encoding="utf-8")
    subprocess.run(("git", "add", ".gitignore", "tracked.txt"), cwd=tmp_path, check=True)
    module = _source_digest_module()

    before = module.source_state_digest(tmp_path)
    selected = tmp_path / "target" / "assets" / "current"
    selected.parent.mkdir(parents=True)
    selected.symlink_to("arm64")

    assert module.source_state_digest(tmp_path) == before

    (tmp_path / "assets").symlink_to("target/assets")
    assert module.source_state_digest(tmp_path) != before


def test_source_state_digest_ignores_node_workspace_atomic_scratch(tmp_path: Path) -> None:
    """pnpm scratch is generated state, not a racing untracked source file."""
    subprocess.run(("git", "init", "-q"), cwd=tmp_path, check=True)
    (tmp_path / ".gitignore").write_bytes((PROJECT_ROOT / ".gitignore").read_bytes())
    tracked = tmp_path / "tracked.txt"
    tracked.write_text("source\n", encoding="utf-8")
    subprocess.run(("git", "add", ".gitignore", "tracked.txt"), cwd=tmp_path, check=True)
    module = _source_digest_module()
    before = module.source_state_digest(tmp_path)

    for workspace in CONFIG.toolchain.node_workspaces:
        scratch = tmp_path / workspace / "_tmp_123_0123456789abcdef"
        scratch.parent.mkdir(parents=True)
        scratch.write_text("pnpm atomic write\n", encoding="utf-8")

    assert module.source_state_digest(tmp_path) == before
