"""Citadel guard: a workflow may not restate the tool registry.

`config/gate.toml` owns which cargo-installed tools exist and at which version.
Seven workflow steps used to spell that out by hand, in seven different subsets,
and nothing related a job's list to what the job runs. The versions happened to
agree; the membership was guesswork.

`b3sum` was in the fast gate's list and not the binary pairing gate's, though
both run the broad suite and its asset-integrity tests shell out to `b3sum`.
Three of them failed on a missing tool ten minutes into a release job where
every binary had already built and installed. Nobody chose that subset -- it was
never compared with any other.

So a workflow names a declared set and `build_system/scripts/ci/gate-tool-list.py` derives the
rest. This keeps the restatement from coming back.
"""

from __future__ import annotations

import re
import subprocess
import tomllib
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = sorted((PROJECT_ROOT / ".github" / "workflows").glob("*.yaml"))

#: What a prebuilt installer is handed: `<name>@<version>`.
PINNED = re.compile(r"\b([a-z0-9][a-z0-9-]*)@(\d+\.\d+\.\d+)\b")

CONFIG = tomllib.loads((PROJECT_ROOT / "config" / "gate.toml").read_text(encoding="utf-8"))
CRATES = CONFIG["toolchain"]["crates"]
SETS = CONFIG["toolchain"]["sets"]

#: The names a prebuilt installer would be given, taken from `cargo install`
#: rather than the crate name -- `cargo-tauri` installs as `tauri-cli`.
INSTALLABLE = {crate["install"][2]: crate["install"][4] for crate in CRATES}


def test_the_registry_is_worth_guarding() -> None:
    """A guard over an empty registry asserts nothing."""
    assert INSTALLABLE, "config declares no cargo-installed tools"
    assert SETS, "config declares no tool sets, so no workflow can select one"


@pytest.mark.parametrize("workflow", WORKFLOWS, ids=lambda path: path.name)
def test_no_workflow_pins_a_tool_the_config_declares(workflow: Path) -> None:
    """The version lives in one file, and YAML is not it."""
    restated = sorted(
        f"{name}@{version}"
        for name, version in PINNED.findall(workflow.read_text(encoding="utf-8"))
        if name in INSTALLABLE
    )
    assert not restated, (
        f"{workflow.name} spells {restated}, which config/gate.toml already "
        "declares. Name a set from [toolchain.sets] and let "
        "build_system/scripts/ci/gate-tool-list.py derive it."
    )


@pytest.mark.parametrize("label", sorted(SETS), ids=str)
def test_every_declared_set_resolves_to_installable_pins(label: str) -> None:
    """The deriver must actually work for each set a workflow may name."""
    rendered = subprocess.run(
        [
            "python3",
            str(PROJECT_ROOT / "build_system/scripts/ci/gate-tool-list.py"),
            "--sets",
            label,
        ],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()

    assert rendered, f"set {label} rendered nothing"
    for pin in rendered.split(","):
        name, _, version = pin.partition("@")
        assert INSTALLABLE.get(name) == version, f"{pin} is not what the registry declares"


def test_every_workflow_that_installs_tools_names_a_declared_set() -> None:
    """A step handing the installer something else has gone around the registry."""
    strays = []
    for workflow in WORKFLOWS:
        text = workflow.read_text(encoding="utf-8")
        for line in text.splitlines():
            stripped = line.strip()
            if not stripped.startswith("tool:"):
                continue
            if "steps.gate_tools.outputs.list" not in stripped:
                strays.append(f"{workflow.name}: {stripped}")
    assert not strays, (
        "these hand a tool list to the installer instead of deriving it: " + "; ".join(strays)
    )
