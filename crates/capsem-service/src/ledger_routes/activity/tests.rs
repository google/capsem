use super::*;

#[test]
fn cost_projections_preserve_known_subtotals_and_legacy_unknowns() {
    let mut counters = LedgerCounters::default();
    counters.model.total.calls = 6;
    counters.model.total.priced_calls = 2;
    counters.model.total.unpriced_calls = 4;
    counters.model.total.cost_micro_usd = 4380;
    counters
        .model
        .by_model
        .entry("google".into())
        .or_default()
        .insert("mixed".into(), counters.model.total.clone());
    let info = ai_info(&counters);
    assert_eq!(info.total_estimated_cost_usd, 0.00438);
    assert_eq!(info.cost_coverage.as_ref().unwrap().unpriced_calls, 4);
    assert_eq!(info.models[0].cost_coverage, info.cost_coverage);
    assert_eq!(stats_summary(&counters).cost_coverage, info.cost_coverage);
    counters.model.total.priced_calls = 0;
    counters.model.total.unpriced_calls = 0;
    assert_eq!(ai_info(&counters).cost_coverage.unwrap().unknown_calls, 6);
}
