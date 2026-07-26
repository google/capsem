from __future__ import annotations

import importlib.util
from pathlib import Path
import sys

import pytest


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "check-cargo-audit.py"
SPEC = importlib.util.spec_from_file_location("check_cargo_audit", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
AUDIT = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = AUDIT
SPEC.loader.exec_module(AUDIT)


def _report(
    *,
    vulnerabilities: int = 0,
    warning_kind: str | None = None,
) -> dict:
    warnings = {}
    if warning_kind is not None:
        warnings[warning_kind] = [
            {
                "advisory": {"id": "RUSTSEC-test"},
                "package": {"name": "example", "version": "1.0.0"},
            }
        ]
    return {
        "vulnerabilities": {"count": vulnerabilities, "list": []},
        "warnings": warnings,
    }


@pytest.mark.parametrize("warning_kind", ("unsound", "yanked"))
def test_actionable_cargo_audit_warnings_are_blocking(
    warning_kind: str,
) -> None:
    with pytest.raises(ValueError, match=warning_kind):
        AUDIT.validate_report(_report(warning_kind=warning_kind))


def test_cargo_audit_vulnerabilities_are_blocking() -> None:
    with pytest.raises(ValueError, match="vulnerabil"):
        AUDIT.validate_report(_report(vulnerabilities=1))


def test_unmaintained_only_warnings_remain_visible_but_nonblocking() -> None:
    assert AUDIT.validate_report(_report(warning_kind="unmaintained")) == {
        "unmaintained": 1
    }


def test_all_shared_rust_audit_callers_use_the_strict_wrapper() -> None:
    justfile = (ROOT / "Justfile").read_text(encoding="utf-8")
    security = (
        ROOT / ".github" / "workflows" / "security-audit.yaml"
    ).read_text(encoding="utf-8")

    assert justfile.count("python3 scripts/check-cargo-audit.py") == 1
    assert "just _test-fast" in justfile
    assert "cargo audit &" not in justfile
    assert "run: python3 scripts/check-cargo-audit.py" in security
