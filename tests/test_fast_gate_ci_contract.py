"""Contracts for deterministic failures shared by smoke, test, and CI."""

import importlib.util
from pathlib import Path
import re
import sys

import yaml


ROOT = Path(__file__).resolve().parents[1]
JUSTFILE = (ROOT / "justfile").read_text(encoding="utf-8")


def _justfile_graph():
    script = ROOT / "scripts" / "justfile-graph.py"
    spec = importlib.util.spec_from_file_location("justfile_graph", script)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


GRAPH = _justfile_graph()


def _recipe(name: str) -> str:
    lines = JUSTFILE.splitlines()
    start = next(
        index
        for index, line in enumerate(lines)
        if line.startswith(f"{name}:") or line.startswith(f"{name} ")
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


def test_smoke_reuses_the_complete_shared_fast_gate() -> None:
    smoke = _recipe("smoke")
    fast = _recipe("_test-fast")
    runner = _recipe("_test-candidate-run")

    assert smoke.index("just _test-fast") < smoke.index("just _check-assets")
    assert "scripts/check-source-syntax.py" in fast
    assert "just _test-release-contracts" in fast
    for required in (
        "scripts/check-cargo-audit.py",
        "scripts/audit-pnpm-bulk.py",
        "scripts/audit-python-lock.sh",
        "cargo clippy --workspace --all-targets -- -D warnings",
        "bash scripts/check-web-surface.sh frontend",
        "bash scripts/check-web-surface.sh release-site",
    ):
        assert required in runner


def test_fast_release_contracts_do_not_depend_on_ignored_build_outputs() -> None:
    release_contracts = _recipe("_test-candidate-run")
    materialized_test = (
        ROOT / "tests/capsem-build-chain/test_materialized_profile_payload.py"
    )
    source_contract = (
        ROOT / "tests/capsem-build-chain/test_profile_payload_contract.py"
    ).read_text(encoding="utf-8")

    assert materialized_test.is_file()
    assert (
        "--ignore=tests/capsem-build-chain/test_materialized_profile_payload.py"
        in release_contracts
    )
    assert (
        release_contracts.count(
            "tests/capsem-build-chain/test_materialized_profile_payload.py"
        )
        == 2
    )
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
