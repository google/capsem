"""Bounded direct diagnostics inherit the repository cache authority."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

from capsem_builder import gatelaunch
from capsem_builder.cache.config import load_policy

ROOT = Path(__file__).resolve().parents[3]
BOUNDED = ROOT / "build_system/scripts/ci/run-bounded-command.py"
CACHE_POLICY = load_policy(ROOT)


def test_contained_environment_exports_resolved_authority(tmp_path, monkeypatch):
    monkeypatch.delenv(CACHE_POLICY.authority_environment, raising=False)
    monkeypatch.setattr(gatelaunch, "_git_common_checkout", lambda _: tmp_path)
    inherited = gatelaunch.contained_environment(ROOT)
    assert inherited[CACHE_POLICY.authority_environment] == str(tmp_path)
    override = tmp_path / "explicit"
    monkeypatch.setenv(CACHE_POLICY.authority_environment, str(override))
    selected = gatelaunch.contained_environment(ROOT)
    assert selected[CACHE_POLICY.authority_environment] == str(override)
    assert Path(selected["CARGO_TARGET_DIR"]).is_relative_to(override)


def test_cold_compiler_cache_socket_parent_exists_before_launch(tmp_path: Path) -> None:
    source = tmp_path / "checkout"
    (source / "config").mkdir(parents=True)
    for name in ("cache.toml", "gate.toml"):
        (source / "config" / name).write_bytes((ROOT / "config" / name).read_bytes())
    environment = gatelaunch.contained_environment(source)
    socket = Path(environment["SCCACHE_SERVER_UDS"])
    assert socket.parent.is_dir(), (
        "sccache binds its configured Unix socket before initializing its cache; "
        "cold bounded Cargo diagnostics must create the policy-owned parent first"
    )
    assert socket.parent == Path(environment["SCCACHE_DIR"])


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


def test_node_compile_cache_is_shared_not_rewritten_per_run(tmp_path: Path) -> None:
    """Node's compile cache defaulted to TMPDIR, and TMPDIR is a fresh
    directory per gate process: every Node process rewrote ~7,000 cache files
    it would never reuse. Millions of those writes left fseventsd hours behind
    at 100% CPU and 38 GB."""
    import os
    import shutil

    node = shutil.which("node")
    if node is None:
        import pytest

        pytest.skip("node is not installed")
    source = tmp_path / "checkout"
    (source / "config").mkdir(parents=True)
    for name in ("cache.toml", "gate.toml"):
        (source / "config" / name).write_bytes((ROOT / "config" / name).read_bytes())
    environment = gatelaunch.contained_environment(source)
    module = tmp_path / "probe.js"
    module.write_text("module.exports = 1;\n")
    for _ in range(2):
        subprocess.run(
            [node, "-e", f"require('node:module').enableCompileCache(); require({str(module)!r})"],
            env={**os.environ, **environment},
            check=True,
        )
    run_temp = Path(environment["TMPDIR"])
    assert not (run_temp / "node-compile-cache").exists(), "the cache followed the per-run TMPDIR"
    shared = Path(environment["NODE_COMPILE_CACHE"])
    assert shared.is_relative_to(Path(environment[CACHE_POLICY.authority_environment]) / "cache")
    assert any(shared.rglob("*")), "Node wrote no compile cache where it was pointed"


def test_a_bounded_command_hashes_the_checkout_once_and_holds_what_it_selected(
    tmp_path: Path, monkeypatch
) -> None:
    """Selecting the cache generation hashes every Python source in the
    checkout. The wrapper selected it, then held it by selecting it again, so
    every bounded command read the tree twice before its child existed -- the
    difference between starting inside and outside a test's readiness window
    under CI load. Holding what was selected is also the only way the lease can
    name the generation the child was actually given."""
    from capsem_builder.gate.tools.ci import run_bounded_command

    source = tmp_path / "checkout"
    (source / "config").mkdir(parents=True)
    for name in ("cache.toml", "gate.toml"):
        (source / "config" / name).write_bytes((ROOT / "config" / name).read_bytes())
    module = source / "probe.py"
    module.write_text("value = 1\n", encoding="utf-8")
    hashed: list[Path] = []
    key = gatelaunch._source_key

    def counted(root: Path) -> str:
        hashed.append(root)
        selected = key(root)
        module.write_text(f"value = {len(hashed) + 1}\n", encoding="utf-8")
        return selected

    held: list[Path] = []
    monkeypatch.setattr(gatelaunch, "_source_key", counted)
    monkeypatch.setattr(gatelaunch, "_hold_generation", held.append)
    monkeypatch.setenv("CAPSEM_REPOSITORY_ROOT", str(source))

    environment = run_bounded_command._contained_environment()

    assert hashed == [source.resolve()]
    generation = Path(environment[gatelaunch.PYCACHE])
    assert held[0] == generation, "the lease must cover the generation the child was given"
    assert {path.name for path in held[1:]} >= {generation.name}
    assert Path(environment["TMPDIR"]) in held
