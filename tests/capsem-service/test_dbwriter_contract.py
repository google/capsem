"""Service-side DbWriter contract: one writer rail per ledger."""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def test_dbwriter_source_boundaries_are_single_rail() -> None:
    service_main = (ROOT / "crates/capsem-service/src/main.rs").read_text()
    # The route module and its submodules, not their test files: a move into a
    # submodule must not read as the rail disappearing.
    routes = ROOT / "crates/capsem-service/src/vm_files"
    service_routes = "\n".join(
        path.read_text()
        for path in [routes.with_suffix(".rs"), *sorted(routes.rglob("*.rs"))]
        if "tests" not in path.relative_to(routes.parent).parts and path.name != "tests.rs"
    )
    service_prod = (
        service_main.split("\n#[cfg(test)]\nmod tests;", 1)[0] + service_routes
    )
    process_main = (ROOT / "crates/capsem-process/src/main.rs").read_text()
    process_prod = process_main.split("\n#[cfg(test)]\nmod tests", 1)[0]
    process_vsock = (ROOT / "crates/capsem-process/src/vsock.rs").read_text()
    logger_writer = (ROOT / "crates/capsem-logger/src/writer.rs").read_text()

    assert 'DbWriter::open(&resolve_session_dir(&state' not in service_prod
    assert 'DbWriter::open(&session_dir.join("session.db")' not in service_prod
    assert "profile_mutation_db: Arc<capsem_logger::DbHandle>" in service_prod
    assert "DbHandle::open(&db_path)" in service_prod
    assert "DbWriter::open(&state.main_db_path()" not in service_prod
    assert 'session_dir.join("session.db")' in service_prod
    # Workspace snapshots were retired (#228): no status query reaches the process.
    assert "SnapshotStatus" not in service_prod

    assert "capsem_logger::DbWriter::open(" in process_prod
    assert '&session_dir.join("session.db")' in process_prod
    assert "Arc<capsem_logger::DbWriter>" in process_vsock
    assert "rusqlite::Connection" not in process_vsock
    assert "write_many" not in process_vsock

    assert "pub struct DbWriter" in logger_writer
    assert "type WriterSender = mpsc::SyncSender<WriterMessage>;" in logger_writer
    assert "mpsc::sync_channel(capacity.max(1))" in logger_writer
    assert "mpsc::channel()" not in logger_writer
    assert '.name("capsem-db-writer".into())' in logger_writer
