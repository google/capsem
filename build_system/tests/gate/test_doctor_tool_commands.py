"""Direct behavior checks for gate-owned diagnostic commands."""

from __future__ import annotations

import sys
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.gate.tools.doctor import (
    check_session,
    check_session_report,
    doctor_session_test,
    kvm_diagnostic,
)


def test_session_report_public_entrypoint_stays_stable() -> None:
    assert check_session.check_session is check_session_report.check_session


def test_doctor_ledger_public_entrypoint_delegates_with_main_db(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    session_dir = tmp_path / "session"
    main_db = tmp_path / "main.db"
    seen: list[tuple[str, Path, Path]] = []

    def fake_verify(session_id: str, path: Path, *, main_db: Path) -> bool:
        seen.append((session_id, path, main_db))
        return True

    monkeypatch.setattr(doctor_session_test, "MAIN_DB", main_db)
    monkeypatch.setattr(doctor_session_test, "_verify_session", fake_verify)

    assert doctor_session_test.verify_session("vm-123", session_dir)
    assert seen == [("vm-123", session_dir, main_db)]


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
    missing_kvm = SimpleNamespace(path=SimpleNamespace(exists=lambda _path: False))
    monkeypatch.setattr(kvm_diagnostic, "os", missing_kvm)

    with pytest.raises(SystemExit) as failure:
        kvm_diagnostic.main()

    assert failure.value.code == 1
