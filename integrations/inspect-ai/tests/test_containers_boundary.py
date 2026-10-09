"""Container boundary tests (`inspect_capsem/containers` imports and lazy VM mode)."""

from __future__ import annotations

import ast
import subprocess
import sys
from pathlib import Path

from inspect_capsem import CapsemSandboxConfig
from inspect_capsem.containers.spec import ContainerSpec

CONTAINERS_DIR = Path(__file__).resolve().parents[1] / "inspect_capsem" / "containers"
ALLOWED_PREFIX = ("inspect_capsem.containers",)


def _imported_modules(py_file: Path) -> set[str]:
    tree = ast.parse(py_file.read_text(encoding="utf-8"), filename=str(py_file))
    found: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            found.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
            found.add(node.module)
    return found


def test_containers_package_has_no_inspect_or_parent_imports() -> None:
    py_files = sorted(CONTAINERS_DIR.glob("*.py"))
    assert py_files, f"Expected Python modules under {CONTAINERS_DIR}"
    for py_file in py_files:
        for mod in _imported_modules(py_file):
            assert not mod.startswith("inspect_ai"), f"{py_file.name} imported {mod!r}"
            if mod.startswith("inspect_capsem"):
                assert mod.startswith(ALLOWED_PREFIX), f"{py_file.name} imported {mod!r}"


def test_vm_mode_does_not_import_containers_at_top_level() -> None:
    code = (
        "import sys, inspect_capsem; "
        "loaded = [m for m in sys.modules if m.startswith('inspect_capsem.containers')]; "
        "assert not loaded, f'Unexpected container modules loaded on import: {loaded}'"
    )
    proc = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, check=False)
    assert proc.returncode == 0, proc.stderr or proc.stdout


def test_to_container_spec_roundtrip(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.setenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", str(tmp_path))
    compose_path = tmp_path / "sub" / "compose.yaml"
    compose_path.parent.mkdir()
    cfg = CapsemSandboxConfig(
        execution_mode="container",
        image="ubuntu:24.04",
        working_dir="/app",
        compose_file=str(compose_path),
        environment={"K": "V"},
        command=("echo", "hi"),
        volumes=(f"{tmp_path}:/mnt:ro",),
        healthcheck={"test": ["CMD", "true"]},
        mem_limit="512m",
        user="1000:1000",
        allowed_host_paths=(str(tmp_path),),
    )
    spec = cfg.to_container_spec()
    assert isinstance(spec, ContainerSpec) and (spec.image, spec.working_dir) == (
        "ubuntu:24.04",
        "/app",
    )
    assert spec.working_dir_explicit is True
    assert {str(compose_path.parent.resolve()), str(tmp_path.resolve())} <= set(
        spec.allowed_host_paths
    )
    assert (
        CapsemSandboxConfig(execution_mode="container", image="ubuntu:24.04")
        .to_container_spec()
        .working_dir_explicit
        is False
    )
