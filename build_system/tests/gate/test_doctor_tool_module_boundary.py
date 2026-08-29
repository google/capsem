"""Guard direct package ownership for diagnostic command implementations."""

from __future__ import annotations

import ast
import tomllib
from pathlib import Path

BUILD_SYSTEM_ROOT = Path(__file__).resolve().parents[2]
REPOSITORY_ROOT = BUILD_SYSTEM_ROOT.parent
DOCTOR_TOOL_ROOT = BUILD_SYSTEM_ROOT / "builder" / "gate" / "tools" / "doctor"

LAUNCHERS = {
    "check_session.py": "check_session",
    "doctor_session_test.py": "doctor_session_test",
    "kvm-diagnostic.py": "kvm_diagnostic",
}

HELPERS = {
    "check_session_report",
    "doctor_session_host_verify",
    "doctor_session_verify",
}


def test_doctor_tools_have_one_exact_gate_owned_package() -> None:
    assert {path.stem for path in DOCTOR_TOOL_ROOT.glob("*.py")} == {
        "__init__",
        *HELPERS,
        *LAUNCHERS.values(),
    }
    project = tomllib.loads((BUILD_SYSTEM_ROOT / "pyproject.toml").read_text(encoding="utf-8"))
    assert "capsem_builder.gate.tools.doctor" in project["tool"]["setuptools"]["packages"]


def test_doctor_script_boundaries_are_thin_status_launchers() -> None:
    for name, module in LAUNCHERS.items():
        script_root = (
            REPOSITORY_ROOT / "scripts"
            if name == "kvm-diagnostic.py"
            else REPOSITORY_ROOT / "build_system" / "scripts" / "doctor"
        )
        source = (script_root / name).read_text(encoding="utf-8")
        tree = ast.parse(source)
        assert len(source.splitlines()) <= 21, f"{name} contains reusable behavior"
        imports = {
            alias.name
            for node in tree.body
            if isinstance(node, ast.ImportFrom)
            and node.module == "capsem_builder.gate.tools.doctor"
            for alias in node.names
        }
        assert module in imports
        assert not any(isinstance(node, (ast.FunctionDef, ast.ClassDef)) for node in tree.body)
        exits = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.Raise)
            and isinstance(node.exc, ast.Call)
            and isinstance(node.exc.func, ast.Name)
            and node.exc.func.id == "SystemExit"
        ]
        assert len(exits) == 1, f"{name} does not propagate its diagnostic status"
