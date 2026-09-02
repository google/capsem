from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
from capsem_builder.gate.config import for_root
from capsem_builder.gate.tools.audit.python_lock import audit_python_lock

PROJECT_ROOT = Path(__file__).resolve().parents[3]
POLICY = for_root(PROJECT_ROOT).audits.python_lock_policy


def completed(
    argv: tuple[str, ...], returncode: int, stderr: str = ""
) -> subprocess.CompletedProcess[str]:
    return subprocess.CompletedProcess(argv, returncode, stdout="", stderr=stderr)


def test_python_lock_export_and_audit_are_strict_and_cached() -> None:
    issued: list[tuple[str, ...]] = []

    def runner(argv, **_kwargs):
        command = tuple(argv)
        issued.append(command)
        return completed(command, 0)

    assert audit_python_lock(PROJECT_ROOT, POLICY, runner=runner) == 0
    export, audit = issued
    assert export[:2] == ("uv", "export")
    assert "--locked" in export and "--no-emit-project" in export
    assert audit[1:3] == ("-m", "pip_audit")
    assert audit[audit.index("--vulnerability-service") + 1] == "pypi"
    assert "--require-hashes" in audit and "--disable-pip" in audit
    assert "--cache-dir" in audit and "--timeout" in audit


def test_transient_service_failure_retries_then_succeeds() -> None:
    audits = iter((completed((), 1, "ServiceError"), completed((), 0)))
    sleeps: list[float] = []

    def runner(argv, **kwargs):
        return completed(tuple(argv), 0) if kwargs.get("check") else next(audits)

    assert audit_python_lock(PROJECT_ROOT, POLICY, runner=runner, sleep=sleeps.append) == 0
    assert sleeps == [POLICY.retry_seconds]


def test_transient_service_failure_exhausts_bounded_backoff() -> None:
    sleeps: list[float] = []
    audit_calls = 0

    def runner(argv, **kwargs):
        nonlocal audit_calls
        if kwargs.get("check"):
            return completed(tuple(argv), 0)
        audit_calls += 1
        return completed(tuple(argv), 1, "HTTPError: 503")

    assert audit_python_lock(PROJECT_ROOT, POLICY, runner=runner, sleep=sleeps.append) == 1
    assert audit_calls == POLICY.attempts
    assert sleeps == [
        POLICY.retry_seconds * attempt for attempt in range(1, POLICY.attempts)
    ]


def test_operational_failure_without_transient_marker_is_not_retried() -> None:
    audit_calls = 0

    def runner(argv, **kwargs):
        nonlocal audit_calls
        if kwargs.get("check"):
            return completed(tuple(argv), 0)
        audit_calls += 1
        return completed(tuple(argv), 2, "invalid requirements input")

    assert audit_python_lock(PROJECT_ROOT, POLICY, runner=runner) == 2
    assert audit_calls == 1


def test_vulnerability_failure_is_not_retried() -> None:
    audit_calls = 0

    def runner(argv, **kwargs):
        nonlocal audit_calls
        if kwargs.get("check"):
            return completed(tuple(argv), 0)
        audit_calls += 1
        return completed(tuple(argv), 1, "Found 1 known vulnerability")

    assert audit_python_lock(PROJECT_ROOT, POLICY, runner=runner) == 1
    assert audit_calls == 1


def test_inherited_uv_cache_cannot_escape_configured_stage(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    variable = for_root(PROJECT_ROOT).environment.uv_cache
    monkeypatch.setenv(variable, str(tmp_path / "outside-cache"))

    with pytest.raises(ValueError, match="configured cache stage"):
        audit_python_lock(PROJECT_ROOT, POLICY, runner=lambda *_args, **_kwargs: None)
