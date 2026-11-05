//! Bounded owner-issued section tickets and FIFO completions. No mesher/GPU.
//! A ticket must be obtained BEFORE dispatch. Revocation frees metadata only
//! after the owner acknowledges removal; readmission uses a fresh nonce, so no
//! unbounded tombstone map is needed and late work cannot resurrect an unload.
use crate::{
    render_contract::{ResourceGeneration, SectionStamp, WorldGeneration},
    scene::SectionKey,
};
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidLimits,
    Allocation,
    Exhausted,
    Removing,
    InvalidEpoch,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Queued {
    Accepted,
    Stale,
    Backpressure,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub slots: usize,
    pub pending: usize,
    pub bytes: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    pub slots: usize,
    pub pending: usize,
    pub bytes: usize,
    pub removals: usize,
}
enum Phase<T> {
    Waiting,
    Pending(T, usize),
    Applied,
    Removing,
}
struct Entry<T> {
    stamp: SectionStamp,
    phase: Phase<T>,
}
pub struct Stream<T> {
    world: WorldGeneration,
    resources: ResourceGeneration,
    nonce: u64,
    limits: Limits,
    entries: HashMap<SectionKey, Entry<T>>,
    order: VecDeque<SectionKey>,
    bytes: usize,
}
impl<T> Stream<T> {
    pub fn new(
        world: WorldGeneration,
        resources: ResourceGeneration,
        limits: Limits,
    ) -> Result<Self, Error> {
        if limits.slots == 0
            || limits.slots > 65536
            || limits.pending == 0
            || limits.pending > limits.slots
            || limits.bytes == 0
        {
            return Err(Error::InvalidLimits);
        }
        Ok(Self {
            world,
            resources,
            nonce: 0,
            limits,
            entries: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
        })
    }
    pub fn world(&self) -> WorldGeneration {
        self.world
    }
    pub fn resources(&self) -> ResourceGeneration {
        self.resources
    }
    pub fn rebased(&self, resources: ResourceGeneration) -> Result<Self, Error> {
        if resources.0 <= self.resources.0 {
            return Err(Error::InvalidEpoch);
        }
        Self::new(self.world, resources, self.limits)
    }
    pub fn usage(&self) -> Usage {
        Usage {
            slots: self.entries.len(),
            pending: self.order.len(),
            bytes: self.bytes,
            removals: self
                .entries
                .values()
                .filter(|e| matches!(e.phase, Phase::Removing))
                .count(),
        }
    }
    /// None is metadata backpressure. A new request cancels an older pending
    /// completion but never changes an already resident GPU mesh. The nonce is
    /// global within this epoch; even a removed/re-admitted key never reuses it.
    pub fn request(&mut self, key: SectionKey) -> Result<Option<SectionStamp>, Error> {
        if self
            .entries
            .get(&key)
            .is_some_and(|e| matches!(e.phase, Phase::Removing))
        {
            return Err(Error::Removing);
        }
        if !self.entries.contains_key(&key) {
            if self.entries.len() == self.limits.slots {
                return Ok(None);
            }
            self.entries.try_reserve(1).map_err(|_| Error::Allocation)?;
        }
        let nonce = self.nonce.checked_add(1).ok_or(Error::Exhausted)?;
        let stamp = SectionStamp {
            world: self.world,
            resources: self.resources,
            revision: nonce,
        };
        self.cancel_pending(key);
        self.entries.insert(
            key,
            Entry {
                stamp,
                phase: Phase::Waiting,
            },
        );
        self.nonce = nonce;
        Ok(Some(stamp))
    }
    fn cancel_pending(&mut self, key: SectionKey) {
        if let Some(entry) = self.entries.get_mut(&key) {
            if let Phase::Pending(_, bytes) = &entry.phase {
                self.bytes -= bytes;
            }
            entry.phase = Phase::Waiting;
        }
        self.order.retain(|k| *k != key);
    }
    /// `bytes` must conservatively include ALL retained payload storage, not
    /// merely logical vertex length. Rejected payloads are dropped, never held.
    pub fn complete(
        &mut self,
        key: SectionKey,
        stamp: SectionStamp,
        payload: T,
        bytes: usize,
    ) -> Result<Queued, Error> {
        let Some(entry) = self.entries.get(&key) else {
            return Ok(Queued::Stale);
        };
        if entry.stamp != stamp || !matches!(entry.phase, Phase::Waiting) {
            return Ok(Queued::Stale);
        }
        let total = self.bytes.checked_add(bytes);
        if self.order.len() == self.limits.pending || total.is_none_or(|n| n > self.limits.bytes) {
            return Ok(Queued::Backpressure);
        }
        self.order.try_reserve(1).map_err(|_| Error::Allocation)?;
        self.entries.get_mut(&key).unwrap().phase = Phase::Pending(payload, bytes);
        self.order.push_back(key);
        self.bytes = total.unwrap();
        Ok(Queued::Accepted)
    }
    pub fn front(&self) -> Option<(SectionKey, SectionStamp, &T, usize)> {
        let key = *self.order.front()?;
        let entry = self.entries.get(&key)?;
        if let Phase::Pending(payload, bytes) = &entry.phase {
            Some((key, entry.stamp, payload, *bytes))
        } else {
            None
        }
    }
    /// Acknowledge only AFTER successful resident replacement. Failed uploads
    /// retain the same payload/ticket at the head for retry or a newer request.
    pub fn uploaded(&mut self, key: SectionKey, stamp: SectionStamp) -> bool {
        if !self
            .front()
            .is_some_and(|(k, s, _, _)| k == key && s == stamp)
        {
            return false;
        }
        self.cancel_pending(key);
        self.entries.get_mut(&key).unwrap().phase = Phase::Applied;
        true
    }
    /// Revokes the current ticket before touching the backend. Pending removal
    /// blocks draws; repeat the operation or drain removals if cleanup fails.
    pub fn revoke(&mut self, key: SectionKey, stamp: SectionStamp) -> bool {
        if !self.entries.get(&key).is_some_and(|e| e.stamp == stamp) {
            return false;
        }
        self.cancel_pending(key);
        self.entries.get_mut(&key).unwrap().phase = Phase::Removing;
        true
    }
    pub fn removal(&self) -> Option<(SectionKey, SectionStamp)> {
        self.entries
            .iter()
            .find(|(_, e)| matches!(e.phase, Phase::Removing))
            .map(|(&k, e)| (k, e.stamp))
    }
    pub fn removed(&mut self, key: SectionKey, stamp: SectionStamp) -> bool {
        if !self
            .entries
            .get(&key)
            .is_some_and(|e| e.stamp == stamp && matches!(e.phase, Phase::Removing))
        {
            return false;
        }
        self.entries.remove(&key);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stream() -> Stream<u8> {
        Stream::new(
            WorldGeneration(1),
            ResourceGeneration(7),
            Limits {
                slots: 2,
                pending: 1,
                bytes: 4,
            },
        )
        .unwrap()
    }
    #[test]
    fn tickets_reject_foreign_duplicate_unissued_and_superseded_work() {
        let mut state = stream();
        let first = state.request([0; 3]).unwrap().unwrap();
        let newer = state.request([0; 3]).unwrap().unwrap();
        for stale in [
            first,
            SectionStamp {
                world: WorldGeneration(2),
                ..newer
            },
            SectionStamp {
                resources: ResourceGeneration(8),
                ..newer
            },
            SectionStamp {
                revision: 999,
                ..newer
            },
        ] {
            assert_eq!(state.complete([0; 3], stale, 9, 4), Ok(Queued::Stale));
        }
        assert_eq!(state.complete([0; 3], newer, 2, 4), Ok(Queued::Accepted));
        assert_eq!(state.complete([0; 3], newer, 3, 4), Ok(Queued::Stale));
        assert!(!state.uploaded([0; 3], first));
        assert_eq!(state.front().unwrap().2, &2);
        assert!(state.uploaded([0; 3], newer));
        assert_eq!(state.complete([0; 3], newer, 3, 4), Ok(Queued::Stale));
    }
    #[test]
    fn backpressure_is_retryable_and_new_requests_cancel_old_pending_bytes() {
        let mut state = stream();
        let a = state.request([0; 3]).unwrap().unwrap();
        let b = state.request([1; 3]).unwrap().unwrap();
        assert!(state.request([2; 3]).unwrap().is_none());
        assert_eq!(state.complete([0; 3], a, 1, 5), Ok(Queued::Backpressure));
        assert_eq!(state.complete([0; 3], a, 1, 4), Ok(Queued::Accepted));
        assert_eq!(state.complete([1; 3], b, 2, 0), Ok(Queued::Backpressure));
        let c = state.request([0; 3]).unwrap().unwrap();
        assert_eq!(state.usage().bytes, 0);
        assert!(state.front().is_none());
        assert_eq!(state.complete([1; 3], b, 2, 4), Ok(Queued::Accepted));
        assert_eq!(
            state.complete([0; 3], c, 3, usize::MAX),
            Ok(Queued::Backpressure)
        );
        assert!(state.uploaded([1; 3], b));
        assert_eq!(state.complete([0; 3], c, 3, 4), Ok(Queued::Accepted));
    }
    #[test]
    fn revocation_survives_failed_cleanup_without_unbounded_tombstones() {
        let mut state = stream();
        for _ in 0..1000 {
            let ticket = state.request([0; 3]).unwrap().unwrap();
            assert_eq!(state.complete([0; 3], ticket, 1, 4), Ok(Queued::Accepted));
            assert!(state.revoke([0; 3], ticket));
            assert_eq!(
                state.usage(),
                Usage {
                    slots: 1,
                    pending: 0,
                    bytes: 0,
                    removals: 1
                }
            );
            assert_eq!(state.request([0; 3]), Err(Error::Removing));
            assert_eq!(state.complete([0; 3], ticket, 2, 4), Ok(Queued::Stale));
            assert_eq!(state.removal(), Some(([0; 3], ticket)));
            assert!(state.removed([0; 3], ticket));
            let next = state.request([0; 3]).unwrap().unwrap();
            assert!(next.revision > ticket.revision);
            assert_eq!(state.complete([0; 3], ticket, 2, 4), Ok(Queued::Stale));
            assert!(!state.revoke([0; 3], ticket));
            assert!(state.revoke([0; 3], next));
            assert!(state.removed([0; 3], next));
        }
        assert_eq!(state.usage().slots, 0);
    }
    #[test]
    fn rebase_drops_jobs_exhaustion_never_wraps_and_limits_are_checked() {
        let mut state = stream();
        let ticket = state.request([0; 3]).unwrap().unwrap();
        state.complete([0; 3], ticket, 1, 4).unwrap();
        let mut next = state.rebased(ResourceGeneration(8)).unwrap();
        let current = next.request([0; 3]).unwrap().unwrap();
        assert_eq!(next.complete([0; 3], ticket, 1, 4), Ok(Queued::Stale));
        assert_eq!(next.complete([0; 3], current, 1, 4), Ok(Queued::Accepted));
        state.nonce = u64::MAX;
        assert_eq!(state.request([1; 3]), Err(Error::Exhausted));
        assert_eq!(state.usage().bytes, 4);
        for limits in [
            Limits {
                slots: 0,
                pending: 1,
                bytes: 1,
            },
            Limits {
                slots: 1,
                pending: 2,
                bytes: 1,
            },
            Limits {
                slots: 65537,
                pending: 1,
                bytes: 1,
            },
            Limits {
                slots: 1,
                pending: 1,
                bytes: 0,
            },
        ] {
            assert!(Stream::<()>::new(WorldGeneration(0), ResourceGeneration(0), limits).is_err());
        }
    }
}
