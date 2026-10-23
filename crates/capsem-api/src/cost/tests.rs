use super::*;
#[test]
fn coverage_distinguishes_priced_unpriced_and_legacy_calls() {
    let usage = capsem_proto::ledger_counters::ModelUsage {
        calls: 6,
        priced_calls: 2,
        unpriced_calls: 4,
        cost_micro_usd: 4380,
        ..Default::default()
    };
    assert_eq!(
        CostCoverage::from(&usage),
        CostCoverage {
            priced_calls: 2,
            unpriced_calls: 4,
            unknown_calls: 0
        }
    );
    let old = capsem_proto::ledger_counters::ModelUsage {
        calls: 6,
        cost_micro_usd: 4380,
        ..Default::default()
    };
    assert_eq!(
        CostCoverage::from(&old),
        CostCoverage {
            priced_calls: 0,
            unpriced_calls: 0,
            unknown_calls: 6
        }
    );
}
