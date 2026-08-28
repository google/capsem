#!/usr/bin/env python3
"""Compatibility launcher for the gate-owned Cargo audit command."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "build_system" / "builder"))
from bootstrap import mount_builder_package

mount_builder_package(ROOT)
from capsem_builder.gate.tools.audit.cargo_audit import main

if __name__ == "__main__":
    raise SystemExit(main())
