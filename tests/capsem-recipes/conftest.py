"""Shared fixtures for just recipe smoke tests."""

from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).parent.parent.parent

pytestmark = pytest.mark.recipe
