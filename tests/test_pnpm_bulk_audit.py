from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
SCRIPT = PROJECT_ROOT / "scripts" / "audit-pnpm-bulk.py"


def _load_module():
    spec = importlib.util.spec_from_file_location("audit_pnpm_bulk", SCRIPT)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_bulk_audit_collects_complete_recursive_dependency_versions() -> None:
    audit = _load_module()
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
    audit = _load_module()
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
    audit = _load_module()

    try:
        audit.advisory_failures([])
    except ValueError as error:
        assert "JSON object" in str(error)
    else:
        raise AssertionError("malformed advisory response was accepted")


def test_default_bulk_audit_covers_every_web_workspace() -> None:
    audit = _load_module()

    assert (
        Path("frontend"),
        Path("docs"),
        Path("site"),
        Path("release-site"),
    ) == audit.DEFAULT_PROJECT_DIRS
    for project in audit.DEFAULT_PROJECT_DIRS:
        assert (PROJECT_ROOT / project / "package.json").is_file()
        assert (PROJECT_ROOT / project / "pnpm-lock.yaml").is_file()


def test_every_fast_gate_blocks_on_bulk_dependency_advisories() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")
    ci = (PROJECT_ROOT / ".github/workflows/ci.yaml").read_text(encoding="utf-8")
    fast_gate = (PROJECT_ROOT / ".github/workflows/fast-gate.yaml").read_text(encoding="utf-8")
    scheduled = (PROJECT_ROOT / ".github/workflows/security-audit.yaml").read_text(encoding="utf-8")

    for source in (justfile, scheduled):
        assert "scripts/audit-pnpm-bulk.py" in source
        assert "pnpm audit" not in source
        assert "--ignore-registry-errors" not in source

    assert "uses: ./.github/workflows/fast-gate.yaml" in ci
    assert "fast-gate" in ci.split("  pr-gate:", 1)[1]
    assert "run: just _test-fast" in fast_gate
    assert "npm bulk advisory audit (blocking security signal)" in scheduled
    assert "run: python3 scripts/audit-pnpm-bulk.py" in scheduled
    assert "--project-dir frontend" not in scheduled
    for lockfile in (
        "frontend/pnpm-lock.yaml",
        "docs/pnpm-lock.yaml",
        "site/pnpm-lock.yaml",
        "release-site/pnpm-lock.yaml",
    ):
        assert lockfile in fast_gate
        assert lockfile in scheduled

    # Audit-only convenience is not a public Just fork.
    assert "\naudit:" not in justfile

    assert "cargo audit reported advisories; see the security-audit workflow" not in justfile
    assert "npm audit reported advisories; see the security-audit workflow" not in justfile
    assert "python3 scripts/check-cargo-audit.py & PID_CARGO_AUDIT=$!" in justfile
    assert "python3 scripts/audit-pnpm-bulk.py & PID_PNPM_AUDIT=$!" in justfile
    assert "--project-dir frontend" not in justfile
    assert (
        'wait $PID_CARGO_AUDIT || { echo "strict cargo audit failed"; FAIL=1; }'
        in justfile
    )
    assert 'wait $PID_PNPM_AUDIT || { echo "npm bulk audit failed"; FAIL=1; }' in justfile
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
