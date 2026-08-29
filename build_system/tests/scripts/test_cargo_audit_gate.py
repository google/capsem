from __future__ import annotations

from pathlib import Path

import pytest
from capsem_builder.gate.tools.audit import cargo_audit as AUDIT

ROOT = Path(__file__).resolve().parents[3]

def _gate_issues(name: str | None = None) -> str:
    """Everything the gate would issue, with real argv. See `helpers.gate`."""
    import sys as _sys

    _sys.path.insert(0, str(ROOT / "tests"))
    from helpers.gate import gate_issues

    return gate_issues(name)



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
    justfile = (ROOT / "justfile").read_text(encoding="utf-8")
    security = (
        ROOT / ".github" / "workflows" / "security-audit.yaml"
    ).read_text(encoding="utf-8")

    assert _gate_issues().count("check-cargo-audit.py") >= 1
    assert "cargo audit &" not in justfile
    assert "capsem-gate test-fast" in justfile
    assert "run: python3 scripts/check-cargo-audit.py" in security
