use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

struct Handle {
    finished: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
}

impl Live for Handle {
    fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

fn declared(host_port: u16, target: PublicationTarget) -> (Declared<Handle>, Arc<AtomicBool>, Arc<AtomicBool>) {
    let (finished, dropped) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let handle = Handle {
        finished: Arc::clone(&finished),
        dropped: Arc::clone(&dropped),
    };
    (
        Declared {
            id: host_port.to_string(),
            host_port: Some(host_port),
            guest_port: 6379,
            target,
            access: capsem_proto::PublicationAccess::LoopbackTcp,
            handle,
        },
        finished,
        dropped,
    )
}

#[test]
fn listing_is_the_live_publications_and_finished_ones_are_released() {
    let registry = Registry::default();
    let (first, _, _) = declared(16379, PublicationTarget::Container);
    let (second, finished, dropped) = declared(18080, PublicationTarget::Vm);
    registry.insert(first);
    registry.insert(second);
    let view = |d: &Declared<Handle>| (d.host_port, d.target);
    assert_eq!(
        registry.list(view),
        vec![
            (Some(16379), PublicationTarget::Container),
            (Some(18080), PublicationTarget::Vm)
        ]
    );
    finished.store(true, Ordering::SeqCst);
    assert_eq!(registry.list(view), vec![(Some(16379), PublicationTarget::Container)]);
    assert!(
        dropped.load(Ordering::SeqCst),
        "a finished publication must not be retained"
    );
}

#[test]
fn removing_a_publication_hands_back_the_handle_that_closes_it() {
    let registry = Registry::default();
    let (entry, _, dropped) = declared(16379, PublicationTarget::Container);
    registry.insert(entry);
    let removed = registry.remove("16379").expect("declared publication");
    assert!(!dropped.load(Ordering::SeqCst));
    drop(removed);
    assert!(dropped.load(Ordering::SeqCst));
    assert!(registry.remove("16379").is_none());
    assert!(registry.list(|d| d.host_port).is_empty());
}
