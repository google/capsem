"""Local mock server fixture helpers for network tests."""

import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
if str(PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(PROJECT_ROOT))

from scripts.mock_server import (  # noqa: E402,F401
    MOCK_SERVER_ADDR,
    MOCK_SERVER_BINARY,
    MOCK_SERVER_LOCK,
    local_fixture_env,
    read_ready_json,
    start_mock_server,
    stop_process,
)
