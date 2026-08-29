"""Local mock server fixture helpers for network tests."""

import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
if str(PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(PROJECT_ROOT))

from build_system.tests.helpers.mock_server import (  # noqa: F401
    MOCK_SERVER_ADDR,
    MOCK_SERVER_BINARY,
    local_fixture_env,
    read_ready_json,
    start_mock_server,
    stop_process,
)
