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


def test_doctor_ledger_public_entrypoint_delegates_with_the_host_ledger(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    session_dir = tmp_path / "session"
    host_ledger = tmp_path / "host.db"
    seen: list[tuple[str, Path, Path]] = []

    def fake_verify(session_id: str, path: Path, *, host_ledger: Path) -> bool:
        seen.append((session_id, path, host_ledger))
        return True

    monkeypatch.setattr(doctor_session_test, "HOST_LEDGER", host_ledger)
    monkeypatch.setattr(doctor_session_test, "_verify_session", fake_verify)

    assert doctor_session_test.verify_session("vm-123", session_dir)
    assert seen == [("vm-123", session_dir, host_ledger)]


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
    row = {"id": "vm-123", "status": "stopped", "created_at_ms": 1, "stopped_at_ms": 2}
    monkeypatch.setattr(check_session, "list_recent_sessions", lambda _count: [row])
    monkeypatch.setattr(sys, "argv", ["check_session.py", "--list"])

    assert check_session.main() == 0


def test_kvm_diagnostic_preserves_missing_device_failure_status(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    missing_kvm = SimpleNamespace(path=SimpleNamespace(exists=lambda _path: False))
    monkeypatch.setattr(kvm_diagnostic, "os", missing_kvm)

    with pytest.raises(SystemExit) as failure:
        kvm_diagnostic.main()

    assert failure.value.code == 1
