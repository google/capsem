"""The modules `just test` is made of, as graphs rather than regions of a file.

`_test-candidate-run` selected between six modules with a `CAPSEM_TEST_MODULE`
environment variable and a `module_enabled` shell function, so a module was the
text between two `if` statements. Running one meant exporting a variable and
hoping; asking what one would do was not possible at all.

These assert edges rather than positions. An edge is a stronger and shorter
claim: "clippy runs after the frontend build" holds however the source is
arranged, where "clippy appears at index 9" holds until someone inserts a step.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.candidate import CandidateCommand
from capsem_builder.gate.module_artifacts import ArtifactsModule
from capsem_builder.gate.staticmodule import StaticModule
from capsem_builder.gate.testmodules import FastModule
from helpers.gate import RecordingRunner

PROJECT_ROOT = Path(__file__).resolve().parents[3]


#: `resources()` takes the runner it should build with; these tests ask
#: *what* is held, so any runner will do.
def _resource_runner():
    from helpers.gate import RecordingRunner

    return RecordingRunner(PROJECT_ROOT)


RUNNER_FOR_RESOURCES = _resource_runner()
CONFIG = gate_config.load(PROJECT_ROOT)


def _module(cls):
    args = argparse.Namespace(dry_run=False, graph=False, timing=False)
    return cls(RecordingRunner(PROJECT_ROOT), args)


def _plan(cls):
    return _module(cls).plan()


def _waves(cls) -> list[set[str]]:
    return [{s.label for s in wave} for wave in _plan(cls).order()]


def _wave_of(cls, label: str) -> int:
    for position, wave in enumerate(_waves(cls)):
        if label in wave:
            return position
    raise AssertionError(f"{label} is not in the {cls.name} plan")


# ---------------------------------------------------------------------------
# The fast module
# ---------------------------------------------------------------------------


def test_clippy_waits_for_the_frontend_build() -> None:
    """`capsem-app` embeds `web/app/dist` at compile time, so clippy reads a
    directory the frontend build produces.

    The shell expressed this as a conditional that skipped clippy entirely
    when the frontend failed -- which lost the clippy result on exactly the
    runs where the most had changed.

    The build, and only the build. While a type-check, vitest and the build
    were one step, clippy waited on all three -- and, through the generated
    mock that only the tests import, on an `mcp_export` build as well.
    """
    assert _wave_of(FastModule, "fast.clippy") > _wave_of(FastModule, "fast.web.frontend-build")
    assert _wave_of(FastModule, "fast.web.frontend-verify") >= _wave_of(FastModule, "fast.clippy"), (
        "clippy waiting on the verify half is the cost the split removed"
    )


def test_rust_format_is_a_fast_source_leaf() -> None:
    """Formatting needs Rust, but no frontend bundle, ORT, or compilation."""
    plan = _plan(FastModule)
    label = "fast.rust-format"

    assert _wave_of(FastModule, label) > _wave_of(FastModule, "fast.audit.source-syntax")
    assert _wave_of(FastModule, label) > _wave_of(FastModule, "fast.toolchain.rust")
    assert _wave_of(FastModule, label) < _wave_of(FastModule, "fast.clippy")
    assert CONFIG.modules.rust_format == ("cargo", "fmt", "--all", "--", "--check")
    assert "cargo fmt --all -- --check" in "\n".join(plan.step_named(label).render())


def test_static_owns_the_frontend_bundle_before_rust_coverage() -> None:
    """A private static checkout cannot inherit ``web/app/dist`` from fast.

    Tauri reads that directory while compiling ``capsem-app`` tests, so the
    static module has to own the ignored Node tree, generated settings, and
    bundle rather than relying on a different gate command's prefix.
    """
    plan = _plan(StaticModule)
    node = "static.toolchain.node"
    settings = "static.audit.generated-settings"
    bundle = "static.web.frontend-bundle"
    coverage = "static.rust-coverage"

    assert _wave_of(StaticModule, node) < _wave_of(StaticModule, settings)
    assert _wave_of(StaticModule, settings) < _wave_of(StaticModule, bundle)
    assert _wave_of(StaticModule, bundle) < _wave_of(StaticModule, coverage)
    assert plan.after_of(coverage) >= {bundle}

    rendered = "\n".join(plan.step_named(bundle).render())
    assert CONFIG.frontend.build_script in rendered
    assert CONFIG.frontend.build_target in rendered


def test_artifacts_owns_the_frontend_bundle_before_build_chain() -> None:
    """The focused artifact owner must build Tauri from a bare checkout."""
    plan = _plan(ArtifactsModule)
    node = "artifacts.toolchain.node"
    bundle = "artifacts.web.frontend-bundle"
    consumer = "artifacts.build-chain"

    assert _wave_of(ArtifactsModule, node) < _wave_of(ArtifactsModule, bundle)
    assert _wave_of(ArtifactsModule, bundle) < _wave_of(ArtifactsModule, consumer)
    assert plan.after_of(consumer) >= {bundle}
    workspace = CONFIG.path(CONFIG.frontend.workspace).name
    assert f"(in {workspace})" in "\n".join(plan.step_named(node).render())


def test_candidate_hands_one_frontend_bundle_to_every_consumer() -> None:
    plan = CandidateCommand(
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
    )._describe()
    rendered = "bash build_system/scripts/web/check-web-surface.sh frontend-build"

    assert [
        step.label
        for step in plan.steps
        if rendered in step.render()
    ] == ["fast.web.frontend-build"]
    assert plan.after_of("artifacts.build-chain") >= {"fast.web.frontend-build"}


@pytest.mark.parametrize(
    ("module", "materializer", "consumer"),
    [
        (FastModule, "fast.toolchain.ort", "fast.clippy"),
        (StaticModule, "static.toolchain.ort", "static.rust-coverage"),
    ],
)
def test_rust_builds_wait_for_the_verified_host_ort_distribution(
    module, materializer: str, consumer: str
) -> None:
    plan = _plan(module)
    materialize = next(step for step in plan.steps if step.label == materializer)
    build = next(step for step in plan.steps if step.label == consumer)

    assert _wave_of(module, consumer) > _wave_of(module, materializer)
    assert any("[outside kernel sandbox]" in line for line in materialize.render())
    rendered = "\n".join(build.render())
    assert "ORT_STRATEGY=system" in rendered
    assert "ORT_LIB_LOCATION=" in rendered


def test_static_materializes_only_dependency_helpers_outside_the_sandbox() -> None:
    plan = _plan(StaticModule)
    networked = {"host-image", "install.materialize", "static.guest-builder"}

    for label in networked:
        rendered = plan.step_named(label).render()
        assert any("[outside kernel sandbox]" in line for line in rendered), label

    assert plan.after_of("static.guest-agents") == {"static.guest-builder"}
    for label in ("install.image-build", "install.image-smoke", "static.guest-agents"):
        assert all(
            "[outside kernel sandbox]" not in line for line in plan.step_named(label).render()
        ), label


def test_the_dependency_is_taken_from_config_not_from_position() -> None:
    """Reordering the surface list must not move the edge onto another one.

    Two edges now, onto two different surfaces, which is the point: clippy
    waits for the bundle and the generated mock is waited for by the tests
    that import it. Held apart in config so neither can drift onto the other.
    """
    assert CONFIG.websurfaces.blocks_clippy == "frontend-build"
    assert CONFIG.websurfaces.needs_generated_settings == "frontend-verify"
    assert CONFIG.websurfaces.blocks_clippy != CONFIG.websurfaces.needs_generated_settings, (
        "one surface carrying both edges is the arrangement that put an "
        "mcp_export build in front of clippy"
    )


def test_nothing_runs_before_the_source_parses() -> None:
    """Every check below spends real time; a syntax error makes all of it
    noise about a file nobody can import."""
    syntax = _wave_of(FastModule, "fast.audit.source-syntax")

    for label in (
        "fast.audit.cargo",
        "fast.audit.public-surface",
        "python.ruff",
        "python.ty.strict",
        "fast.clippy",
    ):
        assert _wave_of(FastModule, label) > syntax


def test_the_environment_is_installed_before_anything_uses_it() -> None:
    """Everything here runs through uv or pnpm. A gate that assumes the
    lockfile is already installed works only on the machine it was written on."""
    python = _wave_of(FastModule, "fast.toolchain.python")
    node = _wave_of(FastModule, "fast.toolchain.node")

    assert _wave_of(FastModule, "fast.audit.source-syntax") > python
    assert _wave_of(FastModule, "fast.web.frontend-build") > node
    assert _wave_of(FastModule, "fast.web.frontend-verify") > node


def test_the_audits_are_independent_of_each_other() -> None:
    """None reads what another writes, so they land in one wave and every
    failure comes back named rather than as a single FAIL bit."""
    waves = _waves(FastModule)
    audits = {
        "fast.audit.cargo",
        "fast.audit.pnpm",
        "fast.audit.python-lock",
        "fast.audit.public-surface",
        "fast.audit.skills",
        "fast.audit.release-selections",
    }

    together = next(wave for wave in waves if "fast.audit.cargo" in wave)
    assert audits <= together


def test_every_web_surface_is_its_own_step() -> None:
    """One step per surface, so a failure says which one rather than
    `check-web-surface.sh failed`."""
    labels = {label for wave in _waves(FastModule) for label in wave}

    for target in CONFIG.websurfaces.targets:
        assert f"fast.web.{target}" in labels


def test_strict_pytest_collection_is_a_fast_leaf() -> None:
    labels = {label for wave in _waves(FastModule) for label in wave}

    assert "fast.pytest.collection" in labels
    assert "fast.pytest.build-system-collection" not in labels


def test_the_fast_module_works_in_an_isolated_home() -> None:
    """Never the developer's `~/.capsem`; only audits get scoped egress."""
    resources = _module(FastModule).resources(RUNNER_FOR_RESOURCES)

    assert [resource.name for resource in resources] == ["release-egress", "workspace"]


def test_the_fast_module_needs_the_machine_to_itself() -> None:
    assert FastModule.exclusive is True


def test_the_plan_is_acyclic_and_therefore_runnable() -> None:
    """A cycle would be reported here rather than forty minutes in."""
    assert _plan(FastModule).order()


# ---------------------------------------------------------------------------
# The source-contract inventory
# ---------------------------------------------------------------------------


def test_every_gate_test_is_a_source_contract_test() -> None:
    """They need no built artifacts and no VM, by construction.

    The list was 47 hand-maintained lines in the justfile, and eleven gate test
    files had been added without reaching it -- so they ran in neither the fast
    module nor the exclusion that keeps them out of the VM matrix.
    """
    listed = set(CONFIG.suites.source_contract)
    gate_tests = {
        str(path.relative_to(PROJECT_ROOT))
        for path in (
            PROJECT_ROOT / CONFIG.suites.pytest.build_system_root / "gate"
        ).glob("test_gate_*.py")
    }

    missing = sorted(gate_tests - listed)
    assert not missing, (
        "these need neither artifacts nor a VM, so they belong in "
        f"config/gate.toml's [suites] source_contract: {missing}"
    )


def test_every_listed_contract_test_exists() -> None:
    """A list naming a deleted file quietly stops excluding anything."""
    missing = sorted(
        entry for entry in CONFIG.suites.source_contract if not (PROJECT_ROOT / entry).is_file()
    )

    assert not missing, f"these no longer exist: {missing}"


@pytest.mark.parametrize("entry", CONFIG.suites.source_contract)
def test_no_contract_test_is_listed_twice(entry: str) -> None:
    assert CONFIG.suites.source_contract.count(entry) == 1
