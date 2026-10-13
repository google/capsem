use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Barrier;
use std::time::Duration;

use super::*;

#[test]
fn identity_cache_counts_hits_refreshes_and_invalidations() {
    let cache = IdentityCache::new("asset_manifest");

    assert!(cache.get(&"v1").is_none());
    let first = cache.publish("v1", String::from("first"));
    let hit = cache.get(&"v1").expect("published value");
    assert!(Arc::ptr_eq(&first, &hit));

    let second = cache.publish("v2", String::from("second"));
    assert_eq!(&*second, "second");
    assert!(cache.get(&"v1").is_none());
    cache.invalidate();

    assert_eq!(
        cache.stats(),
        CacheStats {
            hits: 1,
            misses: 2,
            insertions: 1,
            refreshes: 1,
            invalidations: 1,
            evictions: 0,
            load_errors: 0,
            entries: 0,
        }
    );
}

#[test]
fn keyed_identity_cache_bounds_entries_and_refreshes_by_identity() {
    let cache = KeyedIdentityCache::new("persistent_resume", 16);

    assert!(cache.get(&"box", &"v1").is_none());
    let first = cache.publish("box", "v1", String::from("ready"));
    let hit = cache.get(&"box", &"v1").expect("published value");
    assert!(Arc::ptr_eq(&first, &hit));

    let refreshed = cache.publish("box", "v2", String::from("stopped"));
    assert_eq!(&*refreshed, "stopped");
    assert!(cache.get(&"box", &"v1").is_none());
    assert_eq!(cache.stats().evictions, 0, "refresh is not an eviction");
    assert_eq!(cache.remove(&"box").as_deref(), Some(&String::from("stopped")));

    assert_eq!(
        cache.stats(),
        CacheStats {
            hits: 1,
            misses: 2,
            insertions: 1,
            refreshes: 1,
            invalidations: 1,
            evictions: 0,
            load_errors: 0,
            entries: 0,
        }
    );

    let bounded = KeyedIdentityCache::new("bounded_identity", 2);
    for key in 0..32 {
        bounded.publish(key, key, key * 10);
    }
    assert!(bounded.len() <= 2, "resident identity entries must remain bounded");
    assert!(bounded.stats().evictions > 0);
}

#[test]
fn generation_cache_rejects_a_load_started_before_invalidation() {
    let cache = GenerationCache::new("plugin_policy");
    let stale = cache.lookup();
    assert!(stale.value().is_none());
    cache.invalidate();
    assert!(cache.publish(stale.generation(), "stale").is_none());

    let current = cache.lookup();
    let published = cache
        .publish(current.generation(), "current")
        .expect("current generation publishes");
    assert_eq!(*published, "current");
    assert_eq!(cache.lookup().value().as_deref(), Some(&"current"));
    assert_eq!(cache.stats().invalidations, 1);
    assert_eq!(cache.stats().entries, 1);
}

#[test]
fn bounded_cache_single_flights_loads_and_reports_errors() {
    let cache = BoundedCache::<String, String>::new("responses", 8);
    let mut loads = 0;
    let first = cache
        .get_or_try_insert_with("route", || {
            loads += 1;
            Ok::<_, ()>(String::from("body"))
        })
        .unwrap();
    let second = cache
        .get_or_try_insert_with("route", || {
            loads += 1;
            Ok::<_, ()>(String::from("wrong"))
        })
        .unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(loads, 1);

    let error = cache.get_or_try_insert_with("broken", || Err::<String, _>("load failed"));
    assert_eq!(error.unwrap_err(), "load failed");
    assert_eq!(
        cache.stats(),
        CacheStats {
            hits: 1,
            misses: 2,
            insertions: 1,
            refreshes: 0,
            invalidations: 0,
            evictions: 0,
            load_errors: 1,
            entries: 1,
        }
    );
}

#[test]
fn bounded_cache_computes_one_value_for_concurrent_misses() {
    let cache = Arc::new(BoundedCache::<String, usize>::new("single_flight", 8));
    let start = Arc::new(Barrier::new(8));
    let loads = Arc::new(AtomicUsize::new(0));
    let mut threads = Vec::new();
    for _ in 0..8 {
        let cache = Arc::clone(&cache);
        let start = Arc::clone(&start);
        let loads = Arc::clone(&loads);
        threads.push(std::thread::spawn(move || {
            start.wait();
            cache
                .get_or_try_insert_with("key", || {
                    loads.fetch_add(1, Ordering::Relaxed);
                    std::thread::sleep(Duration::from_millis(10));
                    Ok::<_, ()>(42)
                })
                .unwrap()
        }));
    }
    let values: Vec<_> = threads.into_iter().map(|thread| thread.join().unwrap()).collect();

    assert_eq!(loads.load(Ordering::Relaxed), 1);
    assert!(values.iter().all(|value| **value == 42));
    assert!(values.windows(2).all(|pair| Arc::ptr_eq(&pair[0], &pair[1])));
    assert_eq!(cache.stats().misses, 1);
    assert_eq!(cache.stats().hits, 7);
}

#[test]
fn bounded_cache_enforces_capacity_and_tracks_removal() {
    let cache = BoundedCache::new("bounded", 2);
    for key in 0..32 {
        cache.insert(key, key * 10);
    }
    assert!(cache.len() <= 2, "resident entries must remain bounded");
    assert!(cache.stats().evictions > 0);

    let before = cache.len();
    cache.clear();
    assert!(cache.is_empty());
    assert_eq!(cache.stats().entries, 0);
    assert_eq!(cache.stats().invalidations, before as u64);
}
