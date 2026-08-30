"""Compatibility adapter for the shared test mock-server helper."""

import sys
from importlib import import_module
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

_helper = import_module("build_system.tests.helpers.mock_server")

MOCK_SERVER_ADDR = _helper.MOCK_SERVER_ADDR
MOCK_SERVER_BINARY = _helper.MOCK_SERVER_BINARY
local_fixture_env = _helper.local_fixture_env
read_ready_json = _helper.read_ready_json
start_mock_server = _helper.start_mock_server
stop_process = _helper.stop_process

__all__ = [
    "MOCK_SERVER_ADDR",
    "MOCK_SERVER_BINARY",
    "local_fixture_env",
    "read_ready_json",
    "start_mock_server",
    "stop_process",
]
