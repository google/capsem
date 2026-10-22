use super::*;

#[test]
fn every_kind_round_trips_through_its_name() {
    for kind in HostEventKind::ALL {
        assert_eq!(HostEventKind::parse_str(kind.as_str()), Some(kind));
    }
    assert_eq!(HostEventKind::parse_str("session_teleported"), None);
}
