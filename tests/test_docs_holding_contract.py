"""The public docs deployment is a single Capsem 0.6 holding page."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[1]
DOCS_ROOT = PROJECT_ROOT / "docs"
VERIFIER = PROJECT_ROOT / "scripts" / "check-docs-holding-build.py"


def _verifier_module():
    assert VERIFIER.is_file(), "the docs build has no artifact-level holding-page verifier"
    spec = importlib.util.spec_from_file_location("check_docs_holding_build", VERIFIER)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _holding_html() -> str:
    return """<!doctype html>
    <html><body><main>
      <h1>Capsem 0.6 documentation</h1>
      <p>Capsem 0.6 is in pre-release qualification.</p>
      <p>Documentation is being prepared.</p>
    </main></body></html>
    """


def _write_holding_artifact(dist: Path) -> None:
    (dist / "index.html").write_text(_holding_html(), encoding="utf-8")
    (dist / "404.html").write_text(
        """<!doctype html><html><body><main>
        <h1>Documentation route unavailable</h1>
        <p>Capsem 0.6 is in pre-release qualification.</p>
        </main></body></html>""",
        encoding="utf-8",
    )


def test_docs_build_verifier_accepts_the_holding_page_and_platform_404(tmp_path: Path) -> None:
    verifier = _verifier_module()
    _write_holding_artifact(tmp_path)

    verifier.verify_holding_build(tmp_path)


def test_docs_build_verifier_rejects_a_former_deep_route(tmp_path: Path) -> None:
    verifier = _verifier_module()
    _write_holding_artifact(tmp_path)
    old_route = tmp_path / "getting-started" / "index.html"
    old_route.parent.mkdir()
    old_route.write_text("old installation guide", encoding="utf-8")

    with pytest.raises(verifier.HoldingBuildError, match=r"getting-started/index\.html"):
        verifier.verify_holding_build(tmp_path)


def test_docs_build_verifier_rejects_a_public_installer_copy(tmp_path: Path) -> None:
    verifier = _verifier_module()
    _write_holding_artifact(tmp_path)
    (tmp_path / "install.sh").write_text("old installer", encoding="utf-8")

    with pytest.raises(verifier.HoldingBuildError, match=r"install\.sh"):
        verifier.verify_holding_build(tmp_path)


def test_docs_source_selects_one_holding_route_without_deleting_the_manual() -> None:
    astro = (DOCS_ROOT / "astro.config.mjs").read_text(encoding="utf-8")
    content_config = (DOCS_ROOT / "src" / "content.config.ts").read_text(encoding="utf-8")
    page = (DOCS_ROOT / "src" / "pages" / "index.astro").read_text(encoding="utf-8")
    not_found = (DOCS_ROOT / "src" / "pages" / "404.astro").read_text(encoding="utf-8")
    package = json.loads((DOCS_ROOT / "package.json").read_text(encoding="utf-8"))
    detailed_sources = sorted((DOCS_ROOT / "src" / "content" / "docs").rglob("*.md*"))

    assert "starlight(" not in astro
    assert "publicDir: './public-holding'" in astro
    assert "docsLoader" not in content_config
    assert "export const collections = {};" in content_config
    assert "Capsem 0.6 documentation" in page
    assert "pre-release" in page
    assert "Documentation is being prepared" in page
    assert "Documentation route unavailable" in not_found
    assert "pre-release qualification" in not_found
    assert "releases/latest" not in page
    assert "install.sh" not in page
    assert 'href="/favicon.svg"' not in page
    assert 'src="/logo.svg"' not in page
    assert "/getting-started/" not in page
    assert package["scripts"]["build"] == (
        "astro build && python3 ../scripts/check-docs-holding-build.py dist"
    )
    assert len(detailed_sources) == 48
    assert DOCS_ROOT / "src" / "content" / "docs" / "index.mdx" in detailed_sources
    assert (
        DOCS_ROOT / "src" / "content" / "docs" / "architecture" / "service-api.md"
        in detailed_sources
    )


def test_docs_deploy_smokes_the_holding_page_and_a_missing_old_route() -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / "docs.yaml").read_text(encoding="utf-8")

    assert "Capsem 0.6 documentation" in workflow
    assert "pre-release qualification" in workflow
    assert "OLD_ROUTE_URL: https://docs.capsem.org/getting-started/" in workflow
    assert 'test "$OLD_ROUTE_STATUS" = 404' in workflow
    assert 'href="/getting-started/"' not in workflow


def test_readme_does_not_advertise_unreleased_install_or_deep_docs() -> None:
    readme = (PROJECT_ROOT / "README.md").read_text(encoding="utf-8")

    assert "Capsem 0.6" in readme
    assert "pre-release" in readme
    assert "releases/latest" not in readme
    assert "install.sh" not in readme
    assert "## Install" not in readme
    for old_route in (
        "/getting-started",
        "/architecture/",
        "/security/",
        "/usage/",
        "/benchmarks/",
        "/debugging/",
        "/development/",
        "/releases/",
    ):
        assert old_route not in readme
