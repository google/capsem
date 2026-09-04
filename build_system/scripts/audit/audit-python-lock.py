#!/usr/bin/env python3
"""Thin launcher for the gate-owned Python lock audit."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


def main() -> int:
    """Mount the direct builder source and run its typed audit command."""
    sys.path.insert(0, str(ROOT / "build_system" / "builder"))
    from bootstrap import mount_builder_package

    mount_builder_package(ROOT)
    from capsem_builder.gate.tools.audit.python_lock import entrypoint

    return entrypoint()

if __name__ == "__main__":
    raise SystemExit(main())
