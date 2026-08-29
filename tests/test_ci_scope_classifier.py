"""The CI shortcut may skip expensive product jobs, never the fast gate."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import pytest
from capsem_builder.gate.tools.ci import classify_ci_scope as CLASSIFIER

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "build_system" / "scripts" / "ci" / "classify-ci-scope.py"


def _classifier():
    return CLASSIFIER


@pytest.mark.parametrize(
    "paths",
    [
        (),
        (".github/workflows/docs.yaml",),
        (".github/workflows/ci.yaml",),
        ("scripts/smoke-docs-site.sh",),
        ("tests/test_docs_holding_contract.py",),
        ("site/public/install.sh",),
        ("docs/public/install.sh",),
        ("later/release-note.md",),
        ("unknown-new-root/file.md",),
        ("docs/index.md", "build_system/builder/gate/command.py"),
    ],
)
def test_ambiguous_or_executable_changes_fail_closed(paths: tuple[str, ...]) -> None:
    assert _classifier().web_only(paths) is False


@pytest.mark.parametrize(
    "paths",
    [
        ("README.md",),
        ("docs/src/content/docs/index.mdx",),
        ("site/src/pages/index.astro", "docs/src/pages/index.astro"),
    ],
)
def test_only_inert_web_content_may_skip_expensive_product_jobs(paths: tuple[str, ...]) -> None:
    assert _classifier().web_only(paths) is True


def test_null_delimited_git_paths_are_parsed_without_ambiguity() -> None:
    module = _classifier()
    assert module.paths_from_git(b"docs/a file.md\0site/src/index.astro\0") == (
        "docs/a file.md",
        "site/src/index.astro",
    )
    with pytest.raises(ValueError, match="NUL-terminated"):
        module.paths_from_git(b"docs/index.md")


def test_cli_emits_independent_scopes_and_job_owners_without_changing_the_default_contract() -> None:
    payload = b"web/app/src/App.svelte\0web/docs/src/index.mdx\0"
    default = subprocess.run(
        (sys.executable, str(SCRIPT)),
        cwd=ROOT,
        input=payload,
        check=True,
        capture_output=True,
    )
    scopes = subprocess.run(
        (sys.executable, str(SCRIPT), "--scopes"),
        cwd=ROOT,
        input=payload,
        check=True,
        capture_output=True,
    )
    owners = subprocess.run(
        (sys.executable, str(SCRIPT), "--owners"),
        cwd=ROOT,
        input=payload,
        check=True,
        capture_output=True,
    )
    assert default.stdout == b"false\n"
    assert scopes.stdout == b'["app", "docs"]\n'
    assert owners.stdout == (
        b'["docs-build", "fast-gate", "pr-gate", "test", "test-install", "test-linux"]\n'
    )


def test_cli_rejects_an_unknown_output_mode() -> None:
    result = subprocess.run(
        (sys.executable, str(SCRIPT), "--unknown"),
        cwd=ROOT,
        input=b"README.md\0",
        check=False,
        capture_output=True,
    )
    assert result.returncode == 2
    assert b"unknown classifier mode" in result.stderr


@pytest.mark.parametrize(
    ("path", "owners"),
    [
        ("crates/capsem-core/src/lib.rs", {"fast-gate", "test-linux", "test", "test-install", "pr-gate"}),
        ("build_system/builder/gate/cli.py", {"fast-gate", "test-linux", "test", "test-install", "pr-gate"}),
        ("web/docs/src/index.mdx", {"fast-gate", "docs-build", "pr-gate"}),
        ("web/marketing/src/index.astro", {"fast-gate", "site-build", "pr-gate"}),
        ("build_system/release_site/src/index.astro", {"fast-gate", "release-site-build", "pr-gate"}),
        (".github/workflows/ci.yaml", {"fast-gate", "test-linux", "test", "test-install", "docs-build", "site-build", "release-site-build", "pr-gate"}),
    ],
)
def test_each_surface_maps_to_required_ci_owners(
    path: str, owners: set[str]
) -> None:
    assert _classifier().ci_owners((path,)) == owners


def test_unknown_empty_and_malformed_paths_fail_closed() -> None:
    module = _classifier()
    for paths in [(), ("later/new.md",), ("unknown/file",), ("/absolute",), ("../escape",)]:
        with pytest.raises(ValueError):
            module.ci_owners(paths)


def test_rename_classifies_both_old_and_new_owners() -> None:
    owners = _classifier().ci_owners(
        ("build_system/builder/gate/cli.py", "build_system/builder/gate/cli.py")
    )
    assert {"fast-gate", "test-linux", "test", "test-install", "pr-gate"} == owners


@pytest.mark.parametrize(
    ("path", "scope"),
    [
        ("build_system/builder/gate/cli.py", "build_system"),
        ("web/app/src/App.svelte", "app"),
        ("web/docs/src/index.mdx", "docs"),
        ("web/marketing/src/index.astro", "marketing_graphics"),
        ("web/graphics/logo.svg", "marketing_graphics"),
        ("build_system/release_site/src/index.astro", "release_site"),
        ("crates/capsem-core/src/lib.rs", "rust_guest_config"),
        ("guest/artifacts/capsem-init", "rust_guest_config"),
        ("config/profiles/code/profile.toml", "rust_guest_config"),
        ("benchmarks/collectors/routes", "benchmarks"),
        ("benchmarks/baselines/routes/data.json", "benchmarks"),
        ("sdk/client.py", "sdk"),
    ],
)
def test_each_approved_target_has_one_independent_scope(path: str, scope: str) -> None:
    assert _classifier().ci_scopes((path,)) == {scope}


def test_shared_hidden_and_root_inputs_fan_out() -> None:
    module = _classifier()
    for path in (
        ".github/workflows/ci.yaml",
        ".config/ty.toml",
        "justfile",
        "config/gate.toml",
        "scripts/check-web-surface.sh",
        "scripts/lib/exec_lock.sh",
        "build_system/scripts/ci/classify-ci-scope.py",
        "build_system/scripts/ci/require-ci-jobs.sh",
        "build_system/builder/gate/tools/ci/classify_ci_scope.py",
    ):
        assert module.ci_scopes((path,)) == {"shared"}
        assert module.ci_owners((path,)) == module.ALL_JOBS
    assert module.ci_owners((".config/ty.toml",)) == module.ALL_JOBS


@pytest.mark.parametrize(
    "path",
    [
        "scripts/check-docs-holding-build.py",
        "build_system/builder/gate/tools/web/check_docs_holding_build.py",
    ],
)
def test_docs_build_controls_reach_the_docs_and_build_system_owners(path: str) -> None:
    module = _classifier()
    assert module.ci_scopes((path,)) == {"build_system", "docs"}
    assert module.ci_owners((path,)) == module.PRODUCT_JOBS | {
        "docs-build",
        "fast-gate",
        "pr-gate",
    }


@pytest.mark.parametrize(
    ("path", "scopes"),
    [
        (
            "build_system/tests/release_site/test_release_site_rendering.py",
            {"release_site"},
        ),
        (
            "build_system/builder/gate/releasegraph.py",
            {"build_system", "release_site"},
        ),
    ],
)
def test_release_site_generator_tests_and_graph_inputs_reach_the_site_owner(
    path: str, scopes: set[str]
) -> None:
    module = _classifier()
    assert module.ci_scopes((path,)) == scopes
    assert "release-site-build" in module.ci_owners((path,))


def test_deletion_is_classified_by_its_absent_path() -> None:
    assert _classifier().ci_scopes(("web/docs/src/removed.mdx",)) == {"docs"}


def test_rename_unions_old_and_new_independent_scopes() -> None:
    scopes = _classifier().ci_scopes(
        ("scripts/build.py", "web/app/src/build-status.ts")
    )
    assert scopes == {"build_system", "app"}


@pytest.mark.parametrize(
    "path",
    ["web/unknown/new.ts", "build_system/unknown/new.py", "unknown/new.md"],
)
def test_unknown_target_subtrees_fail_closed(path: str) -> None:
    with pytest.raises(ValueError, match="unowned"):
        _classifier().ci_scopes((path,))


def test_public_installer_changes_reach_product_and_web_owners() -> None:
    module = _classifier()
    assert module.ci_scopes(("web/docs/public/install.sh",)) == {
        "docs",
        "rust_guest_config",
    }
    assert module.ci_owners(("web/docs/public/install.sh",)) >= module.PRODUCT_JOBS


def test_every_current_tracked_path_has_ci_owners() -> None:
    output = subprocess.run(
        ("git", "ls-files", "-z"),
        cwd=ROOT,
        check=True,
        capture_output=True,
    ).stdout
    paths = tuple(path.decode() for path in output.split(b"\0") if path)
    assert _classifier().ci_owners(paths)
    assert _classifier().ci_scopes(paths)
