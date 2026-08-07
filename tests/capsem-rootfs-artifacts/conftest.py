"""Shared fixtures for rootfs artifact validation tests.

No VM needed -- validates build context and Dockerfile consistency.
"""

from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).parent.parent.parent
ARTIFACTS_DIR = PROJECT_ROOT / "guest" / "artifacts"
CONFIG_DIR = PROJECT_ROOT / "config"

pytestmark = pytest.mark.rootfs
