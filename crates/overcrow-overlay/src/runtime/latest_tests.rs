use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::latest_channel;

#[derive(Debug)]
struct CloneCounted {
    clones: Arc<AtomicUsize>,
    value: u32,
}

impl Clone for CloneCounted {
    fn clone(&self) -> Self {
        self.clones.fetch_add(1, Ordering::SeqCst);
        Self {
            clones: Arc::clone(&self.clones),
            value: self.value,
        }
    }
}

#[test]
fn pending_publications_coalesce_without_cloning_the_payload() {
    let clones = Arc::new(AtomicUsize::new(0));
    let payload = |value| CloneCounted {
        clones: Arc::clone(&clones),
        value,
    };
    let (publisher, receiver) = latest_channel(payload(0));

    assert!(publisher.publish(payload(1)));
    assert!(publisher.publish(payload(2)));

    let latest = receiver.take_latest().expect("latest value is ready");
    assert_eq!(latest.revision, 2);
    assert_eq!(latest.value.value, 2);
    for _ in 0..100 {
        assert!(receiver.take_latest().is_none());
    }
    assert_eq!(clones.load(Ordering::SeqCst), 0);
}

#[test]
fn current_and_receiver_reads_clone_only_the_arc() {
    let clones = Arc::new(AtomicUsize::new(0));
    let (publisher, receiver) = latest_channel(CloneCounted {
        clones: Arc::clone(&clones),
        value: 7,
    });

    let current = publisher.current();
    assert_eq!(current.revision, 0);
    assert_eq!(current.value.value, 7);
    assert_eq!(clones.load(Ordering::SeqCst), 0);

    assert!(publisher.publish(CloneCounted {
        clones: Arc::clone(&clones),
        value: 8,
    }));
    let received = receiver.take_latest().expect("published value is ready");
    assert_eq!(received.value.value, 8);
    assert_eq!(clones.load(Ordering::SeqCst), 0);
}

#[test]
fn explicit_derived_update_clones_the_payload_exactly_once() {
    let clones = Arc::new(AtomicUsize::new(0));
    let (publisher, receiver) = latest_channel(CloneCounted {
        clones: Arc::clone(&clones),
        value: 11,
    });

    assert!(publisher.update(|payload| payload.value += 1));

    let latest = receiver.take_latest().expect("derived value is ready");
    assert_eq!(latest.revision, 1);
    assert_eq!(latest.value.value, 12);
    assert_eq!(clones.load(Ordering::SeqCst), 1);
}
