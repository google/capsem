"""Bounded direct diagnostics inherit the repository cache authority."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

from capsem_builder.cache.config import load_policy

ROOT = Path(__file__).resolve().parents[3]
BOUNDED = ROOT / "build_system/scripts/ci/run-bounded-command.py"
CACHE_POLICY = load_policy(ROOT)


def test_bounded_pytest_leaves_no_cache_beside_source(tmp_path: Path) -> None:
    test_file = tmp_path / "test_probe.py"
    test_file.write_text(
        f"""
import os
import sys
from pathlib import Path


def test_cache_authority():
    authority = Path(os.environ.get({CACHE_POLICY.authority_environment!r}, {str(ROOT)!r}))
    root = authority / "cache"
    assert Path(sys.pycache_prefix).is_relative_to(root)
    assert str(root / "tools/python/pytest") in os.environ["PYTEST_ADDOPTS"]
    assert Path(os.environ["UV_CACHE_DIR"]).is_relative_to(root)
    assert Path(os.environ["RUFF_CACHE_DIR"]) == root / "tools/python/ruff"
    assert Path(os.environ["npm_config_store_dir"]).is_relative_to(root)
    assert Path(os.environ["CARGO_TARGET_DIR"]).is_relative_to(root)
    test_tmp = Path(os.environ["TMPDIR"])
    assert test_tmp.parent.parent == Path({str(CACHE_POLICY.stages["test-temp"].path)!r})
    assert f"--basetemp={{test_tmp / 'pytest'}}" in os.environ["PYTEST_ADDOPTS"]
""".strip(),
        encoding="utf-8",
    )

    result = subprocess.run(
        [
            sys.executable,
            str(BOUNDED),
            "--timeout-seconds",
            "30",
            "--",
            sys.executable,
            "-m",
            "pytest",
            "-q",
            str(test_file),
        ],
        cwd=tmp_path,
        check=False,
        capture_output=True,
        text=True,
        timeout=40,
    )

    assert result.returncode == 0, result.stdout + result.stderr
    assert not (tmp_path / ".pytest_cache").exists()
    assert not (tmp_path / "__pycache__").exists()


def test_bounded_ruff_leaves_no_cache_beside_source(tmp_path: Path) -> None:
    source = tmp_path / "probe.py"
    source.write_text("value = 1\n", encoding="utf-8")

    result = subprocess.run(
        [
            sys.executable,
            str(BOUNDED),
            "--timeout-seconds",
            "30",
            "--",
            "ruff",
            "check",
            "--config",
            str(ROOT / "build_system/pyproject.toml"),
            str(source),
        ],
        cwd=tmp_path,
        check=False,
        capture_output=True,
        text=True,
        timeout=40,
    )

    assert result.returncode == 0, result.stdout + result.stderr
    assert not (tmp_path / ".ruff_cache").exists()
