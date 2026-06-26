"""Ironbank guardrails for the single security rail contract."""

from __future__ import annotations

from pathlib import Path

import pytest


pytestmark = pytest.mark.integration

PROJECT_ROOT = Path(__file__).resolve().parents[2]


def _production_text(path: Path) -> str:
    text = path.read_text(errors="ignore")
    cfg_test = text.find("#[cfg(test)]")
    if cfg_test != -1 and "mod tests" in text[cfg_test:]:
        return text[:cfg_test]
    return text


def _source_files(*roots: str):
    for root_name in roots:
        root = PROJECT_ROOT / root_name
        for path in root.rglob("*"):
            if path.is_dir() or path.suffix not in {".rs", ".toml", ".yaml", ".yml"}:
                continue
            rel = path.relative_to(PROJECT_ROOT).as_posix()
            if rel.endswith("/tests.rs") or "/tests/" in rel:
                continue
            yield path


def test_retired_security_rail_symbols_stay_burned() -> None:
    banned_symbols = {
        "LocalMcp" + "DecisionProvider",
        "Mcp" + "Policy",
        "legacy_" + "decision",
        "policy" + "_v2_http_hook",
        "evaluate_model_request_policy",
        "evaluate_model_response_policy",
        "[policy" + ".http",
        "[policy" + ".mcp",
        "[policy" + ".model",
    }
    offenders: list[str] = []
    for path in _source_files("crates", "config"):
        text = _production_text(path)
        for symbol in sorted(banned_symbols):
            if symbol in text:
                offenders.append(f"{path.relative_to(PROJECT_ROOT)} contains {symbol}")

    assert offenders == []


def test_deleted_policy_source_files_stay_deleted() -> None:
    deleted_paths = [
        "crates/capsem-core/src/net/mitm_proxy/policy" + "_v2_model.rs",
        "crates/capsem-core/src/net/mitm_proxy/policy" + "_v2_http_hook.rs",
        "crates/capsem-core/src/net/domain_policy.rs",
        "crates/capsem-network-engine/src/domain_policy.rs",
        "crates/capsem-network-engine/src/http_policy.rs",
        "crates/capsem-network-engine/src/mcp_security.rs",
        "crates/capsem-network-engine/src/model_security.rs",
    ]
    existing = [path for path in deleted_paths if (PROJECT_ROOT / path).exists()]
    assert existing == []


def test_network_mechanics_do_not_make_security_decisions() -> None:
    """Routing/capture settings must not return allow/ask/block decisions."""

    banned_needles = {
        "http-port-not-allowlisted",
        "not in allowlist for",
        "policy" + ".http",
        "policy" + ".mcp",
        "policy" + ".model",
    }
    inspected = [
        PROJECT_ROOT / "crates/capsem-core/src/net/policy.rs",
        PROJECT_ROOT / "crates/capsem-core/src/net/mitm_proxy/mod.rs",
        PROJECT_ROOT / "crates/capsem-core/src/net/dns/server.rs",
    ]
    offenders = []
    for path in inspected:
        text = _production_text(path)
        for needle in sorted(banned_needles):
            if needle in text:
                offenders.append(f"{path.relative_to(PROJECT_ROOT)} contains {needle}")

    assert offenders == []


def test_session_event_writes_stay_behind_dbwriter() -> None:
    allowed_direct_sqlite = {
        "crates/capsem-logger/src/db.rs",
        "crates/capsem-logger/src/reader.rs",
        "crates/capsem-logger/src/schema.rs",
        "crates/capsem-logger/src/writer.rs",
        "crates/capsem-core/src/auto_snapshot.rs",
        "crates/capsem-core/src/session/index.rs",
        "crates/capsem-core/src/session/maintenance.rs",
    }
    allowed_event_inserts = {
        "crates/capsem-logger/src/schema.rs",
        "crates/capsem-logger/src/writer.rs",
    }
    event_tables = {
        "audit_events",
        "dns_events",
        "exec_events",
        "fs_events",
        "model_calls",
        "net_events",
        "profile_mutation_events",
        "security_ask_events",
        "security_decision_events",
        "security_rule_events",
        "substitution_events",
        "tool_calls",
        "tool_responses",
    }
    sqlite_open_needles = (
        "Connection::open(",
        "Connection::open_with_flags(",
        "rusqlite::Connection::open(",
        "rusqlite::Connection::open_with_flags(",
    )
    insert_needles = tuple(
        needle
        for table in event_tables
        for needle in (
            f"INSERT INTO {table}",
            f"INSERT OR IGNORE INTO {table}",
            f"INSERT OR REPLACE INTO {table}",
            f'INSERT INTO "{table}"',
            f'INSERT OR IGNORE INTO "{table}"',
            f'INSERT OR REPLACE INTO "{table}"',
        )
    )

    offenders: list[str] = []
    for crate in (PROJECT_ROOT / "crates").iterdir():
        src = crate / "src"
        if not src.exists():
            continue
        for path in src.rglob("*.rs"):
            rel = path.relative_to(PROJECT_ROOT).as_posix()
            if rel.endswith("/tests.rs") or "/tests/" in rel:
                continue
            text = _production_text(path)
            if rel not in allowed_direct_sqlite:
                for needle in sqlite_open_needles:
                    if needle in text:
                        offenders.append(f"{rel} opens SQLite directly with {needle}")
            if rel not in allowed_event_inserts:
                for needle in insert_needles:
                    if needle in text:
                        offenders.append(f"{rel} inserts event rows directly with {needle}")

    assert offenders == []
