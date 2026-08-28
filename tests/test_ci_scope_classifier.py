"""The CI shortcut may skip expensive product jobs, never the fast gate."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "classify-ci-scope.py"


def _classifier():
    spec = importlib.util.spec_from_file_location("classify_ci_scope", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


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
        ("docs/index.md", "src/capsem/gate/command.py"),
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
        ("src/capsem/gate/cli.py", "build_system/builder/gate/cli.py")
    )
    assert {"fast-gate", "test-linux", "test", "test-install", "pr-gate"} == owners


def test_every_current_tracked_path_has_ci_owners() -> None:
    output = subprocess.run(
        ("git", "ls-files", "-z"),
        cwd=ROOT,
        check=True,
        capture_output=True,
    ).stdout
    paths = tuple(path.decode() for path in output.split(b"\0") if path)
    assert _classifier().ci_owners(paths)
