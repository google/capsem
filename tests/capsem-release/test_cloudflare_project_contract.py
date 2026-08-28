"""Focused contracts for the Cloudflare Pages release project preflight."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]


def _checker_module():
    module_path = PROJECT_ROOT / "scripts/check-cloudflare-pages-project.py"
    spec = importlib.util.spec_from_file_location("check_cloudflare_pages_project", module_path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _project(checker, production_branch: object) -> object:
    return checker.CloudflareResponse(
        200,
        {
            "success": True,
            "result": {
                "name": "release",
                "production_branch": production_branch,
            },
        },
    )


def test_project_preflight_accepts_the_exact_production_branch() -> None:
    checker = _checker_module()

    ok, detail = checker.validate_project_response(
        _project(checker, "release-production"), "release", "release-production"
    )

    assert ok is True
    assert "production branch release-production" in detail


@pytest.mark.parametrize("actual", ["preview-only", "", None])
def test_project_preflight_refuses_a_nonproduction_deploy_branch(actual: object) -> None:
    checker = _checker_module()

    ok, detail = checker.validate_project_response(
        _project(checker, actual), "release", "release-production"
    )

    assert ok is False
    assert "production branch" in detail
    assert "release-production" in detail
