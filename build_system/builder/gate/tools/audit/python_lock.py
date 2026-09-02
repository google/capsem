"""Audit the locked Python graph with shared HTTP cache and bounded retries."""

from __future__ import annotations

import os
import subprocess
import sys
import time
from collections.abc import Callable
from pathlib import Path

from capsem_builder.gate import cachelayout, project_root
from capsem_builder.gate.buildschema import PythonLockAuditConfig
from capsem_builder.gate.config import for_root

Run = Callable[..., subprocess.CompletedProcess[str]]
Sleep = Callable[[float], None]
RETRYABLE_FAILURES = (
    "ServiceError",
    "ConnectionError",
    "ConnectTimeout",
    "ReadTimeout",
    "TimeoutError",
    "HTTPError: 500",
    "HTTPError: 502",
    "HTTPError: 503",
    "HTTPError: 504",
    "temporarily unavailable",
)


def _emit(result: subprocess.CompletedProcess[str]) -> None:
    sys.stdout.write(result.stdout or "")
    sys.stderr.write(result.stderr or "")


def _retryable(result: subprocess.CompletedProcess[str]) -> bool:
    output = f"{result.stdout or ''}\n{result.stderr or ''}"
    return any(marker in output for marker in RETRYABLE_FAILURES)


def _cache_base(variable: str, configured: Path) -> Path:
    configured = configured.resolve()
    inherited = os.environ.get(variable)
    if inherited is None:
        return configured
    candidate = Path(inherited).resolve()
    if candidate != configured:
        raise ValueError(f"{variable} must use configured cache stage {configured}, got {candidate}")
    return candidate


def audit_python_lock(
    root: Path,
    policy: PythonLockAuditConfig,
    *,
    runner: Run = subprocess.run,
    sleep: Sleep = time.sleep,
) -> int:
    """Export once, then audit with fail-closed bounded transient retries."""
    gate_config = for_root(root)
    cache_paths = cachelayout.cache_paths(gate_config)
    requirements = cache_paths.resolve(Path(policy.requirements))
    cache_base = _cache_base(
        gate_config.environment.uv_cache,
        cache_paths.stage(policy.cache_stage),
    )
    http_cache = cache_base / policy.cache_subdirectory
    requirements.parent.mkdir(parents=True, exist_ok=True)
    http_cache.mkdir(parents=True, exist_ok=True)

    runner(
        (
            "uv",
            "export",
            "--project",
            "build_system",
            "--quiet",
            "--format",
            "requirements-txt",
            "--locked",
            "--no-emit-project",
            "--output-file",
            str(requirements),
        ),
        cwd=root,
        check=True,
        text=True,
    )
    command = (
        sys.executable,
        "-m",
        "pip_audit",
        "--vulnerability-service",
        policy.service,
        "--requirement",
        str(requirements),
        "--require-hashes",
        "--disable-pip",
        "--cache-dir",
        str(http_cache),
        "--timeout",
        str(policy.socket_timeout_seconds),
        "--progress-spinner",
        "off",
    )
    for attempt in range(1, policy.attempts + 1):
        result = runner(
            command,
            cwd=root,
            check=False,
            capture_output=True,
            text=True,
        )
        if result.returncode == 0 or not _retryable(result):
            _emit(result)
            return result.returncode
        if attempt == policy.attempts:
            _emit(result)
            return result.returncode
        delay = policy.retry_seconds * attempt
        print(
            f"Python advisory service failed transiently; retrying "
            f"{attempt + 1}/{policy.attempts} in {delay:g}s",
            file=sys.stderr,
        )
        sleep(delay)
    raise AssertionError("bounded Python audit loop did not return")


def main() -> int:
    root = project_root()
    return audit_python_lock(root, for_root(root).audits.python_lock_policy)


def entrypoint() -> int:
    try:
        return main()
    except (OSError, subprocess.SubprocessError, ValueError) as error:
        print(f"Python lock audit failed: {error}", file=sys.stderr)
        return 1
