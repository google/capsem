"""The product SDK may use capsem; retired build-tool imports may not."""

from __future__ import annotations

import ast
from pathlib import Path

from citadel import test_python_project_boundary as boundary

SDK_RATIONALE = (
    "The capsem Python namespace now belongs to the gateway SDK. An import is "
    "SDK-owned only when its module and imported names exist in tracked SDK "
    "source; capsem.gate and other retired build-tool imports remain invalid."
)


def sdk_modules(root: Path, tracked: list[str]) -> dict[str, set[str]]:
    modules = {}
    prefix = "sdk/python/"
    for name in tracked:
        if not name.startswith(prefix + "capsem/") or not name.endswith(".py"):
            continue
        path = Path(name[len(prefix):]).with_suffix("")
        parts = path.parts[:-1] if path.name == "__init__" else path.parts
        exports = set()
        for node in ast.parse((root / name).read_text()).body:
            if isinstance(node, ast.FunctionDef | ast.AsyncFunctionDef | ast.ClassDef):
                exports.add(node.name)
            elif isinstance(node, ast.ImportFrom | ast.Import):
                exports.update(alias.asname or alias.name for alias in node.names)
            elif isinstance(node, ast.Assign):
                exports.update(target.id for target in node.targets if isinstance(target, ast.Name))
            elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
                exports.add(node.target.id)
        modules[".".join(parts)] = exports
    return modules


def unowned_imports(source: str, modules: dict[str, set[str]]) -> list[str]:
    missing = []
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.Import):
            missing.extend(alias.name for alias in node.names
                           if (alias.name == "capsem" or alias.name.startswith("capsem."))
                           and alias.name not in modules)
        elif isinstance(node, ast.ImportFrom) and node.level == 0:
            module = node.module or ""
            if module == "capsem" or module.startswith("capsem."):
                for alias in node.names:
                    if alias.name not in modules.get(module, set()) and f"{module}.{alias.name}" not in modules:
                        missing.append(f"{module}.{alias.name}")
    return missing


def test_sdk_ownership_does_not_revive_build_tool_imports(tmp_path: Path) -> None:
    files = {
        "sdk/python/capsem/__init__.py": "from .hypervisor import Hypervisor\n",
        "sdk/python/capsem/hypervisor.py": "class Hypervisor: pass\n",
        "sdk/python/capsem/models/__init__.py": "from .status import Status as Status\n",
        "sdk/python/capsem/models/status.py": "class Status: pass\n",
        "consumer.py": "from capsem import Hypervisor\nfrom capsem.models import Status\n",
    }
    for name, source in files.items():
        path = tmp_path / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(source)
    modules = sdk_modules(tmp_path, list(files))
    assert unowned_imports(files["consumer.py"], modules) == [], SDK_RATIONALE
    for source in ("from capsem import gate", "import capsem.gate", "from capsem.gate import main",
                   "from capsem.models import Missing", "from capsem import *"):
        assert unowned_imports(source, modules), SDK_RATIONALE
    count, _digest = boundary._old_import_inventory(tmp_path, list(files))
    assert count == 0, SDK_RATIONALE


def test_untracked_sdk_source_cannot_authorize_an_import(tmp_path: Path) -> None:
    assert unowned_imports("from capsem import Hypervisor", sdk_modules(tmp_path, [])), SDK_RATIONALE
