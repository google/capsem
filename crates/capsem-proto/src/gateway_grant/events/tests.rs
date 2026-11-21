use super::*;

#[test]
fn approval_frames_round_trip_and_refuse_unknown_kinds_and_nil_sessions() {
    let event = SessionEvent::ApprovalPending([0x5a; 16]);
    let mut frame = event.encode();
    assert_eq!(SessionEvent::decode(frame), Some(event));
    frame[0] = 0;
    assert_eq!(SessionEvent::decode(frame), None);
    assert_eq!(
        SessionEvent::decode(SessionEvent::ApprovalPending([0; 16]).encode()),
        None
    );
}
