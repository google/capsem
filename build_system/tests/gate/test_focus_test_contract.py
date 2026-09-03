"""The focused developer surface aliases existing gate owners exactly."""

from __future__ import annotations

import argparse
import re
from pathlib import Path

import pytest
import variables
from capsem_builder.gate import cli, focus, module_contracts
from capsem_builder.gate.qualification import LocalQualification
from helpers.gate import RecordingRunner

ROOT = Path(__file__).resolve().parents[3]


def _args(group: str, mode: str = "reuse") -> argparse.Namespace:
    return argparse.Namespace(
        group=group,
        mode=mode,
        dry_run=False,
        graph=False,
        timing=False,
        clean_build=False,
        resume_from="auto",
        stop_before=None,
        sandbox=None,
    )


@pytest.mark.parametrize(("group", "target"), sorted(focus.TARGETS.items()))
def test_each_focus_group_is_the_existing_owning_plan(group: str, target) -> None:
    runner = RecordingRunner(ROOT)
    qualification = LocalQualification(bin_dir="cache/target/cargo/debug")
    alias = focus.FocusTestCommand(runner, _args(group), qualification=qualification)
    owner_args = vars(_args(group)) | {"quick": False, "dimensions": "", "commit": "unknown"}
    owner = target(
        runner,
        argparse.Namespace(**owner_args),
        qualification=qualification,
    )

    assert alias.plan().describe() == owner.plan().describe()


def test_release_system_focus_is_source_only_and_needs_no_local_package() -> None:
    assert focus.TARGETS["release-system"] is module_contracts.ReleaseContractsModule
    plan = focus.FocusTestCommand(
        RecordingRunner(ROOT),
        _args("release-system"),
        qualification=LocalQualification(bin_dir="cache/target/cargo/debug"),
    ).plan().describe()

    assert "contracts.release" in plan
    assert "contracts.build-system" in plan
    assert (
        "uv run --project build_system --frozen python -m pytest -c build_system/pyproject.toml --rootdir . build_system/tests/"
        in plan
    )
    assert "rehearsal.cohort" not in plan


def test_rust_focus_uses_the_configured_affected_selector() -> None:
    assert focus.TARGETS["rust"] is focus.RustAffectedCommand
    command = focus.FocusTestCommand(
        RecordingRunner(ROOT),
        _args("rust"),
        qualification=LocalQualification(bin_dir="cache/target/cargo/debug"),
    )

    rendered = command.plan().describe()
    assert command.private_checkout is False
    assert command.exclusive is True
    assert "rust.affected" in rendered
    assert command._config.devloop.rust_affected in rendered
    assert "capsem-gate" not in rendered


def test_install_focus_materializes_the_standalone_content_pair(monkeypatch) -> None:
    """A focused glow-up must not inherit a prior Ironbank workspace."""
    from capsem_builder.gate.content import ProfileContent

    selected: list[Path] = []
    original = ProfileContent.standalone.__func__

    def observed(cls, config):
        content = original(cls, config)
        selected.append(content.root)
        return content

    monkeypatch.setattr(ProfileContent, "standalone", classmethod(observed))
    plan = focus.FocusTestCommand(
        RecordingRunner(ROOT),
        _args("install"),
        qualification=LocalQualification(bin_dir="cache/target/cargo/debug"),
    ).plan()

    assert selected == [ROOT]
    assert "materialize-config" in plan.labels
    assert plan.labels.index("materialize-config") < plan.labels.index("package.arm64.content")


def test_focus_adopts_the_owner_lifecycle_without_nesting_a_gate_action() -> None:
    command = focus.FocusTestCommand(
        RecordingRunner(ROOT),
        _args("assets", "clean"),
        qualification=LocalQualification(bin_dir="cache/target/cargo/debug"),
    )

    owner = command._target()
    assert command.exclusive == owner.exclusive
    assert command.private_checkout == owner.private_checkout
    assert command._sandbox_mode == owner._sandbox_mode
    assert "reexec" not in vars(type(command))
    assert re.search(
        r"(?<![\w-])capsem-gate(?![\w-])", command.plan().describe()
    ) is None


@pytest.mark.parametrize(
    "argv",
    [[variables.FOCUS_TEST, "unknown"], [variables.FOCUS_TEST, "assets", "maybe"]],
)
def test_unknown_focus_names_and_modes_fail_during_parsing(argv: list[str]) -> None:
    with pytest.raises(SystemExit):
        cli.build_parser().parse_args(argv)


def test_the_public_recipe_passes_only_the_group_and_reuse_mode() -> None:
    recipe = (ROOT / "justfile").read_text(encoding="utf-8")
    assert f'{variables.FOCUS_TEST} group mode="reuse":' in recipe
    assert f"capsem-gate {variables.FOCUS_TEST}" in recipe


def test_the_just_skill_lists_every_focus_owner() -> None:
    guide = (ROOT / "skills/dev-just/SKILL.md").read_text(encoding="utf-8")
    row = next(line for line in guide.splitlines() if "just focus-test <group>" in line)
    for group in focus.TARGETS:
        assert f"`{group}`" in row, f"focus owner {group!r} is missing from /dev-just"
