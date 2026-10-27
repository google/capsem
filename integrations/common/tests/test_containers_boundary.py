"""Import boundary and lazy container loading tests for `capsem_integrations`."""

from __future__ import annotations

import ast
import asyncio
import subprocess
import sys
from pathlib import Path

import capsem_integrations
import pytest
from capsem_integrations.containers import HostBuildGrant
from capsem_integrations.containers import build_grant as _bg
from capsem_integrations.containers.spec import ContainerSpec

PACKAGE_DIR = Path(__file__).resolve().parents[1] / "capsem_integrations"
FORBIDDEN_PREFIXES = ("inspect_ai", "inspect_capsem", "harbor", "harbor_capsem")


def _imported_modules(py_file: Path) -> set[str]:
    tree = ast.parse(py_file.read_text(encoding="utf-8"), filename=str(py_file))
    found: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            found.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
            found.add(node.module)
    return found


def test_capsem_integrations_has_no_framework_imports() -> None:
    py_files = sorted(PACKAGE_DIR.rglob("*.py"))
    assert py_files, f"Expected Python modules under {PACKAGE_DIR}"
    for py_file in py_files:
        for mod in _imported_modules(py_file):
            assert not mod.startswith(FORBIDDEN_PREFIXES), (
                f"{py_file.relative_to(PACKAGE_DIR)} imported forbidden module {mod!r}"
            )
    assert capsem_integrations.HostBuildGrant is HostBuildGrant
    assert capsem_integrations.default_host_build_policy is _bg.default_host_build_policy
    with pytest.raises(AttributeError):
        _ = capsem_integrations.NoSuchSymbol


def test_importing_capsem_integrations_does_not_load_containers_at_top_level() -> None:
    code = (
        "import sys, capsem_integrations; "
        "loaded = [m for m in sys.modules if m.startswith('capsem_integrations.containers')]; "
        "assert not loaded, f'Unexpected container modules loaded on import: {loaded}'"
    )
    proc = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, check=False)
    assert proc.returncode == 0, proc.stderr or proc.stdout


def test_container_spec_defaults_and_fields() -> None:
    spec = ContainerSpec(
        image="ubuntu:24.04",
        working_dir="/app",
        working_dir_explicit=True,
        environment={"K": "V"},
        command=("echo", "hi"),
    )
    assert (spec.image, spec.working_dir, spec.working_dir_explicit) == (
        "ubuntu:24.04",
        "/app",
        True,
    )
    default_spec = ContainerSpec(image="ubuntu:24.04")
    assert default_spec.working_dir == "/workspace" and default_spec.working_dir_explicit is False


async def test_default_host_build_policy_defaults_overrides_and_concurrency(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    for var in (
        "CAPSEM_INSPECT_HOST_BUILD",
        "CAPSEM_INSPECT_BUILD_NETWORK",
        "CAPSEM_ALLOWED_HOST_PATHS",
        "CAPSEM_INSPECT_ALLOWED_HOST_PATHS",
    ):
        monkeypatch.delenv(var, raising=False)
    assert _bg.is_host_build_enabled() is False
    assert _bg._resolve_build_network(None) == "none"
    assert _bg._resolve_allowed_build_roots((), None) == ((), ())

    dir_a, dir_b, dir_env = tmp_path / "a", tmp_path / "b", tmp_path / "env"
    dir_a.mkdir()
    dir_b.mkdir()
    dir_env.mkdir()

    with _bg.default_host_build_policy((dir_a,), True, allow_internet=True):
        assert _bg.is_host_build_enabled() is True
        assert _bg.is_host_build_enabled(False) is False
        assert _bg._resolve_build_network(None) == "default"
        assert _bg._resolve_allowed_build_roots((), None) == (
            (dir_a.resolve(),),
            (dir_a.resolve(),),
        )
        monkeypatch.setenv("CAPSEM_INSPECT_HOST_BUILD", "0")
        monkeypatch.setenv("CAPSEM_INSPECT_BUILD_NETWORK", "none")
        monkeypatch.setenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", str(dir_env))
        assert _bg.is_host_build_enabled() is False
        assert _bg._resolve_build_network(None) == "none"
        assert _bg._resolve_allowed_build_roots((), None) == (
            (dir_env.resolve(),),
            (dir_env.resolve(),),
        )
        monkeypatch.delenv("CAPSEM_INSPECT_HOST_BUILD")
        monkeypatch.delenv("CAPSEM_INSPECT_BUILD_NETWORK")
        monkeypatch.delenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS")

    assert _bg.is_host_build_enabled() is False
    assert _bg._resolve_build_network(None) == "none"
    assert _bg._resolve_allowed_build_roots((), None) == ((), ())

    a_ready, b_ready = asyncio.Event(), asyncio.Event()

    async def _worker_a() -> tuple[bool, str, tuple[Path, ...]]:
        with _bg.default_host_build_policy((dir_a,), True, allow_internet=True):
            a_ready.set()
            await b_ready.wait()
            return (
                _bg.is_host_build_enabled(),
                _bg._resolve_build_network(None),
                _bg._resolve_allowed_build_roots((), None)[0],
            )

    async def _worker_b() -> tuple[bool, str, tuple[Path, ...]]:
        await a_ready.wait()
        assert _bg.is_host_build_enabled() is False
        with _bg.default_host_build_policy((dir_b,), False, allow_internet=False):
            b_ready.set()
            await asyncio.sleep(0.01)
            return (
                _bg.is_host_build_enabled(),
                _bg._resolve_build_network(None),
                _bg._resolve_allowed_build_roots((), None)[0],
            )

    res_a, res_b = await asyncio.gather(_worker_a(), _worker_b())
    assert res_a == (True, "default", (dir_a.resolve(),))
    assert res_b == (False, "none", (dir_b.resolve(),))
    assert _bg.is_host_build_enabled() is False
