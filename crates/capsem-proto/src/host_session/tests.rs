use super::*;

#[test]
fn an_empty_detail_is_one_byte() {
    assert_eq!(HostSessionDetail::default().encode().unwrap(), vec![0x80]);
}

#[test]
fn a_stop_detail_roundtrips_with_its_counters() {
    let mut counters = LedgerCounters::default();
    counters.net.total = 7;
    counters.model.total.cost_micro_usd = 1_500;
    let detail = HostSessionDetail {
        persistent: true,
        ram_bytes: 4 << 30,
        forked_from: Some("parent".into()),
        status: Some("stopped".into()),
        counters: Some(counters),
        ..HostSessionDetail::default()
    };
    assert_eq!(HostSessionDetail::decode(&detail.encode().unwrap()).unwrap(), detail);
}
