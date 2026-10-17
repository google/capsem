"""TypeScript SDK source cannot lose its native tests, strictness or coverage."""

from __future__ import annotations

import json
import re
from types import SimpleNamespace

import pytest
from capsem_builder.gate import sdkchecks
from capsem_builder.gate.context import Context
from capsem_builder.gate.plan import Plan
from citadel.test_sdk_quality import CONFIG, ROOT, SDK_RATIONALE
from helpers.gate import RecordingRunner, gate_plan

BUILD_DEADLINE_RATIONALE = (
    "SDK compilation needs one config-owned process-group deadline. A hidden "
    "60-second child timer killed both hosted builds while concurrent type "
    "checks needed 123 seconds; the gate must own the finite build budget."
)


@pytest.mark.parametrize("seconds", [17, 300])
def test_typescript_build_deadline_reaches_the_process_runner(seconds: int) -> None:
    settings = CONFIG.sdk_typescript.model_copy(update={"build_timeout_seconds": seconds})
    config = CONFIG.model_copy(update={"sdk_typescript": settings})
    runner = RecordingRunner(ROOT)
    sdkchecks.typescript_bundle(config).run(Context(runner, config, observing=True))
    assert len(runner.commands) == 1, BUILD_DEADLINE_RATIONALE
    assert runner.commands[0].argv == ("pnpm", "run", "build"), BUILD_DEADLINE_RATIONALE
    assert runner.commands[0].timeout_seconds == seconds, BUILD_DEADLINE_RATIONALE


def _assert_quality(source: str, scripts: dict, compiler: dict) -> None:
    assert "include: ['src/**/*.ts'], exclude: []" in source, SDK_RATIONALE
    assert "include: ['tests/**/*.test.ts']" in source, SDK_RATIONALE
    assert "provider: 'v8'" in source and "'lcov'" in source, SDK_RATIONALE
    assert "reportsDirectory: '../../cache/target/coverage/typescript-sdk'" in source, SDK_RATIONALE
    thresholds = re.search(r"thresholds: \{([^}]+)\}", source)
    assert thresholds, SDK_RATIONALE
    for metric in ("lines", "branches", "functions", "statements"):
        value = re.search(rf"\b{metric}: (\d+)", thresholds[1])
        assert value and int(value[1]) >= 90, SDK_RATIONALE
    assert scripts["test"] == "vitest run --coverage", SDK_RATIONALE
    assert scripts["check"] == "tsc --noEmit -p tsconfig.test.json && tsc --noEmit -p tsconfig.browser.json", SDK_RATIONALE
    assert scripts["lint"] == "eslint src tests tools eslint.config.js vitest.config.ts --max-warnings 0", SDK_RATIONALE
    assert scripts["build"] == "node tools/build.mjs", SDK_RATIONALE
    assert scripts["prepack"] == "pnpm run build", SDK_RATIONALE
    for option in ("strict", "exactOptionalPropertyTypes", "noUncheckedIndexedAccess", "noUnusedLocals",
                   "noUnusedParameters", "noImplicitReturns", "noFallthroughCasesInSwitch", "noEmitOnError"):
        assert compiler.get(option) is True, SDK_RATIONALE
    assert not compiler.get("skipLibCheck") and not compiler.get("noCheck"), SDK_RATIONALE


def _settings() -> tuple[str, dict, dict]:
    root = ROOT / CONFIG.sdk_typescript.project
    return ((root / "vitest.config.ts").read_text(),
            json.loads((root / "package.json").read_text())["scripts"],
            json.loads((root / "tsconfig.json").read_text())["compilerOptions"])


def test_typescript_quality_is_enforced() -> None:
    _assert_quality(*_settings())
    root = ROOT / CONFIG.sdk_typescript.project
    browser = json.loads((root / "tsconfig.browser.json").read_text())
    _assert_browser_quality(browser)
    for root in (CONFIG.sdk_typescript.source, CONFIG.sdk_typescript.tests):
        for path in (ROOT / root).rglob("*.ts"):
            assert not re.search(r"(?:eslint-disable|@ts-ignore|@ts-nocheck|[vc]8 ignore|istanbul ignore)", path.read_text()), SDK_RATIONALE


def _assert_browser_quality(browser: dict) -> None:
    assert browser == {
        "extends": "./tsconfig.json",
        "compilerOptions": {
            "types": [], "noEmit": True, "customConditions": ["browser"],
            "paths": {"#local": ["./src/local-browser.ts"]},
        },
        "exclude": ["src/local-node.ts"],
    }, SDK_RATIONALE


@pytest.mark.parametrize("mutation", ["node", "built", "types", "exclude"])
def test_browser_discovery_must_stay_independent_of_node_and_build_output(mutation: str) -> None:
    root = ROOT / CONFIG.sdk_typescript.project
    browser = json.loads((root / "tsconfig.browser.json").read_text())
    match mutation:
        case "node":
            browser["compilerOptions"]["paths"]["#local"] = ["./src/local-node.ts"]
        case "built":
            browser["compilerOptions"]["paths"]["#local"] = ["./dist/local-browser.d.ts"]
        case "types":
            browser["compilerOptions"]["types"] = ["node"]
        case "exclude":
            browser["exclude"] = ["src/**/*.ts"]
    with pytest.raises(AssertionError, match="SDK code"):
        _assert_browser_quality(browser)


@pytest.mark.parametrize("mutation", ["source", "tests", "floor", "coverage", "strict", "lib", "browser"])
def test_typescript_quality_weakening_is_rejected(mutation: str) -> None:
    source, scripts, compiler = _settings()
    match mutation:
        case "source":
            source = source.replace("exclude: []", "exclude: ['src/models/**']")
        case "tests":
            source = source.replace("tests/**/*.test.ts", "tests/one.test.ts")
        case "floor":
            source = source.replace("branches: 90", "branches: 0")
        case "coverage":
            scripts["test"] = "vitest run"
        case "strict":
            compiler["strict"] = False
        case "lib":
            compiler["skipLibCheck"] = True
        case "browser":
            scripts["check"] = "tsc --noEmit -p tsconfig.test.json"
    with pytest.raises(AssertionError, match="SDK code"):
        _assert_quality(source, scripts, compiler)


def test_typescript_checks_reach_the_real_fast_gate() -> None:
    leaves = sdkchecks.typescript_fragment(Plan("SDK"), CONFIG, after=())
    actual = gate_plan("test-fast")
    for check in leaves:
        assert check.label in actual.labels, SDK_RATIONALE
        assert actual.step_named(check.label).render() == check.render(), SDK_RATIONALE
    assert "sdk/typescript" in CONFIG.toolchain.node_workspaces, SDK_RATIONALE
    assert "sdk/typescript/pnpm-lock.yaml" in CONFIG.audits.dependency_policy.lockfiles, SDK_RATIONALE


@pytest.mark.parametrize("missing", ["lint", "types", "tests", "build", "generate"])
def test_omitting_a_typescript_check_is_rejected(missing: str, monkeypatch: pytest.MonkeyPatch) -> None:
    actual = gate_plan("test-fast")
    incomplete = SimpleNamespace(
        labels=[label for label in actual.labels if label != f"fast.sdk.typescript.{missing}"],
        step_named=actual.step_named,
    )
    monkeypatch.setitem(globals(), "gate_plan", lambda *_args: incomplete)
    with pytest.raises(AssertionError, match="SDK code"):
        test_typescript_checks_reach_the_real_fast_gate()
