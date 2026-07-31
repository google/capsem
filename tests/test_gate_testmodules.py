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
from helpers.gate import RecordingRunner

from capsem.gate import config as gate_config
from capsem.gate.testmodules import FastModule

PROJECT_ROOT = Path(__file__).resolve().parents[1]
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
    """`capsem-app` embeds `frontend/dist` at compile time, so clippy reads a
    directory the frontend build produces.

    The shell expressed this as a conditional that skipped clippy entirely
    when the frontend failed -- which lost the clippy result on exactly the
    runs where the most had changed.
    """
    assert _wave_of(FastModule, "clippy") > _wave_of(FastModule, "web.frontend")


def test_the_dependency_is_taken_from_config_not_from_position() -> None:
    """Reordering the surface list must not move the edge onto another one."""
    assert CONFIG.websurfaces.blocks_clippy == "frontend"


def test_nothing_runs_before_the_source_parses() -> None:
    """Every check below spends real time; a syntax error makes all of it
    noise about a file nobody can import."""
    syntax = _wave_of(FastModule, "audit.source-syntax")

    for label in ("audit.cargo", "audit.public-surface", "lint", "clippy"):
        assert _wave_of(FastModule, label) > syntax


def test_the_environment_is_installed_before_anything_uses_it() -> None:
    """Everything here runs through uv or pnpm. A gate that assumes the
    lockfile is already installed works only on the machine it was written on."""
    python = _wave_of(FastModule, "toolchain.python")
    node = _wave_of(FastModule, "toolchain.node")

    assert _wave_of(FastModule, "audit.source-syntax") > python
    assert _wave_of(FastModule, "web.frontend") > node


def test_the_audits_are_independent_of_each_other() -> None:
    """None reads what another writes, so they land in one wave and every
    failure comes back named rather than as a single FAIL bit."""
    waves = _waves(FastModule)
    audits = {
        "audit.cargo",
        "audit.pnpm",
        "audit.python-lock",
        "audit.public-surface",
        "audit.skills",
        "audit.release-selections",
    }

    together = next(wave for wave in waves if "audit.cargo" in wave)
    assert audits <= together


def test_every_web_surface_is_its_own_step() -> None:
    """One step per surface, so a failure says which one rather than
    `check-web-surface.sh failed`."""
    labels = {label for wave in _waves(FastModule) for label in wave}

    for target in CONFIG.websurfaces.targets:
        assert f"web.{target}" in labels


def test_the_fast_module_works_in_an_isolated_home() -> None:
    """Never the developer's `~/.capsem`."""
    resources = _module(FastModule).resources()

    assert [resource.name for resource in resources] == ["workspace"]


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
        for path in (PROJECT_ROOT / "tests").glob("test_gate_*.py")
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
