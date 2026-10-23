"""The public docs deployment is a source-derived release-line holding surface."""

from __future__ import annotations

import importlib
import json
import tomllib
from pathlib import Path

import pytest
from capsem_builder.gate.tools.web import check_docs_holding_build as DOCS_HOLDING_BUILD
from helpers.workflow_contract import workflow_reachable_text

PROJECT_ROOT = Path(__file__).resolve().parents[1]
DOCS_ROOT = PROJECT_ROOT / "web" / "docs"
GATE_CONFIG = PROJECT_ROOT / "config" / "gate.toml"
RELEASE_LINE = tomllib.loads(GATE_CONFIG.read_text(encoding="utf-8"))["release"]["line"]


def _verifier_module():
    return importlib.reload(DOCS_HOLDING_BUILD)


def _holding_html() -> str:
    return f"""<!doctype html>
    <html><body><main>
      <h1>Capsem {RELEASE_LINE} documentation</h1>
      <p>Capsem {RELEASE_LINE} is in pre-release qualification.</p>
      <p>Documentation is being prepared.</p>
    </main></body></html>
    """


def _tombstone_html() -> str:
    return f"""<!doctype html>
    <html><head>
      <meta name="robots" content="noindex, nofollow">
    </head><body><main>
      <h1>Documentation route unavailable</h1>
      <p>Capsem {RELEASE_LINE} is in pre-release qualification.</p>
      <p>Documentation is being prepared.</p>
    </main></body></html>
    """


def _write_manual_sources(source_root: Path) -> None:
    (source_root / "architecture").mkdir(parents=True)
    (source_root / "usage").mkdir()
    (source_root / "index.mdx").write_text("root", encoding="utf-8")
    (source_root / "getting-started.md").write_text("guide", encoding="utf-8")
    (source_root / "architecture" / "service-api.md").write_text("api", encoding="utf-8")
    (source_root / "usage" / "index.mdx").write_text("usage", encoding="utf-8")


def _write_holding_artifact(dist: Path) -> None:
    (dist / "index.html").write_text(_holding_html(), encoding="utf-8")
    (dist / "404.html").write_text(
        f"""<!doctype html><html><body><main>
        <h1>Documentation route unavailable</h1>
        <p>Capsem {RELEASE_LINE} is in pre-release qualification.</p>
        </main></body></html>""",
        encoding="utf-8",
    )
    (dist / "_headers").write_text(
        "/*\n  Cache-Control: no-store, no-cache, must-revalidate, max-age=0\n"
        "  CDN-Cache-Control: no-store\n"
        "  Cloudflare-CDN-Cache-Control: no-store\n"
        "  X-Robots-Tag: noindex, nofollow\n",
        encoding="utf-8",
    )
    for route in ("getting-started", "architecture/service-api", "usage"):
        output = dist / route / "index.html"
        output.parent.mkdir(parents=True)
        output.write_text(_tombstone_html(), encoding="utf-8")


def test_docs_build_verifier_accepts_exact_source_derived_tombstones(tmp_path: Path) -> None:
    verifier = _verifier_module()
    source_root = tmp_path / "manual"
    dist = tmp_path / "dist"
    source_root.mkdir()
    dist.mkdir()
    _write_manual_sources(source_root)
    _write_holding_artifact(dist)

    verifier.verify_holding_build(dist, source_root)


def test_docs_build_verifier_rejects_a_missing_source_derived_tombstone(tmp_path: Path) -> None:
    verifier = _verifier_module()
    source_root = tmp_path / "manual"
    dist = tmp_path / "dist"
    source_root.mkdir()
    dist.mkdir()
    _write_manual_sources(source_root)
    _write_holding_artifact(dist)
    (dist / "getting-started" / "index.html").unlink()

    with pytest.raises(verifier.HoldingBuildError, match=r"getting-started/index\.html"):
        verifier.verify_holding_build(dist, source_root)


def test_docs_build_verifier_rejects_an_unexpected_route_or_installer(tmp_path: Path) -> None:
    verifier = _verifier_module()
    source_root = tmp_path / "manual"
    dist = tmp_path / "dist"
    source_root.mkdir()
    dist.mkdir()
    _write_manual_sources(source_root)
    _write_holding_artifact(dist)
    (dist / "install.sh").write_text("old installer", encoding="utf-8")

    with pytest.raises(verifier.HoldingBuildError, match=r"install\.sh"):
        verifier.verify_holding_build(dist, source_root)


def test_docs_build_verifier_rejects_an_unmaterialized_custom_slug(tmp_path: Path) -> None:
    verifier = _verifier_module()
    source_root = tmp_path / "manual"
    dist = tmp_path / "dist"
    source_root.mkdir()
    dist.mkdir()
    _write_manual_sources(source_root)
    _write_holding_artifact(dist)
    (source_root / "getting-started.md").write_text(
        "---\nslug: retired-guide\n---\nold guide\n", encoding="utf-8"
    )

    with pytest.raises(verifier.HoldingBuildError, match="custom slug"):
        verifier.verify_holding_build(dist, source_root)


@pytest.mark.parametrize("leaked", ["Getting Started", "Starlight", "install.sh"])
def test_docs_build_verifier_rejects_old_content_in_a_tombstone(
    tmp_path: Path, leaked: str
) -> None:
    verifier = _verifier_module()
    source_root = tmp_path / "manual"
    dist = tmp_path / "dist"
    source_root.mkdir()
    dist.mkdir()
    _write_manual_sources(source_root)
    _write_holding_artifact(dist)
    tombstone = dist / "getting-started" / "index.html"
    tombstone.write_text(_tombstone_html() + leaked, encoding="utf-8")

    with pytest.raises(verifier.HoldingBuildError, match=leaked):
        verifier.verify_holding_build(dist, source_root)


def test_docs_publishes_the_approved_manual() -> None:
    astro = (DOCS_ROOT / "astro.config.mjs").read_text()
    preview = (DOCS_ROOT / "astro.preview.config.mjs").read_text()
    package = json.loads((DOCS_ROOT / "package.json").read_text())
    assert "export { default } from './astro.preview.config.mjs'" in astro
    assert "starlight(" in preview
    assert "srcDir: './preview'" in preview
    assert "tools/check-preview.py dist" in package["scripts"]["build"]
    assert (DOCS_ROOT / "src/content/docs/architecture/service-api.md").is_file()


def test_docs_deploy_smokes_the_manual() -> None:
    workflow = workflow_reachable_text(
        PROJECT_ROOT, PROJECT_ROOT / ".github/workflows/docs.yaml"
    )
    for title in ("Install Capsem", "Getting started with the SDK", "Security architecture"):
        assert title in workflow
    assert "pre-release qualification" not in workflow


def test_readme_does_not_advertise_unreleased_install_or_deep_docs() -> None:
    readme = (PROJECT_ROOT / "README.md").read_text(encoding="utf-8")

    assert f"Capsem {RELEASE_LINE}" in readme
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
