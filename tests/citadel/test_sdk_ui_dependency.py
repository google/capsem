"""A linked SDK must be built before every frontend consumer, including cold CI."""

from __future__ import annotations

import argparse
import json
from copy import deepcopy
from types import SimpleNamespace

import pytest
import yaml
from capsem_builder.gate.devloop import DevCommand
from capsem_builder.gate.hostpackage import BuildUiCommand
from citadel.test_sdk_quality import CONFIG, ROOT
from helpers.gate import RecordingRunner, gate_plan

RATIONALE = (
    "The UI imports the compiled SDK. Declare its install and build before UI "
    "checks/bundles; a warm dist directory must not hide a broken clean build, "
    "and concurrent packing must not delete files a frontend consumer is reading."
)


def _ancestors(plan, label: str) -> set[str]:
    pending = list(plan.after_of(label))
    found: set[str] = set()
    while pending:
        dependency = pending.pop()
        if dependency not in found:
            found.add(dependency)
            pending.extend(plan.after_of(dependency))
    return found


def _assert_order(plan, node: str, build: str, consumers: tuple[str, ...]) -> None:
    assert node in _ancestors(plan, build), RATIONALE
    assert all(build in _ancestors(plan, consumer) for consumer in consumers), RATIONALE
    rendered = "\n".join(plan.step_named(node).render())
    assert "(in typescript) CI=true pnpm install --offline --frozen-lockfile" in rendered, RATIONALE


@pytest.mark.parametrize(("command", "prefix", "consumers"), [
    ("test-fast", "fast", ("fast.web.frontend-build", "fast.web.frontend-verify")),
    ("test-static", "static", ("static.web.frontend-bundle",)),
    ("test-artifacts", "artifacts", ("artifacts.web.frontend-bundle",)),
])
def test_sdk_build_precedes_all_frontend_consumers(command: str, prefix: str, consumers: tuple[str, ...]) -> None:
    plan = gate_plan(command)
    build = f"{prefix}.sdk.typescript.{'build' if prefix == 'fast' else 'bundle'}"
    _assert_order(plan, f"{prefix}.toolchain.node", build, consumers)


@pytest.mark.parametrize("surface", [surface for surface in CONFIG.devloop.surfaces if surface != "tui"])
def test_development_frontends_build_the_sdk(surface: str) -> None:
    plan = DevCommand(RecordingRunner(ROOT), argparse.Namespace(surface=surface, args=[]))._describe()
    _assert_order(plan, "toolchain.node", "sdk.typescript.bundle", (surface,))


def test_desktop_build_compiles_sdk_before_embedding_frontend() -> None:
    plan = BuildUiCommand(RecordingRunner(ROOT), argparse.Namespace(profile="debug"))._describe()
    bundle = next(step.label for step in plan.steps
                  if any(CONFIG.frontend.build_script in line for line in step.render()))
    _assert_order(plan, "toolchain.node", "sdk.typescript.bundle", (bundle, "app.debug"))
    assert bundle in plan.after_of("app.debug"), RATIONALE


@pytest.mark.parametrize("removed", ["install", "build", "verify"])
def test_missing_sdk_predecessors_are_rejected(removed: str) -> None:
    plan = gate_plan("test-fast")
    build = "fast.sdk.typescript.build"
    victim = build if removed == "install" else f"fast.web.frontend-{removed}"
    broken = SimpleNamespace(step_named=plan.step_named,
                             after_of=lambda label: set() if label == victim else plan.after_of(label))
    with pytest.raises(AssertionError, match="compiled SDK"):
        _assert_order(broken, "fast.toolchain.node", build,
                      ("fast.web.frontend-build", "fast.web.frontend-verify"))


def _assert_ci(workflow: dict) -> None:
    for job in workflow["jobs"].values():
        steps = job.get("steps", [])
        for index, consumer in enumerate(steps):
            if "check-web-surface.sh frontend-" not in consumer.get("run", ""):
                continue
            commands = [line for step in steps[:index]
                        if step.get("working-directory") == CONFIG.sdk_typescript.project
                        for line in step.get("run", "").splitlines()]
            assert "pnpm install --frozen-lockfile" in commands, RATIONALE
            assert "pnpm run build" in commands, RATIONALE
            assert commands.index("pnpm install --frozen-lockfile") < commands.index("pnpm run build"), RATIONALE


@pytest.mark.parametrize("filename", ["ci.yaml", "release.yaml"])
def test_direct_frontend_ci_builds_install_and_compile_sdk(filename: str) -> None:
    workflow = yaml.safe_load((ROOT / ".github/workflows" / filename).read_text())
    _assert_ci(workflow)
    broken = deepcopy(workflow)
    for job in broken["jobs"].values():
        job["steps"] = [step for step in job.get("steps", [])
                        if step.get("name") != "Build TypeScript SDK for frontend consumers"]
    with pytest.raises(AssertionError, match="compiled SDK"):
        _assert_ci(broken)


def test_ui_dependency_uses_public_compiled_package_exports() -> None:
    ui = json.loads((ROOT / "web/app/package.json").read_text())
    assert ui["dependencies"]["@capsem/sdk"] == "link:../../sdk/typescript", RATIONALE
    sdk = json.loads((ROOT / CONFIG.sdk_typescript.manifest).read_text())
    for subpath in (".", "./models", "./operations", "./transport"):
        entry = sdk["exports"][subpath]
        assert entry["types"].startswith("./dist/") and entry["types"].endswith(".d.ts"), RATIONALE
        assert entry["import"].startswith("./dist/") and entry["import"].endswith(".js"), RATIONALE
