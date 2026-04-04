"""Shared fixtures for codesign strict tests.

These tests FAIL (not skip) when binaries are unsigned on macOS.
"""

import os
import subprocess

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent
TARGET_DIR = PROJECT_ROOT / "target" / "debug"

IS_MACOS = os.uname().sysname == "Darwin"

SIGNED_BINARIES = {
    "capsem-process": TARGET_DIR / "capsem-process",
    "capsem-service": TARGET_DIR / "capsem-service",
    "capsem": TARGET_DIR / "capsem",
    "capsem-mcp": TARGET_DIR / "capsem-mcp",
}

pytestmark = pytest.mark.codesign
