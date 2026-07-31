"""Shared fixtures for recovery and crash-resilience tests."""

from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).parent.parent.parent

pytestmark = pytest.mark.recovery
