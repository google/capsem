"""The TypeScript SDK needs cold-run dependency and published coverage owners."""

from __future__ import annotations

from copy import deepcopy

import pytest
from citadel.test_sdk_ci import RATIONALE, _documents


def _assert_owners(fast: dict, ci: dict, coverage: dict) -> None:
    steps = fast["jobs"]["static"]["steps"]
    prewarm = next(index for index, step in enumerate(steps)
                   if "build_system/release_site sdk/typescript; do" in step.get("run", ""))
    sealed = next(index for index, step in enumerate(steps) if step.get("run") == "just fast-test")
    assert prewarm < sealed, RATIONALE
    steps = ci["jobs"]["test"]["steps"]
    test = next(step for step in steps if step.get("working-directory") == "sdk/typescript")
    assert test["run"].splitlines() == ["pnpm install --frozen-lockfile", (
        "pnpm test --reporter=default --reporter=junit "
        "--outputFile=../../cache/target/coverage/junit/typescript-sdk.xml"
    )], RATIONALE
    upload = next(step for step in steps if step.get("with", {}).get("flags") == "typescript-sdk")
    assert upload["with"]["files"] == "cache/target/coverage/typescript-sdk/lcov.info", RATIONALE
    assert upload["uses"].startswith("codecov/codecov-action@"), RATIONALE
    assert coverage["flags"]["typescript-sdk"]["paths"] == ["sdk/typescript/src/**"], RATIONALE
    assert coverage["coverage"]["status"]["project"]["typescript-sdk"]["target"] == "90%", RATIONALE
    component = next(item for item in coverage["component_management"]["individual_components"]
                     if item["component_id"] == "typescript-sdk")
    assert component["paths"] == ["sdk/typescript/src/**"], RATIONALE


def test_typescript_ci_owns_install_and_coverage() -> None:
    _assert_owners(*_documents())


@pytest.mark.parametrize("mutation", ["prewarm", "late", "tests", "upload", "generated", "floor"])
def test_removing_typescript_ci_ownership_is_rejected(mutation: str) -> None:
    fast, ci, coverage = deepcopy(_documents())
    match mutation:
        case "prewarm":
            for step in fast["jobs"]["static"]["steps"]:
                step["run"] = step.get("run", "").replace(" sdk/typescript; do", "; do")
        case "late":
            fast["jobs"]["static"]["steps"].reverse()
        case "tests":
            ci["jobs"]["test"]["steps"] = [step for step in ci["jobs"]["test"]["steps"]
                                              if step.get("working-directory") != "sdk/typescript"]
        case "upload":
            ci["jobs"]["test"]["steps"] = [step for step in ci["jobs"]["test"]["steps"]
                                              if step.get("with", {}).get("flags") != "typescript-sdk"]
        case "generated":
            coverage["flags"]["typescript-sdk"]["paths"] = ["sdk/typescript/src/client.ts"]
        case "floor":
            coverage["coverage"]["status"]["project"]["typescript-sdk"]["target"] = "0%"
    with pytest.raises((AssertionError, StopIteration)):
        _assert_owners(fast, ci, coverage)
