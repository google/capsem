"""Contracts for deterministic failures shared by smoke, test, and CI."""

import re
from pathlib import Path

import variables
import yaml
from capsem_builder.gate.tools.ci import justfile_graph as GRAPH

ROOT = Path(__file__).resolve().parents[3]
JUSTFILE = (ROOT / "justfile").read_text(encoding="utf-8")


def _planned(module: str) -> str:
    """What a module's plan would run, rendered.

    Replaces grepping `_test-candidate-run`, which no longer exists.
    """
    import argparse

    from capsem_builder.gate import cli  # noqa: F401 - imports every command module
    from capsem_builder.gate.command import GateCommand
    from capsem_builder.gate.proc import Runner

    return (
        GateCommand.registry[module](
            Runner(ROOT),
            argparse.Namespace(dry_run=False, graph=False, timing=False),
        )
        .plan()
        .describe()
    )


def _recipe(name: str) -> str:
    lines = JUSTFILE.splitlines()
    start = next(
        index
        for index, line in enumerate(lines)
        if line.startswith((f"{name}:", f"{name} "))
    )
    end = next(
        (
            index
            for index in range(start + 1, len(lines))
            if lines[index]
            and not lines[index].startswith((" ", "\t", "#"))
        ),
        len(lines),
    )
    return "\n".join(lines[start:end])


def test_python_tests_use_the_tracked_lowercase_justfile_name() -> None:
    offenders: list[str] = []
    wrong_case_path = re.compile(r'\b(?:PROJECT_ROOT|ROOT)\s*/\s*["\']Justfile["\']')
    for path in (ROOT / "tests").rglob("*.py"):
        if wrong_case_path.search(path.read_text(encoding="utf-8")):
            offenders.append(path.relative_to(ROOT).as_posix())

    assert offenders == [], (
        "case-insensitive local filesystems hide these Linux CI failures: "
        + ", ".join(offenders)
    )


def test_the_public_fast_gate_is_the_shared_module_itself() -> None:
    """`fast-test` is incomplete feedback and dispatches its module directly.

    The public name has to be the whole shared gate, not a reduced copy. It
    was a reduced copy: `fast-test` ran only the source-checks module while
    `fast-gate.yaml` ran that *and* the compiled-checks module, so the recipe
    advertised as "the fast gate itself" was half of what CI called by that
    name. It now runs both, and CI calls this recipe rather than the internals.
    """
    fast_test = variables.block(variables.FAST_TEST)
    planned = _planned("test-fast")

    assert "uv run --project build_system --frozen capsem-gate test-fast" in fast_test
    assert "incomplete" in fast_test
    assert "just focus-test" in fast_test
    assert "just release-profile" in fast_test
    assert "just release-binaries" in fast_test
    assert fast_test.strip().count("\n") == 1, (
        f"{variables.FAST_TEST} is one message plus one fast-gate dispatch: {fast_test!r}"
    )
    for required in (
        "build_system/scripts/audit/check-source-syntax.py",
        "build_system/scripts/audit/check-cargo-audit.py",
        "build_system/scripts/audit/audit-pnpm-bulk.py",
        "build_system/scripts/audit/audit-python-lock.py",
        "cargo clippy --workspace --all-targets -- -D warnings",
        # Both halves of what was one `frontend` target, named in full. The
        # bare prefix would have gone on passing against `frontend-build`
        # alone, which is the failure mode this whole surface exists to avoid:
        # an assertion that holds while the thing it names has gone.
        "check-web-surface.sh frontend-build",
        "check-web-surface.sh frontend-verify",
        "check-web-surface.sh release-site",
        "check-web-surface.sh release-channel",
    ):
        assert required in planned, f"the fast plan does not run {required}"


def test_fast_release_contracts_do_not_depend_on_ignored_build_outputs() -> None:
    """The cheap contract module must not need what the artifacts module makes.

    `test_materialized_profile_payload.py` reads a materialized catalog, so it
    belongs to the artifacts module. Its source-only counterpart must not reach
    for the same directory, or the cheap gate starts depending on a build.
    """
    materialized = "tests/capsem-build-chain/test_materialized_profile_payload.py"
    source_contract = (
        ROOT / "tests/capsem-build-chain/test_profile_payload_contract.py"
    ).read_text(encoding="utf-8")

    assert (ROOT / materialized).is_file()
    assert f"--ignore={materialized}" in _planned("test-release-contracts")
    assert materialized in _planned("test-artifacts")
    assert "MATERIALIZED_PROFILES_DIR" not in source_contract


def test_every_pnpm_cache_owner_materializes_its_store() -> None:
    offenders: list[str] = []
    for workflow_path in sorted((ROOT / ".github/workflows").glob("*.yaml")):
        workflow = yaml.safe_load(workflow_path.read_text(encoding="utf-8"))
        for job_name, job in (workflow.get("jobs") or {}).items():
            steps = job.get("steps") or []
            owns_pnpm_cache = any(
                isinstance(step, dict)
                and str(step.get("uses", "")).startswith("actions/setup-node@")
                and (step.get("with") or {}).get("cache") == "pnpm"
                for step in steps
            )
            if not owns_pnpm_cache:
                continue
            commands = "\n".join(
                str(step.get("run", "")) for step in steps if isinstance(step, dict)
            )
            # A `just` recipe creates the store just as surely as a literal
            # `pnpm install`; follow the recipe graph rather than accepting one
            # recipe by name, which left every other just-driven job unable to
            # cache at all.
            creates_store = (
                "pnpm" in commands and " install" in commands
            ) or GRAPH.shell_reaches_pnpm(commands, JUSTFILE)
            if not creates_store:
                offenders.append(
                    f"{workflow_path.relative_to(ROOT).as_posix()}:{job_name}"
                )

    assert offenders == [], (
        "cache-enabled setup-node jobs must create their pnpm store before "
        "the post-job save step: " + ", ".join(offenders)
    )
