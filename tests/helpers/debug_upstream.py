"""Local debug upstream fixture helpers for network tests."""

from scripts.debug_upstream import (  # noqa: F401
    DEBUG_UPSTREAM_ADDR,
    DEBUG_UPSTREAM_BINARY,
    DEBUG_UPSTREAM_LOCK,
    local_fixture_env,
    read_ready_json,
    start_debug_upstream,
    stop_process,
)
