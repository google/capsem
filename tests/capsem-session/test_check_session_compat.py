"""Compatibility coverage for scripts/check_session.py."""

import importlib.util
import sqlite3
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "check_session.py"


def load_check_session_module():
    spec = importlib.util.spec_from_file_location("check_session", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def create_old_core_db(path: Path):
    conn = sqlite3.connect(path)
    conn.executescript(
        """
        CREATE TABLE net_events (
            id INTEGER PRIMARY KEY,
            timestamp TEXT NOT NULL,
            domain TEXT NOT NULL,
            decision TEXT NOT NULL
        );
        CREATE TABLE model_calls (
            id INTEGER PRIMARY KEY,
            timestamp TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT,
            input_tokens INTEGER,
            output_tokens INTEGER,
            request_body_preview TEXT
        );
        CREATE TABLE tool_calls (
            id INTEGER PRIMARY KEY,
            model_call_id INTEGER,
            call_id TEXT,
            tool_name TEXT
        );
        CREATE TABLE tool_responses (
            id INTEGER PRIMARY KEY,
            model_call_id INTEGER,
            call_id TEXT,
            is_error INTEGER
        );
        CREATE TABLE mcp_calls (
            id INTEGER PRIMARY KEY,
            timestamp TEXT NOT NULL,
            server_name TEXT NOT NULL,
            method TEXT NOT NULL,
            tool_name TEXT,
            decision TEXT NOT NULL
        );
        CREATE TABLE fs_events (
            id INTEGER PRIMARY KEY,
            timestamp TEXT NOT NULL,
            action TEXT NOT NULL,
            path TEXT NOT NULL,
            size INTEGER
        );
        """
    )
    conn.close()


def create_current_policy_db(path: Path):
    conn = sqlite3.connect(path)
    conn.executescript(
        """
        CREATE TABLE net_events (
            id INTEGER PRIMARY KEY,
            timestamp TEXT NOT NULL,
            domain TEXT NOT NULL,
            decision TEXT NOT NULL,
            method TEXT,
            path TEXT,
            status_code INTEGER,
            duration_ms INTEGER,
            policy_mode TEXT,
            policy_action TEXT,
            policy_rule TEXT,
            policy_reason TEXT,
            trace_id TEXT
        );
        CREATE TABLE dns_events (
            id INTEGER PRIMARY KEY,
            timestamp TEXT NOT NULL,
            qname TEXT NOT NULL,
            rcode INTEGER NOT NULL,
            decision TEXT NOT NULL,
            matched_rule TEXT,
            policy_mode TEXT,
            policy_action TEXT,
            policy_rule TEXT,
            policy_reason TEXT,
            trace_id TEXT
        );
        CREATE TABLE model_calls (
            id INTEGER PRIMARY KEY,
            timestamp TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT,
            input_tokens INTEGER,
            output_tokens INTEGER,
            stop_reason TEXT,
            estimated_cost_usd REAL,
            duration_ms INTEGER,
            request_body_preview TEXT,
            trace_id TEXT
        );
        CREATE TABLE tool_calls (
            id INTEGER PRIMARY KEY,
            model_call_id INTEGER,
            call_id TEXT,
            tool_name TEXT,
            origin TEXT,
            mcp_call_id INTEGER,
            trace_id TEXT
        );
        CREATE TABLE tool_responses (
            id INTEGER PRIMARY KEY,
            model_call_id INTEGER,
            call_id TEXT,
            is_error INTEGER,
            trace_id TEXT
        );
        CREATE TABLE mcp_calls (
            id INTEGER PRIMARY KEY,
            timestamp TEXT NOT NULL,
            server_name TEXT NOT NULL,
            method TEXT NOT NULL,
            tool_name TEXT,
            decision TEXT NOT NULL,
            duration_ms INTEGER,
            policy_mode TEXT,
            policy_action TEXT,
            policy_rule TEXT,
            policy_reason TEXT,
            trace_id TEXT
        );
        CREATE TABLE exec_events (
            id INTEGER PRIMARY KEY,
            timestamp TEXT NOT NULL,
            exec_id INTEGER NOT NULL,
            command TEXT NOT NULL,
            exit_code INTEGER,
            duration_ms INTEGER,
            source TEXT,
            mcp_call_id INTEGER,
            trace_id TEXT
        );
        CREATE TABLE fs_events (
            id INTEGER PRIMARY KEY,
            timestamp TEXT NOT NULL,
            action TEXT NOT NULL,
            path TEXT NOT NULL,
            size INTEGER,
            trace_id TEXT
        );
        CREATE TABLE snapshot_events (
            id INTEGER PRIMARY KEY,
            timestamp TEXT NOT NULL,
            slot INTEGER NOT NULL,
            origin TEXT NOT NULL,
            name TEXT,
            files_count INTEGER,
            trace_id TEXT
        );
        CREATE TABLE audit_events (
            id INTEGER PRIMARY KEY,
            timestamp TEXT NOT NULL,
            pid INTEGER,
            ppid INTEGER,
            uid INTEGER,
            exe TEXT,
            comm TEXT,
            exit_code INTEGER,
            audit_id TEXT,
            exec_event_id INTEGER,
            trace_id TEXT
        );
        CREATE TABLE session_identity (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            updated_at TEXT NOT NULL,
            vm_id TEXT NOT NULL,
            profile_id TEXT NOT NULL,
            user_id TEXT NOT NULL
        );
        CREATE TABLE security_events (
            id INTEGER PRIMARY KEY,
            event_id TEXT NOT NULL UNIQUE,
            timestamp TEXT NOT NULL,
            timestamp_unix_ms INTEGER NOT NULL,
            event_family TEXT NOT NULL,
            event_type TEXT NOT NULL,
            source_engine TEXT NOT NULL,
            final_action TEXT NOT NULL,
            enforceability TEXT NOT NULL,
            attribution_scope TEXT NOT NULL,
            origin_kind TEXT NOT NULL,
            accounting_owner TEXT,
            trace_id TEXT,
            vm_id TEXT,
            session_id TEXT,
            profile_id TEXT,
            user_id TEXT,
            process_id TEXT,
            turn_id TEXT,
            message_id TEXT,
            tool_call_id TEXT,
            mcp_call_id TEXT,
            redaction_state TEXT NOT NULL,
            label_count INTEGER NOT NULL DEFAULT 0,
            mutation_count INTEGER NOT NULL DEFAULT 0,
            finding_count INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE security_event_steps (
            id INTEGER PRIMARY KEY,
            event_id TEXT NOT NULL,
            step_index INTEGER NOT NULL,
            kind TEXT NOT NULL,
            status TEXT NOT NULL,
            rule_id TEXT,
            pack_id TEXT,
            message TEXT
        );
        CREATE TABLE detection_findings (
            id INTEGER PRIMARY KEY,
            finding_id TEXT NOT NULL UNIQUE,
            event_id TEXT NOT NULL,
            rule_id TEXT NOT NULL,
            pack_id TEXT NOT NULL,
            sigma_id TEXT,
            title TEXT NOT NULL,
            severity TEXT NOT NULL,
            confidence TEXT NOT NULL
        );
        CREATE TABLE detection_finding_tags (
            id INTEGER PRIMARY KEY,
            finding_id TEXT NOT NULL,
            tag_index INTEGER NOT NULL,
            tag TEXT NOT NULL
        );
        CREATE TABLE security_event_links (
            id INTEGER PRIMARY KEY,
            event_id TEXT NOT NULL,
            linked_event_id TEXT NOT NULL,
            link_type TEXT NOT NULL,
            evidence TEXT
        );
        INSERT INTO model_calls (
            id, timestamp, provider, model, input_tokens, output_tokens,
            request_body_preview, trace_id
        ) VALUES (
            1, '2026-05-10T10:00:00Z', 'anthropic', 'claude',
            10, 20, '{}', 'trace_t6'
        );
        INSERT INTO mcp_calls (
            id, timestamp, server_name, method, tool_name, decision,
            duration_ms, policy_mode, policy_action, policy_rule,
            policy_reason, trace_id
        ) VALUES (
            7, '2026-05-10T10:00:01Z', 'builtin', 'tools/call',
            'danger', 'denied', 12, 'v2', 'block',
            'policy.mcp.block_danger', 'test block', 'trace_t6'
        );
        INSERT INTO tool_calls (
            id, model_call_id, call_id, tool_name, origin, mcp_call_id, trace_id
        ) VALUES (
            3, 1, 'call_1', 'danger', 'mcp', 7, 'trace_t6'
        );
        INSERT INTO tool_responses (
            id, model_call_id, call_id, is_error, trace_id
        ) VALUES (
            4, 1, 'call_1', 1, 'trace_t6'
        );
        """
    )
    conn.close()


def test_check_session_accepts_old_core_schema(tmp_path, capsys):
    module = load_check_session_module()
    db_path = tmp_path / "old" / "session.db"
    db_path.parent.mkdir()
    create_old_core_db(db_path)

    module.check_session(db_path, preview_rows=1)

    out = capsys.readouterr().out
    assert "Missing required tables" not in out
    assert "Core tables present; optional/current tables absent" in out


def test_check_session_uses_exact_mcp_correlation(tmp_path, capsys):
    module = load_check_session_module()
    db_path = tmp_path / "current" / "session.db"
    db_path.parent.mkdir()
    create_current_policy_db(db_path)

    module.check_session(db_path, preview_rows=1)

    out = capsys.readouterr().out
    assert "All current-version tables present" in out
    assert "1 correlated with tool_calls via exact mcp_call_id" in out
    assert "policy.mcp.block_danger" in out
