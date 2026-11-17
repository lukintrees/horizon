//! Heap filtering without destroying retired payloads under the caller's lock.
//! Also tested directly with rustc: no assets, renderer, or dependency substitutes.
use std::collections::BinaryHeap;

/// The predicate must be stable for the duration of this call. Current work
/// retains its priority/order. The caller owns when/where retired items drop.
pub(super) fn retire_where<T: Ord>(
    queue: &mut BinaryHeap<T>,
    obsolete: impl Fn(&T) -> bool,
) -> Vec<T> {
    if !queue.iter().any(&obsolete) {
        return Vec::new();
    }
    let (retired, kept): (Vec<_>, Vec<_>) = std::mem::take(queue)
        .into_vec()
        .into_iter()
        .partition(obsolete);
    *queue = BinaryHeap::from(kept);
    retired
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn retained_priority_order_matches_an_unmodified_heap() {
        for size in [0, 1, 32, 1024] {
            let mut queue: BinaryHeap<_> = (0..size).rev().map(|i| (i % 7, i)).collect();
            let expected: BinaryHeap<_> =
                queue.iter().copied().filter(|(_, i)| i % 3 != 0).collect();
            let mut retired = retire_where(&mut queue, |(_, i)| i % 3 == 0);
            retired.sort_unstable();
            assert_eq!(queue.into_sorted_vec(), expected.into_sorted_vec());
            let mut wanted: Vec<_> = (0..size)
                .filter(|i| i % 3 == 0)
                .map(|i| (i % 7, i))
                .collect();
            wanted.sort_unstable();
            assert_eq!(retired, wanted);
        }
    }
    #[test]
    fn no_obsolete_items_keeps_the_heap_allocation() {
        let mut queue: BinaryHeap<_> = (0..64).collect();
        let pointer = queue.as_slice().as_ptr();
        assert!(retire_where(&mut queue, |_| false).is_empty());
        assert_eq!(queue.as_slice().as_ptr(), pointer);
        assert_eq!(queue.pop(), Some(63));
    }
    #[derive(Eq, PartialEq, Ord, PartialOrd)]
    struct Payload {
        key: usize,
        // Only key participates in ordering; the destructor observes the lock.
        probe: Probe,
    }
    struct Probe {
        unlocked_drops: Arc<AtomicUsize>,
        locked_drops: Arc<AtomicUsize>,
        lock: Arc<Mutex<()>>,
    }
    impl PartialEq for Probe {
        fn eq(&self, _: &Self) -> bool {
            true
        }
    }
    impl Eq for Probe {}
    impl PartialOrd for Probe {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for Probe {
        fn cmp(&self, _: &Self) -> std::cmp::Ordering {
            std::cmp::Ordering::Equal
        }
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            if self.lock.try_lock().is_ok() {
                self.unlocked_drops.fetch_add(1, Ordering::SeqCst);
            } else {
                self.locked_drops.fetch_add(1, Ordering::SeqCst);
            }
        }
    }
    #[test]
    fn payloads_are_returned_without_early_destruction() {
        let lock = Arc::new(Mutex::new(()));
        let drops = Arc::new(AtomicUsize::new(0));
        let locked = Arc::new(AtomicUsize::new(0));
        let mut queue = BinaryHeap::new();
        for key in 0..8 {
            queue.push(Payload {
                key,
                probe: Probe {
                    unlocked_drops: drops.clone(),
                    locked_drops: locked.clone(),
                    lock: lock.clone(),
                },
            });
        }
        let guard = lock.lock().unwrap();
        let retired = retire_where(&mut queue, |p| p.key % 2 == 0);
        assert_eq!(
            locked.load(Ordering::SeqCst),
            0,
            "payload destroyed under queue lock"
        );
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert_eq!(queue.len(), 4);
        drop(guard);
        drop(retired);
        assert_eq!(drops.load(Ordering::SeqCst), 4);
        drop(queue);
        assert_eq!(drops.load(Ordering::SeqCst), 8);
        assert_eq!(locked.load(Ordering::SeqCst), 0);
    }
}
