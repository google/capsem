"""Contracts for deterministic macOS package target-user resolution."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parent.parent
RESOLVER = ROOT / "scripts" / "pkg-scripts" / "install-user"


def _run_resolver(
    request: Path, *, env: dict[str, str] | None = None
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            "bash",
            "-c",
            'source "$1"; capsem_console_user() { printf root; }; '
            'USER=root SUDO_USER=root capsem_resolve_install_user "$2"',
            "bash",
            str(RESOLVER),
            str(request),
        ],
        capture_output=True,
        text=True,
        timeout=10,
        env=env,
    )


def test_secure_request_resolves_headless_non_root_user(tmp_path: Path) -> None:
    if os.geteuid() == 0:
        pytest.skip("the resolver's production owner is the effective package-script user")
    request = tmp_path / "install-user"
    request.write_text(f"{os.environ['USER']}\n", encoding="utf-8")
    request.chmod(0o600)

    result = _run_resolver(request)

    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == os.environ["USER"]


def test_secure_request_uses_gnu_file_stat_before_filesystem_stat(
    tmp_path: Path,
) -> None:
    if os.geteuid() == 0:
        pytest.skip("the resolver's production owner is the effective package-script user")
    request = tmp_path / "install-user"
    request.write_text(f"{os.environ['USER']}\n", encoding="utf-8")
    request.chmod(0o600)
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    fake_stat = fake_bin / "stat"
    fake_stat.write_text(
        """#!/bin/bash
set -euo pipefail
if [ "$1" = "-f" ]; then
    printf 'GNU filesystem report\\n%s\\n' "${CAPSEM_TEST_STAT_VALUE:-wrong}"
    exit 0
fi
exec python3 - "$2" "$3" <<'PY'
import os
import stat
import sys
value = os.stat(sys.argv[2])
print(value.st_uid if sys.argv[1] == "%u" else oct(stat.S_IMODE(value.st_mode))[2:])
PY
""",
        encoding="utf-8",
    )
    fake_stat.chmod(0o755)
    env = os.environ.copy()
    env["PATH"] = f"{fake_bin}:{env['PATH']}"

    result = _run_resolver(request, env=env)

    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == os.environ["USER"]


def test_insecure_request_is_rejected_instead_of_silently_skipping(tmp_path: Path) -> None:
    if os.geteuid() == 0:
        pytest.skip("the resolver's production owner is the effective package-script user")
    request = tmp_path / "install-user"
    request.write_text(f"{os.environ['USER']}\n", encoding="utf-8")
    request.chmod(0o644)

    result = _run_resolver(request)

    assert result.returncode != 0
    assert "secure install-user request" in result.stderr


def test_missing_headless_user_is_a_hard_failure(tmp_path: Path) -> None:
    request = tmp_path / "missing"

    result = _run_resolver(request)

    assert result.returncode != 0
    assert "could not determine a non-root installing user" in result.stderr
