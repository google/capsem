"""Gate modules must stay small before expensive qualification starts."""

from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config

ROOT = Path(__file__).resolve().parents[2]
RATIONALE = (
    "A gate module over the configured ceiling is the 2000-line justfile "
    "growing back in Python; split it by responsibility. This guard belongs "
    "in Citadel: a 301-line schema escaped fast feedback and cost a full "
    "qualification attempt before the longer build-system suite caught it."
)


def _assert_module_sizes(root: Path, ceiling: int) -> None:
    modules = sorted(root.rglob("*.py"))
    assert len(modules) > 3, RATIONALE + "\nscanned too few modules to trust this guard"
    sizes = {
        path.relative_to(root).as_posix(): len(path.read_text().splitlines()) for path in modules
    }
    oversized = {name: size for name, size in sizes.items() if size > ceiling}
    assert not oversized, f"{RATIONALE}\nceiling={ceiling}: {oversized}"


def test_no_gate_module_grows_into_the_justfile_it_replaced() -> None:
    _assert_module_sizes(
        ROOT / "build_system/builder/gate", gate_config.load(ROOT).boundary.max_module_lines
    )


def test_the_exact_ceiling_passes_and_one_extra_line_fails(tmp_path: Path) -> None:
    for index in range(4):
        (tmp_path / f"module_{index}.py").write_text("pass\n" * 300)
    _assert_module_sizes(tmp_path, 300)
    (tmp_path / "module_0.py").write_text("pass\n" * 301)
    with pytest.raises(AssertionError, match=r"module_0\.py.*301"):
        _assert_module_sizes(tmp_path, 300)


def test_empty_inventory_cannot_qualify(tmp_path: Path) -> None:
    with pytest.raises(AssertionError, match="too few modules"):
        _assert_module_sizes(tmp_path, 300)
