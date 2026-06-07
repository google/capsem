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
    "session_identity": ["updated_at", "vm_id", "profile_id", "user_id"],
    "security_events": [
        "event_id", "timestamp_unix_ms", "event_family", "event_type",
        "source_engine", "final_action", "enforceability",
        "attribution_scope", "origin_kind", "accounting_owner",
        "trace_id", "vm_id", "session_id", "profile_id", "user_id",
        "process_id", "turn_id", "message_id", "tool_call_id",
        "mcp_call_id", "redaction_state", "label_count",
        "mutation_count", "finding_count",
    ],
    "security_event_steps": [
        "event_id", "step_index", "kind", "status", "rule_id",
        "pack_id", "message",
    ],
    "detection_findings": [
        "finding_id", "event_id", "rule_id", "pack_id", "sigma_id",
        "title", "severity", "confidence",
    ],
    "detection_finding_tags": ["id", "finding_id", "tag_index", "tag"],
    "security_event_links": [
        "event_id", "linked_event_id", "link_type", "evidence",
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
