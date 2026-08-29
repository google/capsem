#!/usr/bin/env python3
"""Compatibility launcher for the gate-owned pnpm audit command."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "build_system" / "builder"))
from bootstrap import mount_builder_package  # noqa: E402

mount_builder_package(ROOT)
from capsem_builder.gate.tools.audit.pnpm_bulk import entrypoint  # noqa: E402

if __name__ == "__main__":
    raise SystemExit(entrypoint())
