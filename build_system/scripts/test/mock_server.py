"""Compatibility adapter for the shared test mock-server helper."""

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from build_system.tests.helpers.mock_server import *  # noqa: E402,F403
