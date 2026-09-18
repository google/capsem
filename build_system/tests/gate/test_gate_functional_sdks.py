"""The functional suites drive every SDK, so the functional module prepares them.

The Braavos suite builds and runs the Python, TypeScript and Rust SDKs against a
real gateway. The suites run inside the kernel sandbox with no network, and the
test itself was installing what it needed: `uv run` built the Python SDK in an
isolated environment that fetched hatchling from PyPI, `node tools/build.mjs`
found no `node_modules`, and `cargo run` compiled the SDK inside a 60-second
budget. All three failed in a focused functional run and passed only on a
checkout an earlier phase had happened to warm.
"""

from __future__ import annotations

from capsem_builder.gate import config as gate_config
from helpers.gate import PROJECT_ROOT, gate_plan

PREPARED = ("sdk.python.sync", "functional.sdk.rust.example", "functional.sdk.typescript.bundle")


def _ancestors(plan, label: str) -> set[str]:
    seen: set[str] = set()
    pending = [label]
    while pending:
        for before in plan.after_of(pending.pop()):
            if before not in seen:
                seen.add(before)
                pending.append(before)
    return seen


def test_every_functional_suite_starts_after_the_sdks_are_prepared() -> None:
    plan = gate_plan("test-functional")
    suites = [label for label in plan.labels if ".pytest." in label]
    assert suites, "the functional plan has no pytest suites to check"
    for suite in suites:
        missing = [step for step in PREPARED if step not in _ancestors(plan, suite)]
        assert not missing, f"{suite} can start before {missing}"


def test_the_typescript_sdk_is_among_the_installed_workspaces() -> None:
    config = gate_config.load(PROJECT_ROOT)
    assert config.sdk_typescript.project in config.functional.node_workspaces


def test_sdk_preparation_stays_offline_inside_the_sandbox() -> None:
    plan = gate_plan("test-functional")
    sync = plan.step_named("sdk.python.sync").actions[0].render()
    assert "--no-build-isolation" in sync, "an isolated build fetches its backend from the network"
    example = plan.step_named("functional.sdk.rust.example").actions[0].render()
    assert "--frozen" in example, "cargo must not reach the registry from inside the sandbox"
