from pathlib import Path


PROJECT_ROOT = Path(__file__).parent.parent


def _text(path):
    return path.read_text(errors="ignore")


def _production_text(path):
    text = _text(path)
    cfg_test = text.find("#[cfg(test)]")
    if cfg_test != -1 and "mod tests" in text[cfg_test:]:
        return text[:cfg_test]
    return text


def test_retired_policy_v2_and_mcp_decision_rails_stay_absent():
    live_roots = [
        PROJECT_ROOT / "crates",
        PROJECT_ROOT / "config",
    ]
    banned_symbols = [
        "LocalMcpDecisionProvider",
        "McpPolicy",
        "legacy_decision",
        "policy_v2_http_hook",
        "evaluate_model_request_policy",
        "evaluate_model_response_policy",
    ]
    offenders = []
    for root in live_roots:
        for path in root.rglob("*"):
            if path.is_dir() or path.suffix not in {".rs", ".toml", ".yaml", ".yml"}:
                continue
            text = _production_text(path)
            for symbol in banned_symbols:
                if symbol in text:
                    offenders.append(f"{path.relative_to(PROJECT_ROOT)} contains {symbol}")

    assert offenders == []


def test_policy_v2_and_domain_policy_source_files_stay_deleted():
    deleted_paths = [
        "crates/capsem-core/src/net/mitm_proxy/policy_v2_model.rs",
        "crates/capsem-core/src/net/mitm_proxy/policy_v2_http_hook.rs",
        "crates/capsem-core/src/net/domain_policy.rs",
        "crates/capsem-network-engine/src/domain_policy.rs",
        "crates/capsem-network-engine/src/http_policy.rs",
        "crates/capsem-network-engine/src/mcp_security.rs",
        "crates/capsem-network-engine/src/model_security.rs",
    ]
    existing = [path for path in deleted_paths if (PROJECT_ROOT / path).exists()]
    assert existing == []


def test_old_policy_authoring_is_not_live_configuration():
    live_config = [
        PROJECT_ROOT / "config",
    ]
    offenders = []
    for root in live_config:
        for path in root.rglob("*"):
            if path.is_dir() or path.suffix not in {".toml", ".yaml", ".yml"}:
                continue
            text = _production_text(path)
            for old_prefix in ("[policy.http", "[policy.mcp", "[policy.model"):
                if old_prefix in text:
                    offenders.append(f"{path.relative_to(PROJECT_ROOT)} contains {old_prefix}")

    assert offenders == []


def test_session_event_writes_stay_behind_dbwriter():
    """The event ledger has one writer: capsem_logger::DbWriter.

    Product code may read session DBs, clone them for snapshots, or run offline
    maintenance. It must not open ad-hoc SQLite write connections or insert
    directly into event tables from protocol/security code.
    """

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
        "mcp_calls",
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

    offenders = []
    for root in (PROJECT_ROOT / "crates").iterdir():
        src = root / "src"
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
