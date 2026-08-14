"""The CI shortcut may skip expensive product jobs, never the fast gate."""

from __future__ import annotations

import importlib.util
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
        ("later/release-note.md",),
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
