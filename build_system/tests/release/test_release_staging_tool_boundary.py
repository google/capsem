"""Guard direct package ownership for release staging and input tools."""

from __future__ import annotations

import ast
import stat
from pathlib import Path

BUILD_SYSTEM_ROOT = Path(__file__).resolve().parents[2]
REPOSITORY_ROOT = BUILD_SYSTEM_ROOT.parent
TOOL_ROOT = BUILD_SYSTEM_ROOT / "builder" / "release" / "tools"

FOUNDATIONS = {
    "release_channel_author",
    "release_cohort",
    "release_first_release",
    "release_fixture_server",
    "release_glowup",
    "release_inputs",
    "release_installed_probe",
    "release_test_profiles",
    "release_transition",
    "release_transition_candidates",
    "release_version_tag",
}
COMMANDS = {
    "fetch-channel-source-manifest.py": "fetch_channel_source_manifest",
    "fetch-release-artifacts.py": "fetch_release_artifacts",
    "finalize-binary-staging-fixtures.py": "finalize_binary_staging_fixtures",
    "generate-host-binary-sbom.py": "generate_host_binary_sbom",
    "list-release-manifest-assets.py": "list_release_manifest_assets",
    "materialize-graph-profile-artifacts.py": "materialize_graph_profile_artifacts",
    "project-first-channel-before.py": "project_first_channel_before",
    "prove-release-profile-assets.py": "prove_release_profile_assets",
    "stage-profile-publication.py": "stage_profile_publication",
    "stage-release-test-inputs.py": "stage_release_test_inputs",
    "verify-release-inputs.py": "verify_release_inputs",
}
SUPPORT = {"profile_root_payload"}


def test_release_staging_tools_extend_the_exact_owned_package() -> None:
    assert {
        "__init__",
        *FOUNDATIONS,
        *SUPPORT,
        *COMMANDS.values(),
    } <= {path.stem for path in TOOL_ROOT.glob("*.py")}


def test_release_staging_launchers_are_thin_direct_commands() -> None:
    for name, module in COMMANDS.items():
        path = REPOSITORY_ROOT / "build_system" / "scripts" / "release" / name
        source = path.read_text(encoding="utf-8")
        tree = ast.parse(source)
        assert len(source.splitlines()) <= 21, f"{name} contains reusable behavior"
        imports = [
            node
            for node in tree.body
            if isinstance(node, ast.ImportFrom)
            and node.module == f"capsem_builder.release.tools.{module}"
        ]
        assert len(imports) == 1, f"{name} does not import its direct owner"
        assert [alias.name for alias in imports[0].names] == ["main"]
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
        assert len(exits) == 1, f"{name} does not propagate command status"
        assert bool(path.stat().st_mode & stat.S_IXUSR) == (
            name == "finalize-binary-staging-fixtures.py"
        )


def test_release_staging_tools_use_package_relative_sibling_imports() -> None:
    sibling_names = FOUNDATIONS | SUPPORT | set(COMMANDS.values())
    for module in SUPPORT | set(COMMANDS.values()):
        path = TOOL_ROOT / f"{module}.py"
        tree = ast.parse(path.read_text(encoding="utf-8"))
        for node in ast.walk(tree):
            if not isinstance(node, ast.ImportFrom) or node.module is None:
                continue
            top = node.module.split(".", 1)[0]
            assert not (node.level == 0 and top in sibling_names), (
                f"{path.name} uses an ambient sibling import"
            )
            assert not node.module.startswith("scripts."), (
                f"{path.name} imports through a compatibility launcher"
            )
