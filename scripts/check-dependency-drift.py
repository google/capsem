#!/usr/bin/env python3
"""Compatibility launcher for the gate-owned dependency drift report."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "build_system" / "builder"))
from bootstrap import mount_builder_package  # noqa: E402

mount_builder_package(ROOT)
from capsem_builder.gate.tools.audit.dependency_drift import main  # noqa: E402

if __name__ == "__main__":
    raise SystemExit(main())
