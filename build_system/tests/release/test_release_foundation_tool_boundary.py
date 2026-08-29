"""Guard direct package ownership for release foundation implementations."""

from __future__ import annotations

import ast
import tomllib
from pathlib import Path

BUILD_SYSTEM_ROOT = Path(__file__).resolve().parents[2]
REPOSITORY_ROOT = BUILD_SYSTEM_ROOT.parent
TOOL_ROOT = BUILD_SYSTEM_ROOT / "builder" / "release" / "tools"

COMMANDS = {
    "release-test-profiles.py": "release_test_profiles",
    "release_transition.py": "release_transition",
    "release_version_tag.py": "release_version_tag",
}
ADAPTERS = {
    "release_channel_author.py": "release_channel_author",
    "release_cohort.py": "release_cohort",
    "release_first_release.py": "release_first_release",
    "release_fixture_server.py": "release_fixture_server",
    "release_glowup.py": "release_glowup",
    "release_inputs.py": "release_inputs",
    "release_installed_probe.py": "release_installed_probe",
    "release_binary_cohort.py": "release_cohort",
    "release_transition_candidates.py": "release_transition_candidates",
}
LAUNCHERS = {**COMMANDS, **ADAPTERS}


def test_release_foundations_have_direct_owned_package_modules() -> None:
    assert {"__init__", *LAUNCHERS.values()} <= {
        path.stem for path in TOOL_ROOT.glob("*.py")
    }
    project = tomllib.loads(
        (BUILD_SYSTEM_ROOT / "pyproject.toml").read_text(encoding="utf-8")
    )
    assert "capsem_builder.release.tools" in project["tool"]["setuptools"][
        "packages"
    ]


def test_release_foundation_boundaries_are_thin_direct_adapters() -> None:
    for name, module in LAUNCHERS.items():
        source = (
            REPOSITORY_ROOT / "build_system" / "scripts" / "release" / name
        ).read_text(encoding="utf-8")
        tree = ast.parse(source)
        assert len(source.splitlines()) <= 21, f"{name} contains reusable behavior"
        imports = [
            node
            for node in tree.body
            if isinstance(node, ast.ImportFrom)
            and node.module == f"capsem_builder.release.tools.{module}"
        ]
        assert len(imports) == 1, f"{name} does not import its direct owner"
        assert not any(
            isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef))
            for node in tree.body
        )

        exits = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.Raise)
            and isinstance(node.exc, ast.Call)
            and isinstance(node.exc.func, ast.Name)
            and node.exc.func.id == "SystemExit"
        ]
        assert len(exits) == (1 if name in COMMANDS else 0)
        assert any(alias.name == "*" for alias in imports[0].names) == (
            name in ADAPTERS
        )


def test_release_foundations_use_package_relative_sibling_imports() -> None:
    sibling_names = set(LAUNCHERS.values())
    for path in TOOL_ROOT.glob("*.py"):
        tree = ast.parse(path.read_text(encoding="utf-8"))
        for node in ast.walk(tree):
            if not isinstance(node, ast.ImportFrom) or node.module is None:
                continue
            top = node.module.split(".", 1)[0]
            assert not (node.level == 0 and top in sibling_names), (
                f"{path.name} uses an ambient sibling import"
            )
            assert not node.module.startswith("scripts.release_"), (
                f"{path.name} imports through a compatibility adapter"
            )
