"""Guard direct package ownership for release orchestration and report tools."""

from __future__ import annotations

import ast
import stat
from pathlib import Path

BUILD_SYSTEM_ROOT = Path(__file__).resolve().parents[2]
REPOSITORY_ROOT = BUILD_SYSTEM_ROOT.parent
TOOL_ROOT = BUILD_SYSTEM_ROOT / "builder" / "release" / "tools"

EXISTING_TOOLS = {
    "check_channel_deploy_freshness",
    "check_profile_release_delta",
    "check_public_binary_release",
    "check_release_graph_diff",
    "check_remote_release_readiness",
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
    "release_package_contract",
    "release_test_profiles",
    "release_transition",
    "release_transition_candidates",
    "release_version_tag",
    "stage_profile_publication",
    "stage_release_test_inputs",
    "verify_channel_downloads",
    "verify_immutable_publication",
    "verify_installed_release",
    "verify_profile_publication",
    "verify_release_inputs",
    "verify_release_recovery_run",
}
COMMANDS = {
    "build-complete-release-channel.py": "build_complete_release_channel",
    "extract-release-notes.py": "extract_release_notes",
    "local-release-glowup.py": "local_release_glowup",
    "marketing_install_surface.py": "marketing_install_surface",
    "nightly_release_scheduler.py": "nightly_release_scheduler",
    "publish-release-source.py": "publish_release_source",
    "release-binaries.py": "release_binaries",
    "release_collect_evidence.py": "release_collect_evidence",
    "replay-release-lane.py": "replay_release_lane",
    "write-binary-channel-staging-proof.py": "write_binary_channel_staging_proof",
    "write-release-notes.py": "write_release_notes",
    "write-release-summary.py": "write_release_summary",
}
SUPPORT = {"release_pairing_baseline", "remote_ci_gate"}
EXECUTABLES = {
    "local-release-glowup.py",
    "write-binary-channel-staging-proof.py",
    "write-release-notes.py",
}


def test_release_orchestration_tools_close_the_exact_owned_package() -> None:
    assert {path.stem for path in TOOL_ROOT.glob("*.py")} == {
        "__init__",
        *EXISTING_TOOLS,
        *SUPPORT,
        *COMMANDS.values(),
    }


def test_release_orchestration_launchers_are_thin_direct_commands() -> None:
    for name, module in COMMANDS.items():
        script_root = (
            REPOSITORY_ROOT / "build_system" / "scripts" / "web"
            if name == "marketing_install_surface.py"
            else REPOSITORY_ROOT / "build_system" / "scripts" / "release"
        )
        path = script_root / name
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


def test_release_orchestration_tools_use_package_relative_sibling_imports() -> None:
    sibling_names = EXISTING_TOOLS | SUPPORT | set(COMMANDS.values())
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
