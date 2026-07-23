use std::{
    sync::{Arc, Barrier},
    thread,
};

use super::ProviderReadiness;

#[test]
fn provider_marks_coalesce_and_a_mark_racing_with_take_is_not_lost() {
    let readiness = ProviderReadiness::default();
    readiness.mark_media();
    readiness.mark_media();
    readiness.mark_worldstate();

    let initial = readiness.take();
    assert!(initial.media());
    assert!(initial.worldstate());
    assert!(!initial.market());
    assert!(readiness.take().is_empty());

    let barrier = Arc::new(Barrier::new(2));
    let worker_readiness = readiness.clone();
    let worker_barrier = Arc::clone(&barrier);
    let marker = thread::spawn(move || {
        worker_barrier.wait();
        worker_readiness.mark_market();
    });

    barrier.wait();
    let first = readiness.take();
    marker.join().unwrap();
    let second = readiness.take();
    assert_ne!(first.market(), second.market());
    assert!(readiness.take().is_empty());
}
