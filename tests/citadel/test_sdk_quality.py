"""SDK packages must bring their own executable language and coverage gates."""

from __future__ import annotations

import json
import re
import tomllib
from copy import deepcopy
from os.path import relpath
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate import sdkchecks
from capsem_builder.gate.execution import Needs
from capsem_builder.gate.plan import Plan
from helpers.gate import gate_plan
from packaging.requirements import Requirement

ROOT = Path(__file__).resolve().parents[2]
CONFIG = gate_config.load(ROOT)
SDK_RATIONALE = (
    "SDK code, including generated code, needs actual lint, strict type, "
    "test/coverage and package-build checks in the fast gate. Size checks "
    "alone do not prove that a usable SDK was tested."
)
INSPECT_LOCK_RATIONALE = (
    "Inspect's frozen editable SDK must match the actual SDK source and satisfy "
    "the extension requirement. A stale0.6.3 lock row previously hid behind "
    "passing source tests after labels and typed errors required a newer SDK."
)


def test_registry_packages_identify_their_canonical_source() -> None:
    for project in (CONFIG.sdk_typescript.project, CONFIG.mcp_typescript.project):
        package = json.loads((ROOT / project / "package.json").read_text())
        assert package.get("repository") == {
            "type": "git", "url": "git+https://github.com/google/capsem.git", "directory": project,
        }, SDK_RATIONALE
        assert package.get("license") == "Apache-2.0", SDK_RATIONALE
    python = tomllib.loads(CONFIG.path(CONFIG.sdk_python.manifest).read_text())["project"]
    assert python.get("readme") == "README.md", SDK_RATIONALE
    assert python.get("license") == "Apache-2.0", SDK_RATIONALE
    assert python.get("urls", {}).get("Repository") == "https://github.com/google/capsem", SDK_RATIONALE


def _inspect_lock_problems(extension: dict, sdk: dict, lock: dict) -> list[str]:
    packages = {entry["name"]: entry for entry in lock["package"]}
    problems = []
    if packages[extension["name"]]["version"] != extension["version"]:
        problems.append("extension version differs from its lock")
    locked_sdk = packages[sdk["name"]]
    if locked_sdk["version"] != sdk["version"]:
        problems.append("SDK version differs from its source")
    editable = Path(relpath(ROOT / CONFIG.sdk_python.project,
                            ROOT / CONFIG.integrations_inspect_ai.project)).as_posix()
    if locked_sdk["source"] != {"editable": editable}:
        problems.append("SDK lock must select its actual editable source")
    requirement = next(Requirement(value) for value in extension["dependencies"]
                       if Requirement(value).name == sdk["name"])
    if sdk["version"] not in requirement.specifier:
        problems.append("source SDK does not satisfy the extension requirement")
    return problems


def _inspect_version_inputs() -> tuple[dict, dict, dict]:
    project = Path(CONFIG.integrations_inspect_ai.project)
    extension = tomllib.loads(CONFIG.path(CONFIG.integrations_inspect_ai.manifest).read_text())["project"]
    sdk = tomllib.loads(CONFIG.path(CONFIG.sdk_python.manifest).read_text())["project"]
    lockfile = next(path for path in CONFIG.audits.dependency_policy.lockfiles
                    if Path(path).parent == project)
    lock = tomllib.loads(CONFIG.path(lockfile).read_text())
    return extension, sdk, lock


def test_inspect_frozen_sdk_matches_the_actual_package_contract() -> None:
    assert not _inspect_lock_problems(*_inspect_version_inputs()), INSPECT_LOCK_RATIONALE


def test_rust_registry_packages_are_verified_and_exported_by_the_fast_gate() -> None:
    plan = gate_plan("test-fast")
    packaged = plan.step_named("fast.sdk.rust.package")
    assert "capsem_builder.release.tools.package_rust_sdk" in " ".join(packaged.render()), SDK_RATIONALE
    output = Path(CONFIG.sdk_rust.build_output)
    assert any(Path(path) == output or Path(path) in output.parents
               for path in CONFIG.prefix.exports), SDK_RATIONALE


@pytest.mark.parametrize("owner", ["candidate", "test-fast"])
def test_qualification_validates_all_retained_client_archives_before_native_work(owner: str) -> None:
    plan = gate_plan(owner)
    cohort = plan.step_named("fast.sdk.packages.validate")
    assert "capsem_builder.release.tools.client_package_archives" in " ".join(cohort.render()), SDK_RATIONALE
    producers = {"fast.sdk.rust.package", "fast.sdk.python.build", "fast.mcp.typescript.tests"}
    assert producers <= plan.after_of(cohort.label), SDK_RATIONALE


def test_mcp_tests_consume_the_exported_publication_archives() -> None:
    plan = gate_plan("test-fast")
    assert plan.step_named("fast.mcp.typescript.types").render() == [
        f"(in {CONFIG.path(CONFIG.mcp_typescript.project)}) pnpm run check"
    ], SDK_RATIONALE
    output = getattr(CONFIG.mcp_typescript, "build_output", None)
    assert output and output in CONFIG.prefix.exports, SDK_RATIONALE
    selection = f"{CONFIG.environment.mcp_package_archives}={CONFIG.path(output)}"
    for label in ("fast.mcp.typescript.package-prewarm", "fast.mcp.typescript.tests"):
        assert selection in " ".join(plan.step_named(label).render()), SDK_RATIONALE


def test_adversarial_python_scratch_never_enters_a_publication_export() -> None:
    for settings in (CONFIG.sdk_python, CONFIG.integrations_inspect_ai):
        scratch = Path(settings.pytest_scratch)
        for export in CONFIG.prefix.exports:
            assert not scratch.is_relative_to(export), SDK_RATIONALE
            assert not Path(export).is_relative_to(scratch), SDK_RATIONALE


@pytest.mark.parametrize("mutation", ["sdk_version", "extension_version", "sdk_path", "minimum"])
def test_inspect_lock_guard_rejects_stale_or_incompatible_packages(mutation: str) -> None:
    extension, sdk, lock = deepcopy(_inspect_version_inputs())
    packages = {entry["name"]: entry for entry in lock["package"]}
    # Establish one coherent fixture before introducing the selected defect.
    packages[sdk["name"]]["version"] = sdk["version"]
    assert not _inspect_lock_problems(extension, sdk, lock)
    match mutation:
        case "sdk_version":
            packages[sdk["name"]]["version"] = "0.0.1"
        case "extension_version":
            packages[extension["name"]]["version"] = "0.0.1"
        case "sdk_path":
            packages[sdk["name"]]["source"] = {"registry": "https://pypi.org/simple"}
        case "minimum":
            extension["dependencies"] = ["capsem>=999.0.0"]
    expected = {
        "sdk_version": "SDK version differs from its source",
        "extension_version": "extension version differs from its lock",
        "sdk_path": "SDK lock must select its actual editable source",
        "minimum": "source SDK does not satisfy the extension requirement",
    }
    assert _inspect_lock_problems(extension, sdk, lock) == [expected[mutation]], INSPECT_LOCK_RATIONALE


def _coverage_problems(project: dict) -> list[str]:
    coverage = project["tool"]["coverage"]
    pytest_config = project["tool"]["pytest"]["ini_options"]
    problems = []
    if coverage["run"].get("source") != ["capsem"] or coverage["run"].get("branch") is not True:
        problems.append("SDK source and branches must be measured")
    if coverage["report"].get("fail_under", 0) < 90 or coverage["report"].get("precision", 0) < 2:
        problems.append("SDK coverage floor cannot be lowered")
    if coverage.get("xml", {}).get("output") != "../../cache/target/coverage/python-sdk/coverage.xml":
        problems.append("SDK coverage report must reach its CI upload owner")
    if any(coverage[section].get(key) for section, key in (
        ("run", "omit"), ("report", "exclude_lines"), ("report", "exclude_also"),
        ("report", "partial_branches"),
    )):
        problems.append("SDK generation cannot be excluded from coverage")
    options = pytest_config.get("addopts", "").split()
    if options != ["--cov=capsem", "--cov-report=term-missing:skip-covered", "--cov-report=xml"]:
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


@pytest.mark.parametrize("mutation", ["branch", "floor", "omit", "unmeasured", "override", "partial", "no_cov", "xml"])
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
        case "xml":
            coverage["xml"]["output"] = "unpublished.xml"
    assert _coverage_problems(project), SDK_RATIONALE


def test_sdk_checks_are_in_the_real_fast_plan() -> None:
    plan = Plan("SDK checks")
    leaves = sdkchecks.fragment(plan, CONFIG, after=())
    actual = gate_plan("test-fast")
    for check in leaves:
        assert check.label in actual.labels, SDK_RATIONALE
        assert actual.step_named(check.label).render() == check.render(), SDK_RATIONALE
    rendered = "\n".join(line for check in leaves for line in check.render())
    for command in ("ruff check", "ty check", "--error-on-warning", "pytest", "python -m build", "capsem_builder.sdkgen", "--check"):
        assert command in rendered, SDK_RATIONALE
    assert "--ignore" not in rendered and "--exit-zero" not in rendered, SDK_RATIONALE
    assert "sdk/python/uv.lock" in CONFIG.audits.dependency_policy.lockfiles, SDK_RATIONALE


def test_python_sdk_build_backend_is_warmed_before_sealed_sync() -> None:
    plan = gate_plan("test-fast")
    prewarm = plan.step_named("sdk.python.prewarm")
    synced = plan.step_named("sdk.python.sync")
    built = plan.step_named("fast.sdk.python.build")

    assert plan.after_of("sdk.python.sync") >= {prewarm.label}, SDK_RATIONALE
    assert prewarm.needs == frozenset({Needs.DISK, Needs.NETWORK}), SDK_RATIONALE
    assert prewarm.render() == [
        "uv sync --project sdk/python --frozen --no-install-project [outside kernel sandbox]"
    ], SDK_RATIONALE
    assert synced.render() == [
        "uv sync --project sdk/python --frozen --no-build-isolation"
    ], SDK_RATIONALE
    assert "--no-isolation" in " ".join(built.render()), SDK_RATIONALE


@pytest.mark.parametrize("missing", ["lint", "types", "tests", "build", "generate"])
def test_omitting_an_sdk_check_is_rejected(missing: str, monkeypatch: pytest.MonkeyPatch) -> None:
    actual = gate_plan("test-fast")
    incomplete = SimpleNamespace(
        labels=[label for label in actual.labels if label != f"fast.sdk.python.{missing}"],
        step_named=actual.step_named,
    )
    monkeypatch.setitem(globals(), "gate_plan", lambda *_args: incomplete)
    with pytest.raises(AssertionError, match="SDK code"):
        test_sdk_checks_are_in_the_real_fast_plan()


def test_inspect_extension_checks_are_in_the_real_fast_plan() -> None:
    settings = CONFIG.integrations_inspect_ai
    leaves = sdkchecks.inspect_fragment(Plan("Inspect checks"), CONFIG, after=())
    actual = gate_plan("test-fast")
    for check in leaves:
        assert check.label in actual.labels, SDK_RATIONALE
        assert actual.step_named(check.label).render() == check.render(), SDK_RATIONALE
    rendered = "\n".join(line for check in leaves for line in check.render())
    for command in ("ruff check", "ty check", "--error-on-warning", "pytest", "python -m build"):
        assert command in rendered, SDK_RATIONALE
    assert f"{settings.project}/uv.lock" in CONFIG.audits.dependency_policy.lockfiles, SDK_RATIONALE
    project = tomllib.loads(CONFIG.path(settings.manifest).read_text())
    coverage, options = project["tool"]["coverage"], project["tool"]["pytest"]["ini_options"]
    assert coverage["run"]["branch"] is True and coverage["report"]["fail_under"] >= 90, SDK_RATIONALE
    assert options["filterwarnings"] == ["error"], SDK_RATIONALE
    assert options["addopts"].split()[0] == "--cov=inspect_capsem", SDK_RATIONALE
    for path in (path for root in (settings.source, settings.tests) for path in (ROOT / root).rglob("*.py")):
        assert not re.search(r"#\s*(noqa|type:\s*ignore|ty:\s*ignore)\b", path.read_text()), SDK_RATIONALE
