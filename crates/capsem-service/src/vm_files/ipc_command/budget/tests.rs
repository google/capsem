use super::*;
use capsem_proto::approval::ApprovalRegistration;

fn registered(remaining_ms: u64) -> ExecApprovalProgress {
    ExecApprovalProgress::Registered(ApprovalRegistration {
        ask_id: "0123456789ab".into(),
        event_id: "abcdef012345".into(),
        expires_at_unix_ms: 2_000_000_000_000,
        remaining_ms,
    })
}

fn resolved() -> ExecApprovalProgress {
    ExecApprovalProgress::Resolved {
        ask_id: "0123456789ab".into(),
        event_id: "abcdef012345".into(),
    }
}

#[tokio::test(start_paused = true)]
async fn human_wait_preserves_only_the_remaining_ordinary_execution_budget() {
    let mut budget = ExecBudget::new(Some(30));
    tokio::time::advance(Duration::from_secs(5)).await;
    budget.observe(&registered(120_000)).unwrap();
    tokio::time::advance(Duration::from_secs(31)).await;
    assert!(budget.waiting());
    assert!(
        budget.deadline().unwrap() > Instant::now(),
        "original human wait survives thirty seconds"
    );
    budget.observe(&resolved()).unwrap();
    assert!(!budget.waiting());
    assert_eq!(budget.deadline().unwrap() - Instant::now(), Duration::from_secs(25));
    tokio::time::advance(Duration::from_secs(25)).await;
    assert_eq!(budget.deadline().unwrap(), Instant::now());
    assert!(
        budget.observe(&registered(120_000)).is_err(),
        "a resolved original cannot start another wait"
    );
}

#[tokio::test(start_paused = true)]
async fn invalid_duplicate_or_mismatched_receipts_never_extend_a_deadline() {
    let mut budget = ExecBudget::new(Some(30));
    let ordinary = budget.deadline();
    assert!(budget.observe(&registered(0)).is_err());
    assert!(budget.observe(&registered(120_001)).is_err());
    assert!(budget.observe(&resolved()).is_err());
    assert_eq!(budget.deadline(), ordinary);
    budget.observe(&registered(2_000)).unwrap();
    let original = budget.deadline();
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(budget.observe(&registered(120_000)).is_err());
    assert!(budget
        .observe(&ExecApprovalProgress::Resolved {
            ask_id: "111111111111".into(),
            event_id: "abcdef012345".into()
        })
        .is_err());
    assert_eq!(budget.deadline(), original);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(
        budget.observe(&resolved()).is_err(),
        "expired registration cannot revive execution"
    );
    assert_eq!(budget.deadline(), original);
}

#[tokio::test(start_paused = true)]
async fn absent_or_exhausted_ordinary_budget_cannot_admit_human_wait() {
    assert!(ExecBudget::new(None).observe(&registered(120_000)).is_err());
    let mut budget = ExecBudget::new(Some(1));
    let ordinary = budget.deadline();
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(budget.observe(&registered(120_000)).is_err());
    assert_eq!(budget.deadline(), ordinary);
}
