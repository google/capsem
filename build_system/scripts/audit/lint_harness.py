"""Compatibility imports for the gate-owned lint harness."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "build_system" / "builder"))
from bootstrap import mount_builder_package  # noqa: E402

mount_builder_package(ROOT)
from capsem_builder.gate.tools.audit.lint_harness import (  # noqa: E402, F401
    EmptySurface,
    Finding,
    Outcome,
    Sources,
    Tool,
    ToolFailure,
    embedded,
    report,
    run,
    tracked_files,
)
