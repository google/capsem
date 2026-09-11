use super::*;

#[test]
fn create_tables_includes_dns_events() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    conn.execute(
        "INSERT INTO dns_events (
                timestamp, qname, qtype, qclass, rcode, decision,
                policy_mode, policy_action, policy_rule, policy_reason
             )
             VALUES (
                '2026-01-01T00:00:00Z', 'anthropic.com', 1, 1, 0, 'allowed',
                'enforce', 'allow', 'policy.dns.allow_example', 'allowed by dns policy'
             )",
        [],
    )
    .unwrap();
    let (qname, policy_rule): (String, String) = conn
        .query_row(
            "SELECT qname, policy_rule FROM dns_events WHERE decision = 'allowed'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(qname, "anthropic.com");
    assert_eq!(policy_rule, "policy.dns.allow_example");
}

#[test]
fn migrate_dns_events_idempotent() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    // Run migrate twice -- second call must not error.
    migrate(&conn).unwrap();
    migrate(&conn).unwrap();
    // Verify dns_events table exists and accepts a row.
    conn.execute(
        "INSERT INTO dns_events (timestamp, qname, qtype, qclass, rcode, decision, trace_id)
             VALUES ('2026-01-01T00:00:00Z', 'pypi.org', 1, 1, 0, 'allowed', 'tr_abc')",
        [],
    )
    .unwrap();
    let trace: String = conn
        .query_row("SELECT trace_id FROM dns_events WHERE qname = 'pypi.org'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(trace, "tr_abc");
}

#[test]
fn dns_events_has_indexes() {
    let conn = Connection::open_in_memory().unwrap();
    create_tables(&conn).unwrap();
    for idx in [
        "idx_dns_events_timestamp",
        "idx_dns_events_qname",
        "idx_dns_events_trace_id",
        "idx_dns_events_decision",
        "idx_dns_events_policy_rule",
    ] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name = ?1",
                [idx],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "missing index {idx}");
    }
}
