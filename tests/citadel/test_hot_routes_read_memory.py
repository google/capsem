"""Polled routes are answered from the ledger DB object's memory.

The TUI, the tray and the web app poll the service on a timer, per VM. A
route on that path that readies a ledger handle, runs a query or opens a
ledger costs a reader round trip and a SQLite read on every poll, so the
service's CPU scales with how often clients ask rather than with what the
sessions do. `/info` did exactly that after session totals moved to the
counter snapshot: it readied its handle twice and read the snapshot back
from the file on every request.
"""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SERVICE = ROOT / "crates" / "capsem-service" / "src"

HOT_PATH_RATIONALE = """
A polled route reads the session's counter snapshot from its ledger handle's
memory: `session_db(...)` to find the handle once, then
`DbHandle::ledger_counters()`, which the handle's reader worker keeps current
off the request path. `open_ready_session_db`, `.ready()`, `.query(...)`,
`.query_many(...)`, the response cache (which readies the handle) and any
ledger open are per-request database work and do not belong on a hot path.
A route that genuinely needs rows is fetched on demand; move it off this list
and say which client polls it, if any.
"""

# `file -> functions` that serve a polled route, or compute what one returns.
HOT_FUNCTIONS: dict[str, tuple[str, ...]] = {
    "vm_files.rs": ("handle_info", "handle_vm_status", "handle_stats_summary"),
    "sandbox_info.rs": ("handle_list",),
    "router_runtime.rs": ("handle_service_status",),
    "vm_lifecycle.rs": ("handle_history_counts", "handle_history_processes"),
    "ledger_routes.rs": (
        "handle_security_info",
        "handle_service_security_status",
        "handle_service_detection_status",
    ),
    "ledger_routes/vm_info.rs": ("populate_vm_info", "session_counters"),
    "ledger_routes/activity.rs": ("read_counters", "counters_if_ready"),
    "ledger_routes/security.rs": ("security_stats_for_vm", "security_stats_for_session", "security_stats"),
}

FORBIDDEN = re.compile(
    r"open_ready_session_db|session_response_cache_lookup|query_route_\w+|"
    r"\.ready\(\)|\.query\(|\.query_many\(|\bopen_external_reader\b|\bDbHandle::open\b|"
    r"\bDbReader::open\b|\bConnection::open|\bregister_session_db_handle"
)


def function_body(source: str, name: str) -> str | None:
    """The brace-balanced body of `fn name`, or None when it is not there."""
    match = re.search(rf"\bfn {re.escape(name)}\b", source)
    if match is None:
        return None
    start = source.find("{", match.end())
    depth = 0
    for index in range(start, len(source)):
        if source[index] == "{":
            depth += 1
        elif source[index] == "}":
            depth -= 1
            if depth == 0:
                return source[start : index + 1]
    return None


def violations(source: str, names: tuple[str, ...]) -> tuple[list[str], list[str]]:
    missing, found = [], []
    for name in names:
        body = function_body(source, name)
        if body is None:
            missing.append(name)
            continue
        code = re.sub(r"//[^\n]*", "", body)
        found.extend(f"{name}: {hit}" for hit in sorted(set(FORBIDDEN.findall(code))))
    return missing, found


def test_hot_route_handlers_read_the_ledger_handle_memory_only() -> None:
    failures: list[str] = []
    for relative, names in HOT_FUNCTIONS.items():
        path = SERVICE / relative
        missing, found = violations(path.read_text(), names)
        failures.extend(
            f"{relative}: fn {name} is gone; point HOT_FUNCTIONS at where it moved, or this guard stops "
            "watching it"
            for name in missing
        )
        failures.extend(f"{relative}: {hit}" for hit in found)
    assert not failures, HOT_PATH_RATIONALE + "\n" + "\n".join(failures)


# -- adversarial: the guard must see what it exists to refuse -----------------


def test_database_work_on_a_hot_path_is_caught() -> None:
    for call in [
        "let db = open_ready_session_db(state, vm_id, ledger, db_path).await?;",
        "db.ready().await?;",
        'let rows = db.query("SELECT 1", &[]).await?;',
        "let raw = db.query_many(batch).await?;",
        "let rows = query_route_typed_rows::<Row>(vm_id, a, b, c, &db, SQL, &[]).await?;",
        "let slot = session_response_cache_lookup(state, id, key, ledger, path).await?;",
        "let db = capsem_logger::DbHandle::open_external_reader(&path)?;",
        "let conn = rusqlite::Connection::open(&path)?;",
    ]:
        source = f"async fn handle_poll() {{\n    {call}\n}}\n"
        _, found = violations(source, ("handle_poll",))
        assert found, call


def test_the_memory_path_and_prose_are_allowed() -> None:
    source = """
async fn handle_poll(state: &ServiceState) -> Result<(), AppError> {
    // This used to call open_ready_session_db and db.ready() on every poll.
    let db = session_db(state, vm_id, "info", &db_path).await?;
    let counters = db.ledger_counters().await?;
    let nested = Some(1).map(|value| { value + 1 });
    Ok(())
}
"""
    missing, found = violations(source, ("handle_poll",))
    assert not missing and not found, found


def test_a_body_is_read_to_its_own_closing_brace() -> None:
    source = "fn hot() {\n    if x { y(); }\n}\nfn cold() {\n    db.ready();\n}\n"
    _, found = violations(source, ("hot",))
    assert not found, found
