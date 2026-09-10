"""SDK checks must work on cold runners and publish coverage for all SDK source."""

from __future__ import annotations

from copy import deepcopy
from pathlib import Path
from typing import Any

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[2]
RATIONALE = "SDK tests need cached dependencies before the sandbox and measured CI/Codecov ownership."


def _documents() -> tuple[dict[str, Any], dict[str, Any], dict[str, Any]]:
    return tuple(yaml.safe_load((ROOT / path).read_text()) for path in (
        ".github/workflows/fast-gate.yaml", ".github/workflows/ci.yaml", "codecov.yml",
    ))


def _assert_owners(fast: dict[str, Any], ci: dict[str, Any], coverage: dict[str, Any]) -> None:
    steps = fast["jobs"]["static"]["steps"]
    prewarm = next(index for index, step in enumerate(steps)
                   if "uv sync --project sdk/python --frozen" in step.get("run", "").splitlines())
    sealed = next(index for index, step in enumerate(steps) if step.get("run") == "just fast-test")
    assert prewarm < sealed, RATIONALE
    steps = ci["jobs"]["test"]["steps"]
    test = next(step for step in steps if step.get("working-directory") == "sdk/python")
    assert test["run"] == "uv run --frozen pytest --junitxml=../../cache/target/coverage/junit/python-sdk.xml", RATIONALE
    assert test["env"]["COVERAGE_FILE"].endswith("/python-sdk/.coverage"), RATIONALE
    upload = next(step for step in steps if step.get("with", {}).get("flags") == "python-sdk")
    assert upload["with"]["files"] == "cache/target/coverage/python-sdk/coverage.xml", RATIONALE
    assert upload["uses"].startswith("codecov/codecov-action@"), RATIONALE
    assert coverage["flags"]["python-sdk"]["paths"] == ["sdk/python/capsem/**"], RATIONALE
    assert coverage["coverage"]["status"]["project"]["python-sdk"]["target"] == "90%", RATIONALE
    components = coverage["component_management"]["individual_components"]
    component = next(item for item in components if item["component_id"] == "python-sdk")
    assert component["paths"] == ["sdk/python/capsem/**"], RATIONALE


def test_sdk_ci_owns_cold_install_tests_and_complete_coverage() -> None:
    _assert_owners(*_documents())


@pytest.mark.parametrize("mutation", ["prewarm", "late_prewarm", "tests", "upload", "generated", "floor"])
def test_removing_ci_ownership_is_rejected(mutation: str) -> None:
    fast, ci, coverage = deepcopy(_documents())
    match mutation:
        case "prewarm":
            for step in fast["jobs"]["static"]["steps"]:
                step["run"] = step.get("run", "").replace("uv sync --project sdk/python --frozen", "")
        case "late_prewarm":
            fast["jobs"]["static"]["steps"].reverse()
        case "tests":
            ci["jobs"]["test"]["steps"] = [step for step in ci["jobs"]["test"]["steps"] if step.get("working-directory") != "sdk/python"]
        case "upload":
            ci["jobs"]["test"]["steps"] = [step for step in ci["jobs"]["test"]["steps"] if step.get("with", {}).get("flags") != "python-sdk"]
        case "generated":
            coverage["flags"]["python-sdk"]["paths"] = ["sdk/python/capsem/client.py"]
        case "floor":
            coverage["coverage"]["status"]["project"]["python-sdk"]["target"] = "0%"
    with pytest.raises((AssertionError, StopIteration)):
        _assert_owners(fast, ci, coverage)
