#!/usr/bin/env python3
"""Run the installed guest-shell proof."""

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from build_system.tests.helpers.prove_installed_shell import main  # noqa: E402

if __name__ == "__main__":
    raise SystemExit(main())
