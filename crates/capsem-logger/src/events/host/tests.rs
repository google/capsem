use super::*;

fn event(kind: HostEventKind, session: Option<&str>) -> HostEvent {
    HostEvent {
        timestamp_unix_ms: 1_791_000_000_000,
        kind,
        session_id: session.map(str::to_string),
        actor: "cli".to_string(),
        detail: vec![0x80],
        trace_id: None,
    }
}

#[test]
fn every_kind_round_trips_through_its_name() {
    for kind in HostEventKind::ALL {
        assert_eq!(HostEventKind::parse_str(kind.as_str()), Some(kind));
    }
    assert_eq!(HostEventKind::parse_str("session_teleported"), None);
}

#[test]
fn the_hash_depends_on_the_previous_link_and_every_field() {
    let base = event(HostEventKind::SessionStarted, Some("s1"));
    let hash = chain_hash(&GENESIS_HASH, &base);
    assert_ne!(chain_hash(&[1; 32], &base), hash, "previous link");
    let mut changed = base.clone();
    changed.timestamp_unix_ms += 1;
    assert_ne!(chain_hash(&GENESIS_HASH, &changed), hash, "timestamp");
    let mut changed = base.clone();
    changed.kind = HostEventKind::SessionStopped;
    assert_ne!(chain_hash(&GENESIS_HASH, &changed), hash, "kind");
    let mut changed = base.clone();
    changed.session_id = None;
    assert_ne!(chain_hash(&GENESIS_HASH, &changed), hash, "absent session");
    let mut changed = base.clone();
    changed.actor = "desktop".into();
    assert_ne!(chain_hash(&GENESIS_HASH, &changed), hash, "actor");
    let mut changed = base.clone();
    changed.detail = vec![];
    assert_ne!(chain_hash(&GENESIS_HASH, &changed), hash, "detail");
    let mut changed = base;
    changed.trace_id = Some(String::new());
    assert_ne!(
        chain_hash(&GENESIS_HASH, &changed),
        hash,
        "empty trace differs from none"
    );
}

#[test]
fn adjacent_fields_cannot_trade_bytes() {
    let mut left = event(HostEventKind::SessionStarted, Some("ab"));
    left.actor = "c".into();
    let mut right = event(HostEventKind::SessionStarted, Some("a"));
    right.actor = "bc".into();
    assert_ne!(chain_hash(&GENESIS_HASH, &left), chain_hash(&GENESIS_HASH, &right));
}

/// The doctor tools recompute this chain in Python
/// (`build_system/builder/gate/tools/doctor/host_ledger.py`); both pin the
/// same digest, so the two cannot drift apart.
#[test]
fn the_chain_hash_matches_the_shared_golden_vector() {
    let event = HostEvent {
        timestamp_unix_ms: 1_791_000_000_000,
        kind: HostEventKind::SessionStopped,
        session_id: Some("golden".into()),
        actor: "service".into(),
        detail: vec![0x81, 0xa1, b'a', 0x05],
        trace_id: None,
    };
    let digest = chain_hash(&GENESIS_HASH, &event)
        .iter()
        .fold(String::new(), |mut hex, byte| {
            use std::fmt::Write;
            let _ = write!(hex, "{byte:02x}");
            hex
        });
    assert_eq!(
        digest,
        "334ad1daa78a4731b89861fb2ad5fc4cee7cc3fa98eb2669542245d40c90a089"
    );
}
