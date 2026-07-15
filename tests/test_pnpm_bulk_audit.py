from __future__ import annotations

import importlib.util
from pathlib import Path
import sys


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
                    "dependencies": {
                        "shared": {"from": "shared", "version": "3.0.0"}
                    },
                }
            },
            "devDependencies": {
                "beta": {
                    "from": "beta",
                    "version": "4.0.0",
                    "optionalDependencies": {
                        "shared": {"from": "shared", "version": "3.1.0"}
                    },
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

    assert failures == [
        "alpha: high: unsafe alpha (<=2.0.0) https://example.test/advisories/123"
    ]


def test_bulk_audit_rejects_malformed_registry_response() -> None:
    audit = _load_module()

    try:
        audit.advisory_failures([])
    except ValueError as error:
        assert "JSON object" in str(error)
    else:
        raise AssertionError("malformed advisory response was accepted")


def test_all_release_gates_use_bulk_audit_without_registry_error_bypass() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")
    ci = (PROJECT_ROOT / ".github/workflows/ci.yaml").read_text(encoding="utf-8")

    for source in (justfile, ci):
        assert "scripts/audit-pnpm-bulk.py" in source
        assert "pnpm audit" not in source
        assert "--ignore-registry-errors" not in source
