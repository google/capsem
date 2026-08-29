from __future__ import annotations

import json
from pathlib import Path

from capsem_builder.gate.tools.audit import pnpm_bulk as audit

PROJECT_ROOT = Path(__file__).resolve().parents[3]

def _gate_issues(name: str | None = None) -> str:
    """Everything the gate would issue, with real argv. See `helpers.gate`."""
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_issues

    return gate_issues(name)



def test_bulk_audit_collects_complete_recursive_dependency_versions() -> None:
    tree = [
        {
            "name": "app",
            "version": "1.0.0",
            "dependencies": {
                "alpha": {
                    "from": "alpha",
                    "version": "2.0.0",
                    "dependencies": {"shared": {"from": "shared", "version": "3.0.0"}},
                }
            },
            "devDependencies": {
                "beta": {
                    "from": "beta",
                    "version": "4.0.0",
                    "optionalDependencies": {"shared": {"from": "shared", "version": "3.1.0"}},
                }
            },
        }
    ]

    assert audit.collect_versions(tree) == {
        "alpha": ["2.0.0"],
        "app": ["1.0.0"],
        "beta": ["4.0.0"],
        "shared": ["3.0.0", "3.1.0"],
    }


def test_bulk_audit_rejects_every_returned_advisory() -> None:
    advisories = {
        "alpha": [
            {
                "id": 123,
                "severity": "high",
                "title": "unsafe alpha",
                "url": "https://example.test/advisories/123",
                "vulnerable_versions": "<=2.0.0",
            }
        ]
    }

    failures = audit.advisory_failures(advisories)

    assert failures == ["alpha: high: unsafe alpha (<=2.0.0) https://example.test/advisories/123"]


def test_bulk_audit_rejects_malformed_registry_response() -> None:

    try:
        audit.advisory_failures([])
    except ValueError as error:
        assert "JSON object" in str(error)
    else:
        raise AssertionError("malformed advisory response was accepted")


def test_default_bulk_audit_covers_every_web_workspace() -> None:

    assert (
        Path("frontend"),
        Path("docs"),
        Path("site"),
        Path("build_system/release_site"),
    ) == audit.DEFAULT_PROJECT_DIRS
    for project in audit.DEFAULT_PROJECT_DIRS:
        assert (PROJECT_ROOT / project / "package.json").is_file()
        assert (PROJECT_ROOT / project / "pnpm-lock.yaml").is_file()


def test_every_fast_gate_blocks_on_bulk_dependency_advisories() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")
    ci = (PROJECT_ROOT / ".github/workflows/ci.yaml").read_text(encoding="utf-8")
    fast_gate = (PROJECT_ROOT / ".github/workflows/fast-gate.yaml").read_text(encoding="utf-8")
    scheduled = (PROJECT_ROOT / ".github/workflows/security-audit.yaml").read_text(encoding="utf-8")

    for source in (_gate_issues(), scheduled):
        assert "audit-pnpm-bulk.py" in source
        assert "pnpm audit" not in source
        assert "--ignore-registry-errors" not in source

    assert "uses: ./.github/workflows/fast-gate.yaml" in ci
    assert "fast-gate" in ci.split("  pr-gate:", 1)[1]
    assert "run: just fast-test" in fast_gate
    assert "run: uv run --project build_system --frozen capsem-gate test-release-contracts" in fast_gate
    assert "npm bulk advisory audit (blocking security signal)" in scheduled
    assert "run: python3 scripts/audit-pnpm-bulk.py" in scheduled
    assert "--project-dir frontend" not in scheduled
    for lockfile in (
        "frontend/pnpm-lock.yaml",
        "docs/pnpm-lock.yaml",
        "site/pnpm-lock.yaml",
        "build_system/release_site/pnpm-lock.yaml",
    ):
        assert lockfile in fast_gate
        assert lockfile in scheduled

    # Audit-only convenience is not a public Just fork.
    assert "\naudit:" not in justfile

    assert "cargo audit reported advisories; see the security-audit workflow" not in justfile
    assert "npm audit reported advisories; see the security-audit workflow" not in justfile
    # The shell ran the two audits with `&` and collected both exit statuses
    # into `FAIL`, so one advisory could not hide the other. They are two
    # independent steps of one phase now: the scheduler runs them together
    # because nothing orders them, and a failing step never cancels its peers.
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_plan

    plan = gate_plan("test-fast")
    assert plan.after_of("fast.audit.cargo") == plan.after_of("fast.audit.pnpm"), (
        "one audit waits on the other, so the first advisory hides the second"
    )
    assert "--project-dir frontend" not in _gate_issues()
    assert "continue-on-error: true" not in fast_gate


def test_frontend_owns_theme_css_without_preline_build_dependency() -> None:
    package = json.loads((PROJECT_ROOT / "frontend" / "package.json").read_text(encoding="utf-8"))
    global_css = (PROJECT_ROOT / "frontend" / "src" / "styles" / "global.css").read_text(
        encoding="utf-8"
    )
    lockfile = (PROJECT_ROOT / "frontend" / "pnpm-lock.yaml").read_text(encoding="utf-8")

    assert "preline" not in package["dependencies"]
    assert "preline" not in lockfile
    assert "node_modules/preline" not in global_css
    assert "preline/variants.css" not in global_css
    assert "preline/css/themes/theme.css" not in global_css
    assert '@import "./capsem-theme.css";' in global_css
