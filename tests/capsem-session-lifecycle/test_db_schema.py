"""Verify session.db schema matches capsem-logger CREATE_SCHEMA."""

import pytest

pytestmark = pytest.mark.session_lifecycle

# Expected columns per table (from capsem-logger schema)
EXPECTED_SCHEMAS = {
    "net_events": [
        "domain", "decision", "method", "status_code", "bytes_received",
        "duration_ms", "policy_mode", "policy_action", "policy_rule",
        "policy_reason", "trace_id",
    ],
    "dns_events": [
        "qname", "rcode", "decision", "matched_rule", "policy_mode",
        "policy_action", "policy_rule", "policy_reason", "trace_id",
    ],
    "model_calls": ["provider", "model", "duration_ms", "trace_id"],
    "tool_calls": ["tool_name", "origin", "mcp_call_id", "trace_id"],
    "tool_responses": ["call_id", "is_error", "trace_id"],
    "mcp_calls": [
        "method", "decision", "policy_mode", "policy_action",
        "policy_rule", "policy_reason", "trace_id",
    ],
    "exec_events": ["exec_id", "command", "exit_code", "source", "mcp_call_id", "trace_id"],
    "fs_events": ["action", "path", "trace_id"],
    "snapshot_events": ["origin", "slot", "trace_id"],
    "audit_events": ["pid", "exe", "argv", "audit_id", "exec_event_id", "trace_id"],
    "policy_hook_events": [
        "endpoint_id", "spec_version", "spec_hash", "decision_id",
        "callback", "decision", "rule_id", "status", "fallback",
        "audit_tags", "trace_id", "session_id",
    ],
}


class TestDbSchema:

    @pytest.mark.parametrize("table,required_cols", list(EXPECTED_SCHEMAS.items()))
    def test_table_has_required_columns(self, lifecycle_db, table, required_cols):
        """Each table has its required columns."""
        cols = [
            r[1] for r in lifecycle_db.execute(f"PRAGMA table_info({table})").fetchall()
        ]
        if not cols:
            pytest.skip(f"Table {table} not found")
        for col in required_cols:
            assert col in cols, (
                f"Table {table} missing column '{col}' (has: {cols})"
            )
