"""SDK packages must bring their own executable language and coverage gates."""

from __future__ import annotations

import re
import tomllib
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import sdkchecks
from capsem_builder.gate.plan import Plan
from helpers.gate import gate_plan

ROOT = Path(__file__).resolve().parents[2]
CONFIG = gate_config.load(ROOT)
SDK_RATIONALE = (
    "SDK code, including generated code, needs actual lint, strict type, "
    "test/coverage and package-build checks in the fast gate. Size checks "
    "alone do not prove that a usable SDK was tested."
)


def _coverage_problems(project: dict) -> list[str]:
    coverage = project["tool"]["coverage"]
    pytest_config = project["tool"]["pytest"]["ini_options"]
    problems = []
    if coverage["run"].get("source") != ["capsem"] or coverage["run"].get("branch") is not True:
        problems.append("SDK source and branches must be measured")
    if coverage["report"].get("fail_under", 0) < 90 or coverage["report"].get("precision", 0) < 2:
        problems.append("SDK coverage floor cannot be lowered")
    if any(coverage[section].get(key) for section, key in (
        ("run", "omit"), ("report", "exclude_lines"), ("report", "exclude_also"),
        ("report", "partial_branches"),
    )):
        problems.append("SDK generation cannot be excluded from coverage")
    options = pytest_config.get("addopts", "").split()
    if options != ["--cov=capsem", "--cov-report=term-missing:skip-covered"]:
        problems.append("tests must measure SDK coverage with the configured floor")
    if pytest_config.get("testpaths") != ["tests"]:
        problems.append("the full SDK test directory must run")
    return problems


def test_python_sdk_coverage_is_enforced_without_exclusions() -> None:
    project = tomllib.loads(CONFIG.path(CONFIG.sdk_python.manifest).read_text())
    assert not _coverage_problems(project), SDK_RATIONALE
    paths = [path for source in (CONFIG.sdk_python.source, CONFIG.sdk_python.tests)
             for path in (ROOT / source).rglob("*.py")]
    for path in paths:
        assert not re.search(r"#\s*(noqa|type:\s*ignore|ty:\s*ignore)\b", path.read_text()), SDK_RATIONALE


@pytest.mark.parametrize("mutation", ["branch", "floor", "omit", "unmeasured", "override", "partial", "no_cov"])
def test_coverage_weakening_is_rejected(mutation: str) -> None:
    project = tomllib.loads(CONFIG.path(CONFIG.sdk_python.manifest).read_text())
    coverage = project["tool"]["coverage"]
    pytest_config = project["tool"]["pytest"]["ini_options"]
    match mutation:
        case "branch":
            coverage["run"]["branch"] = False
        case "floor":
            coverage["report"]["fail_under"] = 0
        case "omit":
            coverage["run"]["omit"] = ["*/models/*"]
        case "unmeasured":
            pytest_config["addopts"] = ""
        case "override":
            pytest_config["addopts"] += " --cov-fail-under=0"
        case "partial":
            pytest_config["testpaths"] = ["tests/one_test.py"]
        case "no_cov":
            pytest_config["addopts"] += " --no-cov"
    assert _coverage_problems(project), SDK_RATIONALE


def test_sdk_checks_are_in_the_real_fast_plan() -> None:
    plan = Plan("SDK checks")
    leaves = sdkchecks.fragment(plan, CONFIG, after=())
    actual = gate_plan("test-fast")
    for check in leaves:
        assert check.label in actual.labels, SDK_RATIONALE
        assert actual.step_named(check.label).render() == check.render(), SDK_RATIONALE
    rendered = "\n".join(line for check in leaves for line in check.render())
    for command in ("ruff check", "ty check", "--error-on-warning", "pytest", "uv build"):
        assert command in rendered, SDK_RATIONALE
    assert "--ignore" not in rendered and "--exit-zero" not in rendered, SDK_RATIONALE
    assert "sdk/python/uv.lock" in CONFIG.audits.dependency_policy.lockfiles, SDK_RATIONALE


@pytest.mark.parametrize("missing", ["lint", "types", "tests", "build"])
def test_omitting_an_sdk_check_is_rejected(missing: str, monkeypatch: pytest.MonkeyPatch) -> None:
    actual = gate_plan("test-fast")
    incomplete = SimpleNamespace(
        labels=[label for label in actual.labels if label != f"fast.sdk.python.{missing}"],
        step_named=actual.step_named,
    )
    monkeypatch.setitem(globals(), "gate_plan", lambda *_args: incomplete)
    with pytest.raises(AssertionError, match="SDK code"):
        test_sdk_checks_are_in_the_real_fast_plan()
