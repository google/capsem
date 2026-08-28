"""Citadel guard: Python type debt must fail in the fast source phase.

The relaxed Ty pass ignores each recorded diagnostic family so known debt does
not block development. That makes the exact inventory a separate obligation:
without this fast guard, a count can grow locally and first fail the hosted
release-contract phase even though ``just fast-test`` was green.
"""

from __future__ import annotations

import re
import subprocess
from collections import Counter
from pathlib import Path

from capsem_builder.gate import config as gate_config
from capsem_builder.gate.sourcechecks import ty_inventory_argv

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CONFIG = gate_config.load(PROJECT_ROOT)


def test_the_type_ratchet_records_every_diagnostic_count_exactly() -> None:
    """Debt may shrink deliberately, but it may never grow invisibly."""
    result = subprocess.run(
        ty_inventory_argv(CONFIG, CONFIG.lint.relaxed_roots),
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    counts = Counter(re.findall(r"(?:error|warning)\[([a-z][a-z0-9-]+)\]", result.stdout))

    assert counts == Counter(CONFIG.lint.ty_ratchet), (
        "Ty debt changed. Fix new diagnostics; for reductions, lower the exact "
        "count or remove the family from config/gate.toml.\n"
        f"expected: {dict(CONFIG.lint.ty_ratchet)}\nactual: {dict(sorted(counts.items()))}"
    )
