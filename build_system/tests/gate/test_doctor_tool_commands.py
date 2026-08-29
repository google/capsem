"""Direct behavior checks for gate-owned diagnostic commands."""

from __future__ import annotations

import sys

import pytest
from capsem_builder.gate.tools.doctor import check_session, kvm_diagnostic


def test_session_list_preserves_empty_ledger_failure_status(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(check_session, "list_recent_sessions", lambda _count: [])
    monkeypatch.setattr(sys, "argv", ["check_session.py", "--list"])

    with pytest.raises(SystemExit) as failure:
        check_session.main()

    assert failure.value.code == 1


def test_session_list_success_returns_zero_status(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    row = {
        "id": "vm-123",
        "mode": "persistent",
        "status": "stopped",
        "created_at": "2026-08-29T00:00:00Z",
        "allowed_requests": 2,
        "total_requests": 3,
        "total_input_tokens": 4,
        "total_output_tokens": 5,
        "total_estimated_cost": 0.0,
        "total_tool_calls": 6,
        "total_file_events": 7,
    }
    monkeypatch.setattr(check_session, "list_recent_sessions", lambda _count: [row])
    monkeypatch.setattr(sys, "argv", ["check_session.py", "--list"])

    assert check_session.main() is None


def test_kvm_diagnostic_preserves_missing_device_failure_status(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(kvm_diagnostic.os.path, "exists", lambda _path: False)

    with pytest.raises(SystemExit) as failure:
        kvm_diagnostic.main()

    assert failure.value.code == 1
