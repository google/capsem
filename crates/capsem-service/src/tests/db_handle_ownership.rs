//! Source contract: exactly one owner opens the main ledger and exactly one
//! opens per-session external readers.

#[test]
fn service_db_handle_open_is_owned_by_explicit_service_state_owners() {
    // ServiceState's DB-handle methods live in session_db_handles.rs; the
    // main-ledger owner stays in main.rs.
    let source = ["/src/main.rs", "/src/session_db_handles.rs"]
        .map(|file| {
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR")).to_owned() + file)
                .expect("service source must be readable")
        })
        .concat();
    let opens = source.matches("DbHandle::open(").count();
    let external_reader_opens = source.matches("DbHandle::open_external_reader(").count();
    assert_eq!(
        opens, 1,
        "DbHandle::open must live only in the service main-ledger owner. Per-session \
         route ledgers are external readers because capsem-process owns writes. Routes and helpers \
         resolve registered handles and call ready/query/write; they do not create a second \
         DB lifecycle."
    );
    assert_eq!(
        external_reader_opens, 1,
        "DbHandle::open_external_reader must live only in register_session_db_handle for \
         per-session ledgers written by capsem-process."
    );
    assert!(
        source.contains("fn register_session_db_handle(") && source.contains("DbHandle::open_external_reader("),
        "the session-state registration method must own the external DB reader lifecycle"
    );
    assert!(
        source.contains("fn open_profile_mutation_db_handle("),
        "one DbHandle::open owner must be the profile mutation main-ledger method"
    );
    assert!(
        !source.contains("Arc<capsem_logger::DbWriter>"),
        "service state must not own DbWriter directly. See AGENTS.md, skills/dev-testing/SKILL.md \
         Logged-data DB ownership, and skills/dev-rust-patterns/SKILL.md Logger DB boundary: \
         service owns DbHandle references; capsem-logger owns writer channels and storage mechanics."
    );
    assert!(
        !source.contains("DbWriter::open("),
        "service production code must not open DbWriter side paths. Create a DbHandle owner and \
         call db.write(event).await so structured DB logging, future mem/disk ownership, and \
         explicit schema failure semantics stay centralized."
    );
}
