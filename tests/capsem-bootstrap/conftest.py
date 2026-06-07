"""Shared fixtures for bootstrap and install flow tests.

These tests do NOT require a running VM -- they validate the build toolchain.
"""

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent
ASSETS_DIR = PROJECT_ROOT / "assets"
JUSTFILE = PROJECT_ROOT / "justfile"

pytestmark = pytest.mark.bootstrap
