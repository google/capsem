"""Guard direct package ownership for release verification and recovery tools."""

from __future__ import annotations

import ast
import stat
from pathlib import Path

BUILD_SYSTEM_ROOT = Path(__file__).resolve().parents[2]
REPOSITORY_ROOT = BUILD_SYSTEM_ROOT.parent
TOOL_ROOT = BUILD_SYSTEM_ROOT / "builder" / "release" / "tools"

EXISTING_TOOLS = {
    "fetch_channel_source_manifest",
    "fetch_release_artifacts",
    "finalize_binary_staging_fixtures",
    "generate_host_binary_sbom",
    "list_release_manifest_assets",
    "materialize_graph_profile_artifacts",
    "package_payload",
    "profile_root_payload",
    "project_first_channel_before",
    "prove_release_profile_assets",
    "release_channel_author",
    "release_cohort",
    "release_first_release",
    "release_fixture_server",
    "release_glowup",
    "release_inputs",
    "release_installed_probe",
    "release_manifest_rows",
    "release_test_profiles",
    "release_transition",
    "release_transition_candidates",
    "release_version_tag",
    "stage_profile_publication",
    "stage_release_test_inputs",
    "verify_release_inputs",
}
COMMANDS = {
    "check-channel-deploy-freshness.py": "check_channel_deploy_freshness",
    "check-profile-release-delta.py": "check_profile_release_delta",
    "check-public-binary-release.py": "check_public_binary_release",
    "check-release-graph-diff.py": "check_release_graph_diff",
    "check-remote-release-readiness.py": "check_remote_release_readiness",
    "release-package-contract.py": "release_package_contract",
    "verify-channel-downloads.py": "verify_channel_downloads",
    "verify-immutable-publication.py": "verify_immutable_publication",
    "verify-installed-release.py": "verify_installed_release",
    "verify-profile-publication.py": "verify_profile_publication",
    "verify-release-recovery-run.py": "verify_release_recovery_run",
}
EXECUTABLES = {
    "check-public-binary-release.py",
    "check-remote-release-readiness.py",
    "release-package-contract.py",
}


def test_release_verification_tools_extend_the_exact_owned_package() -> None:
    assert {path.stem for path in TOOL_ROOT.glob("*.py")} == {
        "__init__",
        *EXISTING_TOOLS,
        *COMMANDS.values(),
    }


def test_release_verification_launchers_are_thin_direct_commands() -> None:
    for name, module in COMMANDS.items():
        path = REPOSITORY_ROOT / "scripts" / name
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
        assert bool(path.stat().st_mode & stat.S_IXUSR) == (name in EXECUTABLES)


def test_release_verification_tools_use_package_relative_sibling_imports() -> None:
    sibling_names = EXISTING_TOOLS | set(COMMANDS.values())
    for module in COMMANDS.values():
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
