"""Import-boundary tests for `inspect_capsem.containers`."""

from __future__ import annotations

import ast
import pathlib
import subprocess
import sys

import inspect_capsem.containers as pkg

ALLOWED_PREFIX = "inspect_capsem.containers"
FORBIDDEN = (
    "inspect_ai",
    "inspect_capsem.sandbox",
    "inspect_capsem.config",
    "inspect_capsem._tools",
    "inspect_capsem._registry",
    "inspect_capsem._controller",
    "inspect_capsem._compose",
)


def test_containers_never_import_the_plugin() -> None:
    root = pathlib.Path(pkg.__file__).parent
    py_files = sorted(root.glob("*.py"))
    assert py_files
    pkg_parts = ("inspect_capsem", "containers")
    for py in py_files:
        tree = ast.parse(py.read_text(encoding="utf-8"))
        for node in ast.walk(tree):
            names: list[str] = []
            if isinstance(node, ast.Import):
                names = [a.name for a in node.names]
            elif isinstance(node, ast.ImportFrom):
                if node.level == 0:
                    base = node.module or ""
                else:
                    keep = len(pkg_parts) - (node.level - 1)
                    prefix = ".".join(pkg_parts[: max(keep, 0)])
                    base = (
                        f"{prefix}.{node.module}"
                        if (prefix and node.module)
                        else (prefix or node.module or "")
                    )
                names = [f"{base}.{a.name}".strip(".") for a in node.names]
                if base:
                    names.append(base)
            for n in names:
                assert not n.startswith(FORBIDDEN), f"{py.name} imports {n}"
                if n.startswith("inspect_capsem"):
                    assert n.startswith(ALLOWED_PREFIX), f"{py.name} imports {n}"


def test_containers_imports_without_inspect_ai() -> None:
    pkg_dir = pathlib.Path(pkg.__file__).resolve().parent.parent
    code = (
        "import importlib, sys, types; "
        "sys.modules['inspect_ai'] = None; "
        f"stub = types.ModuleType('inspect_capsem'); stub.__path__ = [{str(pkg_dir)!r}]; "
        "sys.modules['inspect_capsem'] = stub; "
        "mod = importlib.import_module('inspect_capsem.containers'); "
        "assert hasattr(mod, 'ContainerSpec'); "
        "assert hasattr(mod, 'start_container_for_init')"
    )
    res = subprocess.run(
        [sys.executable, "-c", code],
        capture_output=True,
        text=True,
        check=False,
    )
    assert res.returncode == 0, f"stderr: {res.stderr}\nstdout: {res.stdout}"
