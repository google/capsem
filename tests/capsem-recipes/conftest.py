"""Shared fixtures for just recipe smoke tests."""

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent

pytestmark = pytest.mark.recipe
