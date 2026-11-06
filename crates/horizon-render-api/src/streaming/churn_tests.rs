//! Deterministic owner-state churn. No GPU, mesher, world, or OS qualification.
use super::*;
use std::sync::{Arc, Weak};

#[test]
fn bounded_payload_ownership_survives_retries_revocation_and_resource_churn() {
    let limits = Limits {
        slots: 8,
        pending: 3,
        bytes: 512,
    };
    let mut stream =
        Stream::<Arc<Vec<u8>>>::new(WorldGeneration(9), ResourceGeneration(1), limits).unwrap();
    let mut issued = Vec::new();
    let mut payloads: Vec<Weak<Vec<u8>>> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut rng = 0x74e1_32a9_u64;
    let mut accepted = 0;
    let mut pressured = 0;
    let mut stale = 0;
    let mut acks = 0;
    let mut removals = 0;
    let mut rebases = 0;
    for step in 0..20_000 {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        // Fixed low word keeps producer choices identical on x86 and x86_64.
        let choice = (rng & 0xffff_ffff) as usize;
        let key = [(rng % 12) as i32, 0, 0];
        match (rng >> 8) % 7 {
            0 | 1 => {
                if let Ok(Some(stamp)) = stream.request(key) {
                    assert!(
                        seen.insert((key, stamp.world.0, stamp.resources.0, stamp.revision)),
                        "ticket reused across removal/rebase"
                    );
                    issued.push((key, stamp));
                    if issued.len() > 64 {
                        issued.remove(0);
                    }
                }
            }
            2 | 3 if !issued.is_empty() => {
                let (key, stamp) = issued[choice % issued.len()];
                let payload = Arc::new(vec![step as u8; (choice % 256) + 1]);
                let weak = Arc::downgrade(&payload);
                // Declared retained input charge, not a process allocator budget.
                let bytes = if step % 19 == 0 {
                    usize::MAX
                } else {
                    std::mem::size_of::<Vec<u8>>()
                        + 2 * std::mem::size_of::<usize>()
                        + payload.capacity()
                };
                let before = stream.usage();
                let waiting = stream.accepts_completion(key, stamp);
                match stream.complete(key, stamp, payload, bytes).unwrap() {
                    Queued::Accepted => {
                        accepted += 1;
                        assert!(weak.upgrade().is_some());
                        payloads.push(weak);
                    }
                    Queued::Backpressure => {
                        pressured += 1;
                        assert!(waiting);
                        assert_eq!(stream.usage(), before);
                        assert!(
                            stream.accepts_completion(key, stamp),
                            "retry consumed the ticket"
                        );
                        assert!(weak.upgrade().is_none(), "rejected completion retained");
                    }
                    Queued::Stale => {
                        stale += 1;
                        assert!(!waiting);
                        assert_eq!(stream.usage(), before);
                        assert!(weak.upgrade().is_none());
                    }
                }
            }
            4 => {
                if let Some((head, stamp, _, _)) = stream.front() {
                    assert!(!stream.uploaded([i32::MAX, 0, 0], stamp));
                    assert!(stream.uploaded(head, stamp));
                    assert!(
                        !stream.uploaded(head, stamp),
                        "duplicate acknowledgement accepted"
                    );
                    acks += 1;
                }
            }
            5 => {
                if let Some(entry) = stream.entries.get(&key) {
                    let stamp = entry.stamp;
                    assert!(stream.revoke(key, stamp));
                    assert!(!stream.accepts_completion(key, stamp));
                    assert_eq!(stream.request(key), Err(Error::Removing));
                }
                if let Some((key, stamp)) = stream.removal() {
                    // Deliberately leave some revocations unacknowledged so
                    // cleanup retry and metadata pressure coexist with uploads.
                    if step % 3 != 0 {
                        assert!(stream.removed(key, stamp));
                        assert!(!stream.removed(key, stamp));
                        removals += 1;
                    }
                }
            }
            _ => {}
        }
        if step % 257 == 256 {
            stream = stream
                .rebased(ResourceGeneration(stream.resources().0 + 1))
                .unwrap();
            assert!(
                payloads.iter().all(|p| p.upgrade().is_none()),
                "rebase retained old payloads"
            );
            payloads.clear();
            rebases += 1;
        }
        let usage = stream.usage();
        assert!(usage.slots <= limits.slots);
        assert!(usage.pending <= limits.pending);
        assert!(usage.bytes <= limits.bytes);
        assert_eq!(
            stream.order.len(),
            stream
                .order
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
        );
        let pending: Vec<_> = stream
            .entries
            .values()
            .filter_map(|entry| {
                if let Phase::Pending(_, bytes) = &entry.phase {
                    Some(*bytes)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(pending.len(), usage.pending);
        assert_eq!(pending.iter().sum::<usize>(), usage.bytes);
        assert_eq!(
            payloads.iter().filter(|p| p.upgrade().is_some()).count(),
            usage.pending
        );
    }
    // Fixed-seed coverage assertions prevent a vacuous bounded/empty-state pass.
    assert!(
        accepted > 100 && pressured > 10 && stale > 100,
        "vacuous churn coverage: accepted={accepted}, pressured={pressured}, stale={stale}"
    );
    assert!(
        acks > 100 && removals > 100 && rebases > 50,
        "vacuous cleanup coverage: acks={acks}, removals={removals}, rebases={rebases}"
    );
    drop(stream);
    assert!(payloads.iter().all(|p| p.upgrade().is_none()));
}
