from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
from capsem_builder.cache.config import load_policy
from capsem_builder.gate.config import for_root
from capsem_builder.gate.tools.audit import python_lock
from capsem_builder.gate.tools.audit.python_lock import audit_python_lock

PROJECT_ROOT = Path(__file__).resolve().parents[3]
POLICY = for_root(PROJECT_ROOT).audits.python_lock_policy
CACHE_POLICY = load_policy(PROJECT_ROOT)


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

    def unexpected_runner(*_args, **_kwargs) -> subprocess.CompletedProcess[str]:
        raise AssertionError("cache validation must precede subprocess work")

    with pytest.raises(ValueError, match="configured cache stage"):
        audit_python_lock(PROJECT_ROOT, POLICY, runner=unexpected_runner)


def test_private_checkout_uses_the_outer_shared_cache_authority(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    config = for_root(PROJECT_ROOT)
    private_config = config.model_copy(update={"root": tmp_path})
    policy_dir = tmp_path / "config"
    policy_dir.mkdir()
    (policy_dir / "cache.toml").write_text(
        (PROJECT_ROOT / "config/cache.toml").read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    shared = PROJECT_ROOT / "cache/tools/python/uv"
    monkeypatch.setenv(CACHE_POLICY.authority_environment, str(PROJECT_ROOT))
    monkeypatch.setenv(config.environment.uv_cache, str(shared))
    monkeypatch.setattr(python_lock, "for_root", lambda _root: private_config)
    issued: list[tuple[str, ...]] = []

    def runner(argv, **_kwargs):
        command = tuple(argv)
        issued.append(command)
        return completed(command, 0)

    assert audit_python_lock(tmp_path, POLICY, runner=runner) == 0
    audit = issued[1]
    assert audit[audit.index("--cache-dir") + 1] == str(shared / "pip-audit")
