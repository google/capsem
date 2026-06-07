"""Shared fixtures for rootfs artifact validation tests.

No VM needed -- validates build context and Dockerfile consistency.
"""

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent
ARTIFACTS_DIR = PROJECT_ROOT / "guest" / "artifacts"
CONFIG_DIR = PROJECT_ROOT / "config"

pytestmark = pytest.mark.rootfs
