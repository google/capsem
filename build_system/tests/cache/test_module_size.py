"""Every cache module stays at or below 300 lines.

The cache skill set this ceiling in prose and nothing held it: `inventory.py`
had reached 314 lines when the Cargo stage inventory moved out beside the
units it accounts.
"""

from pathlib import Path

import pytest

PACKAGE = Path(__file__).resolve().parents[2] / "builder" / "cache"
MAX_LINES = 300


@pytest.mark.parametrize("module", sorted(PACKAGE.glob("*.py")), ids=lambda path: path.name)
def test_cache_module_stays_small(module: Path) -> None:
    lines = len(module.read_text().splitlines())
    assert lines <= MAX_LINES, (
        f"{module.name} has {lines} lines; split it by owner before growing past {MAX_LINES}"
    )
