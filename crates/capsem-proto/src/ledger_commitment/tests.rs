use super::*;

fn grant(role: LedgerClientRole) -> LedgerChannelGrant {
    LedgerChannelGrant::new(LedgerGeneration::new([7; 16]), 11, role).unwrap()
}

#[test]
fn commitment_binds_authority_order_kind_content_and_chain() {
    let commitment =
        LedgerCommitment::new(grant(LedgerClientRole::Proxy), 2, 19, "net_event", [3; 32], [5; 32]).unwrap();
    commitment.validate_grant(grant(LedgerClientRole::Proxy)).unwrap();
    assert_ne!(commitment.commitment_hash(), ZERO_COMMITMENT_HASH);

    let mut altered = commitment.clone();
    altered.event_hash[0] ^= 1;
    assert_eq!(altered.validate(), Err(LedgerCommitmentError::HashMismatch));

    let stale = LedgerChannelGrant::new(LedgerGeneration::new([8; 16]), 11, LedgerClientRole::Proxy).unwrap();
    assert_eq!(
        commitment.validate_grant(stale),
        Err(LedgerCommitmentError::AuthorityMismatch)
    );
}

#[test]
fn commitments_reject_reader_and_unbounded_identity() {
    assert_eq!(
        LedgerCommitment::new(grant(LedgerClientRole::Reader), 1, 1, "net_event", [1; 32], [0; 32]),
        Err(LedgerCommitmentError::NonProducer)
    );
    assert_eq!(
        LedgerCommitment::new(
            grant(LedgerClientRole::VmOwner),
            1,
            1,
            "x".repeat(MAX_COMMITMENT_EVENT_KIND_BYTES + 1),
            [1; 32],
            [0; 32]
        ),
        Err(LedgerCommitmentError::InvalidEventKind)
    );
}

#[test]
fn reservation_batches_share_the_commitment_identity_bound() {
    let reservation = CommitmentReservation::new("dns_event", [9; 32]).unwrap();
    assert_eq!(reservation.event_kind(), "dns_event");
    assert_eq!(reservation.event_hash(), [9; 32]);
    assert_eq!(
        CommitmentReservation::new("x".repeat(MAX_COMMITMENT_EVENT_KIND_BYTES + 1), [0; 32]),
        Err(LedgerCommitmentError::InvalidEventKind)
    );
}
