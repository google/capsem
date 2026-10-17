"""SDK acceptance consumes the environment prepared before the network seals."""

import ast
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
ENVIRONMENT_RATIONALE = (
    "The gate prepares SDK dependencies before acceptance. uv run without "
    "--no-sync can rebuild editable packages after a snapshot refresh and fetch "
    "Hatchling inside the sealed namespace; see skills/dev-testing/SKILL.md."
)


def _syncing_runs(source: str) -> list[int]:
    failures = []
    for node in ast.walk(ast.parse(source)):
        if not isinstance(node, (ast.List, ast.Tuple)):
            continue
        argv = [part.value if isinstance(part, ast.Constant) else None for part in node.elts]
        if argv[:2] != ["uv", "run"]:
            continue
        options = argv[:argv.index("python")] if "python" in argv else argv
        if "--no-sync" not in options:
            failures.append(node.lineno)
    return failures


def test_ironbank_sdk_launchers_use_the_prepared_environment():
    failures = {
        str(path.relative_to(ROOT)): lines
        for path in sorted((ROOT / "tests/ironbank").glob("test_*.py"))
        if (lines := _syncing_runs(path.read_text()))
    }
    assert not failures, f"{failures}: {ENVIRONMENT_RATIONALE}"


@pytest.mark.parametrize("source", [
    '["uv", "run", "--frozen", "python", "probe.py"]',
    '("uv", "run", "--project", "sdk/python", "python")',
    '["uv", "run", "python", "probe.py", "--no-sync"]',
    '# --no-sync\n["uv", "run", "python"]',
])
def test_guard_rejects_runtime_sync(source):
    assert _syncing_runs(source), ENVIRONMENT_RATIONALE


@pytest.mark.parametrize("source", [
    '["uv", "run", "--frozen", "--no-sync", "python", "probe.py"]',
    '("uv", "run", "--project", "sdk/python", "--no-sync", "python")',
    '["uv", "sync", "--project", "sdk/python"]',
    '["node", "probe.mjs"]',
])
def test_guard_accepts_prepared_runs_and_other_commands(source):
    assert not _syncing_runs(source), ENVIRONMENT_RATIONALE
