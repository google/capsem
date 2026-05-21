from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]
DOCS_ROOT = PROJECT_ROOT / "docs" / "src" / "content" / "docs"


def _doc(path: str) -> str:
    return (DOCS_ROOT / path).read_text(encoding="utf-8")


def test_admin_cli_docs_cover_corp_and_developer_install_paths() -> None:
    doc = _doc("usage/admin-cli.md")

    assert "python -m pip install capsem" in doc
    assert "uv sync" in doc
    assert "uv run capsem-admin --version" in doc
    assert "capsem-admin detection compile" in doc
    assert "capsem-admin policy validate" in doc


def test_detection_docs_require_pysigma_and_detection_ir() -> None:
    doc = _doc("security/detection.md")

    assert "pySigma" in doc
    assert "capsem.detection.ir.v1" in doc
    assert "capsem-admin detection check" in doc
    assert "Rejected constructs fail closed" in doc
    assert "avoiding a second, ad hoc Sigma" in doc
    assert "implementation inside Capsem" in doc


def test_enforcement_docs_keep_policy_and_detection_separate() -> None:
    doc = _doc("security/enforcement.md")

    assert "Do not use Sigma as a blocking policy language" in doc
    assert "capsem-admin policy validate" in doc
    assert "`allow`, `block`, `ask`, or `rewrite`" in doc


def test_corp_custom_image_docs_use_profile_admin_flow() -> None:
    doc = _doc("architecture/custom-images.md")

    assert "capsem-admin profile init" in doc
    assert "capsem-admin image build" in doc
    assert "capsem-admin detection compile" in doc
    assert "capsem-builder init" not in doc
