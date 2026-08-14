#!/usr/bin/env python3
"""Run the strict typed runtime-preflight selector from a source checkout."""

from __future__ import annotations

import runpy
import sys
from pathlib import Path

if __name__ == "__main__":
    source = Path(__file__).resolve().parents[1] / "src"
    sys.path.insert(0, str(source))
    runpy.run_module("capsem.runtime_preflight_manifest", run_name="__main__")
