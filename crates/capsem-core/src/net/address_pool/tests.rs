use super::*;

fn small_pool() -> PrivatePool {
    PrivatePool::parse("192.168.7.8/29").unwrap()
}

#[test]
fn allocation_starts_after_the_gateway_and_never_repeats_while_held() {
    let mut allocator = AddressAllocator::new(PrivatePool::DEFAULT);
    let first = allocator.allocate().unwrap();
    let second = allocator.allocate().unwrap();
    assert_eq!(first, Ipv4Addr::new(10, 128, 0, 2));
    assert_eq!(second, Ipv4Addr::new(10, 128, 0, 3));
    assert_eq!(allocator.in_use(), 2);
}

#[test]
fn a_released_address_is_not_the_next_one_handed_out() {
    let mut allocator = AddressAllocator::new(small_pool());
    let first = allocator.allocate().unwrap();
    assert!(allocator.release(first));
    assert!(!allocator.release(first), "releasing twice is a no-op");
    let next = allocator.allocate().unwrap();
    assert_ne!(next, first, "the cursor moves on before wrapping");
}

#[test]
fn exhaustion_is_an_exact_error_and_release_recovers() {
    let mut allocator = AddressAllocator::new(small_pool());
    let mut held = Vec::new();
    for _ in 0..5 {
        held.push(allocator.allocate().unwrap());
    }
    assert_eq!(held.first(), Some(&Ipv4Addr::new(192, 168, 7, 10)));
    assert_eq!(held.last(), Some(&Ipv4Addr::new(192, 168, 7, 14)));
    assert_eq!(
        allocator.allocate(),
        Err(AddressError::Exhausted {
            pool: small_pool(),
            capacity: 5
        })
    );
    assert!(allocator.release(held[2]));
    assert_eq!(
        allocator.allocate().unwrap(),
        held[2],
        "the only free address wraps back around"
    );
}

#[test]
fn reservations_reject_what_they_must() {
    let mut allocator = AddressAllocator::new(small_pool());
    allocator.reserve(Ipv4Addr::new(192, 168, 7, 12)).unwrap();
    assert_eq!(
        allocator.reserve(Ipv4Addr::new(192, 168, 7, 12)),
        Err(AddressError::InUse {
            address: Ipv4Addr::new(192, 168, 7, 12)
        })
    );
    for reserved in [
        Ipv4Addr::new(192, 168, 7, 8),
        Ipv4Addr::new(192, 168, 7, 9),
        Ipv4Addr::new(192, 168, 7, 15),
    ] {
        assert_eq!(
            allocator.reserve(reserved),
            Err(AddressError::Reserved { address: reserved })
        );
    }
    assert_eq!(
        allocator.reserve(Ipv4Addr::new(10, 128, 0, 2)),
        Err(AddressError::OutsidePool {
            address: Ipv4Addr::new(10, 128, 0, 2),
            pool: small_pool()
        })
    );
    // A reservation is skipped by allocation like any held address.
    let handed: Vec<_> = (0..4).map(|_| allocator.allocate().unwrap()).collect();
    assert!(!handed.contains(&Ipv4Addr::new(192, 168, 7, 12)));
}

#[test]
fn racing_allocations_never_share_an_address() {
    let allocator = std::sync::Arc::new(std::sync::Mutex::new(AddressAllocator::new(small_pool())));
    // Every thread is spawned before any is joined, or there is no race.
    let mut handles = Vec::new();
    for _ in 0..5 {
        let allocator = std::sync::Arc::clone(&allocator);
        handles.push(std::thread::spawn(move || {
            allocator.lock().unwrap().allocate().unwrap()
        }));
    }
    let mut handed: Vec<Ipv4Addr> = handles.into_iter().map(|handle| handle.join().unwrap()).collect();
    handed.sort();
    handed.dedup();
    assert_eq!(handed.len(), 5, "five racing callers, five distinct addresses");
    assert!(allocator.lock().unwrap().allocate().is_err(), "and nothing left over");
}
