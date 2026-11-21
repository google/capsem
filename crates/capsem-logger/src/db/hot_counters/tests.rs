use super::*;

#[test]
fn errors_and_decreases_do_not_invalidate_the_denial_view() {
    let hot = HotCounters::default();
    let mut denials = hot.subscribe_denials();
    let publish = |version, denied| {
        let mut counters = LedgerCounters::default();
        counters.net.denied = denied;
        hot.publish(Some(version), Ok(Arc::new(counters)));
    };

    publish(1, 10);
    hot.publish(None, Err("read failure".into()));
    publish(2, 10);
    assert!(!denials.has_changed().unwrap());

    hot.publish(None, Err("read failure".into()));
    publish(3, 11);
    assert!(denials.has_changed().unwrap(), "kept the last valid baseline");
    denials.borrow_and_update();

    publish(4, 2);
    assert!(!denials.has_changed().unwrap(), "a counter reset is not a denial");
    publish(5, 3);
    assert!(denials.has_changed().unwrap(), "new denials follow the new baseline");
}
