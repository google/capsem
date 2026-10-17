use super::super::*;

fn session(provider: &str, model: &str, calls: u64, cost: u64) -> LedgerCounters {
    let mut counters = LedgerCounters::default();
    counters.net.total = calls;
    counters.model.total.calls = calls;
    counters.model.total.cost_micro_usd = cost;
    let usage = bounded_entry(bounded_entry(&mut counters.model.by_model, provider), model);
    usage.calls = calls;
    usage.cost_micro_usd = cost;
    counters
}

#[test]
fn absorbing_the_default_changes_nothing() {
    let counters = session("anthropic", "claude", 3, 1_250);
    let mut merged = counters.clone();
    merged.absorb(&LedgerCounters::default());
    assert_eq!(merged, counters);
}

#[test]
fn totals_and_attribution_sum_exactly() {
    let mut merged = session("anthropic", "claude", 3, 1_250);
    merged.absorb(&session("anthropic", "claude", 2, 750));
    merged.absorb(&session("openai", "gpt", 1, 100));
    assert_eq!(merged.net.total, 6);
    assert_eq!(merged.model.total.calls, 6);
    assert_eq!(merged.model.total.cost_micro_usd, 2_100);
    assert_eq!(merged.model.by_model["anthropic"]["claude"].calls, 5);
    assert_eq!(merged.model.by_model["openai"]["gpt"].cost_micro_usd, 100);
}

#[test]
fn merging_is_order_independent() {
    let (a, b, c) = (
        session("anthropic", "claude", 3, 1_250),
        session("openai", "gpt", 1, 100),
        session("anthropic", "haiku", 7, 9),
    );
    let mut left = a.clone();
    left.absorb(&b);
    left.absorb(&c);
    let mut right = c.clone();
    right.absorb(&a);
    right.absorb(&b);
    assert_eq!(left, right);
}

#[test]
fn a_merge_keeps_every_map_inside_its_bound() {
    let mut merged = LedgerCounters::default();
    for index in 0..(MAX_KEYS_PER_MAP + 10) {
        let mut one = LedgerCounters::default();
        bounded_entry(&mut one.tools.by_tool, &format!("tool-{index}")).calls = 1;
        one.tools.calls = 1;
        merged.absorb(&one);
    }
    assert_eq!(merged.tools.by_tool.len(), MAX_KEYS_PER_MAP + 1);
    assert_eq!(
        merged.tools.by_tool[OVERFLOW_KEY].calls, 10,
        "totals stay exact past the bound"
    );
    let total: u64 = merged.tools.by_tool.values().map(|usage| usage.calls).sum();
    assert_eq!(total, merged.tools.calls);
}

#[test]
fn ranges_and_latest_values_keep_their_meaning() {
    let mut first = LedgerCounters::default();
    first.audit.by_exe.insert(
        "/bin/sh".into(),
        ProcessUsage {
            count: 1,
            first_seen: "2026-10-03T10:00:00.000Z".into(),
            last_seen: "2026-10-03T10:00:00.000Z".into(),
        },
    );
    first.plugins.insert(
        "p".into(),
        PluginCounters {
            executions: 1,
            max_duration_us: 50,
            ..PluginCounters::default()
        },
    );
    let mut second = LedgerCounters::default();
    second.audit.by_exe.insert(
        "/bin/sh".into(),
        ProcessUsage {
            count: 2,
            first_seen: "2026-10-02T09:00:00.000Z".into(),
            last_seen: "2026-10-04T11:00:00.000Z".into(),
        },
    );
    second.plugins.insert(
        "p".into(),
        PluginCounters {
            executions: 2,
            max_duration_us: 20,
            ..PluginCounters::default()
        },
    );

    first.absorb(&second);

    let shell = &first.audit.by_exe["/bin/sh"];
    assert_eq!(shell.count, 3);
    assert_eq!(shell.first_seen, "2026-10-02T09:00:00.000Z");
    assert_eq!(shell.last_seen, "2026-10-04T11:00:00.000Z");
    assert_eq!(first.plugins["p"].executions, 3);
    assert_eq!(first.plugins["p"].max_duration_us, 50);
}

#[test]
fn open_asks_stay_bounded_with_the_excess_counted() {
    let mut merged = LedgerCounters::default();
    for chunk in 0..2 {
        let mut one = LedgerCounters::default();
        for index in 0..MAX_OPEN_ASKS {
            one.security.open_asks.insert(format!("ask-{chunk}-{index}"));
        }
        merged.absorb(&one);
    }
    assert_eq!(merged.security.open_asks.len(), MAX_OPEN_ASKS);
    assert_eq!(merged.security.open_asks_overflow, MAX_OPEN_ASKS as u64);
}
