//! Owner-thread mesh admission/delivery hooks. Workers carry only a CPU stamp.
//! Modern streaming retains its existing unstamped upload list.
use crate::mesher::SectionMesh;
use horizon_render_api::{render_contract::SectionStamp, scene::SectionKey};

pub enum Admission {
    Unstamped,
    Stamped(SectionStamp),
    /// Leave the section dirty; do not allocate a sequence or dispatch work.
    Defer,
}
pub enum Delivery {
    /// Filtered by the streamer's existing sequence/column checks, before sink.
    Obsolete(SectionMesh),
    Upload(SectionMesh),
    Queued,
    /// Keep the same sequence/stamp in the existing bounded in-flight set.
    Retry(SectionMesh),
    /// Reclaim on the worker and ask for a fresh snapshot/ticket if still loaded.
    Discard(SectionMesh),
}
pub trait MeshAdmission {
    /// Native caps current in-flight sections, not merely candidate columns.
    /// Default false preserves the existing modern scheduling behavior.
    fn bounded_section_dispatch(&self) -> bool {
        false
    }
    fn request(&mut self, key: SectionKey) -> Admission;
    fn complete(
        &mut self,
        key: SectionKey,
        stamp: Option<SectionStamp>,
        mesh: SectionMesh,
    ) -> Delivery;
}
/// A column may contain many dirty sections. Charge the native worker room
/// before minting a ticket, and only for an actually admitted dispatch.
pub(super) struct DispatchQuota(Option<usize>);
impl DispatchQuota {
    pub fn new(admission: &impl MeshAdmission, room: usize) -> Self {
        Self(admission.bounded_section_dispatch().then_some(room))
    }
    pub fn request(&mut self, admission: &mut impl MeshAdmission, key: SectionKey) -> Admission {
        if self.0 == Some(0) {
            return Admission::Defer;
        }
        let result = admission.request(key);
        if !matches!(result, Admission::Defer)
            && let Some(left) = self.0.as_mut()
        {
            *left -= 1;
        }
        result
    }
}

pub(super) fn deliver_current(
    admission: &mut impl MeshAdmission,
    key: SectionKey,
    stamp: Option<SectionStamp>,
    mesh: SectionMesh,
    issued_sequence: Option<u64>,
    completed_sequence: u64,
    column_loaded: bool,
) -> Delivery {
    if issued_sequence != Some(completed_sequence) || !column_loaded {
        Delivery::Obsolete(mesh)
    } else {
        admission.complete(key, stamp, mesh)
    }
}

pub(super) struct Modern;
impl MeshAdmission for Modern {
    fn request(&mut self, _: SectionKey) -> Admission {
        Admission::Unstamped
    }
    fn complete(
        &mut self,
        _: SectionKey,
        stamp: Option<SectionStamp>,
        mesh: SectionMesh,
    ) -> Delivery {
        if stamp.is_none() {
            Delivery::Upload(mesh)
        } else {
            Delivery::Discard(mesh)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_render_api::render_contract::{ResourceGeneration, WorldGeneration};
    #[test]
    fn section_quota_never_mints_after_capacity_and_defer_does_not_charge() {
        struct Counting {
            bounded: bool,
            defer: bool,
            calls: usize,
        }
        impl MeshAdmission for Counting {
            fn bounded_section_dispatch(&self) -> bool {
                self.bounded
            }
            fn request(&mut self, _: SectionKey) -> Admission {
                self.calls += 1;
                if self.defer {
                    Admission::Defer
                } else {
                    Admission::Unstamped
                }
            }
            fn complete(
                &mut self,
                _: SectionKey,
                _: Option<SectionStamp>,
                _: SectionMesh,
            ) -> Delivery {
                panic!("dispatch only")
            }
        }
        let mut sink = Counting {
            bounded: true,
            defer: true,
            calls: 0,
        };
        let mut quota = DispatchQuota::new(&sink, 1);
        assert!(matches!(quota.request(&mut sink, [0; 3]), Admission::Defer));
        sink.defer = false;
        assert!(matches!(
            quota.request(&mut sink, [0; 3]),
            Admission::Unstamped
        ));
        assert!(matches!(quota.request(&mut sink, [1; 3]), Admission::Defer));
        assert_eq!(sink.calls, 2, "full capacity must not mint a ticket");
        let mut quota = DispatchQuota::new(&sink, 0);
        assert!(matches!(quota.request(&mut sink, [0; 3]), Admission::Defer));
        assert_eq!(sink.calls, 2);
        // Existing modern scheduling is not changed by this native fix.
        let mut modern = Modern;
        let mut quota = DispatchQuota::new(&modern, 0);
        for _ in 0..64 {
            assert!(matches!(
                quota.request(&mut modern, [0; 3]),
                Admission::Unstamped
            ));
        }
    }
    #[test]
    fn delivery_filters_obsolete_work_before_sink_and_preserves_retry_stamp() {
        struct Sink {
            stamp: Option<SectionStamp>,
            calls: usize,
        }
        impl MeshAdmission for Sink {
            fn request(&mut self, _: SectionKey) -> Admission {
                panic!("never mint at completion")
            }
            fn complete(
                &mut self,
                _: SectionKey,
                stamp: Option<SectionStamp>,
                mesh: SectionMesh,
            ) -> Delivery {
                assert_eq!(stamp, self.stamp);
                self.calls += 1;
                Delivery::Retry(mesh)
            }
        }
        let stamp = SectionStamp {
            world: WorldGeneration(4),
            resources: ResourceGeneration(8),
            revision: 11,
        };
        let mut sink = Sink {
            stamp: Some(stamp),
            calls: 0,
        };
        for (current, loaded) in [(None, true), (Some(6), true), (Some(5), false)] {
            assert!(matches!(
                deliver_current(
                    &mut sink,
                    [0; 3],
                    Some(stamp),
                    SectionMesh::default(),
                    current,
                    5,
                    loaded
                ),
                Delivery::Obsolete(_)
            ));
        }
        assert_eq!(sink.calls, 0);
        let mesh = SectionMesh::default();
        let retry = match deliver_current(&mut sink, [0; 3], Some(stamp), mesh, Some(5), 5, true) {
            Delivery::Retry(mesh) => mesh,
            _ => panic!("expected deferred completion"),
        };
        assert!(matches!(
            deliver_current(&mut sink, [0; 3], Some(stamp), retry, Some(5), 5, true),
            Delivery::Retry(_)
        ));
        assert_eq!(sink.calls, 2);
    }
    #[test]
    fn modern_delivery_never_relabels_native_work_as_an_unstamped_upload() {
        let mut modern = Modern;
        assert!(matches!(modern.request([0; 3]), Admission::Unstamped));
        assert!(matches!(
            modern.complete([0; 3], None, SectionMesh::default()),
            Delivery::Upload(_)
        ));
        let stamp = SectionStamp {
            world: WorldGeneration(1),
            resources: ResourceGeneration(2),
            revision: 3,
        };
        assert!(matches!(
            modern.complete([0; 3], Some(stamp), SectionMesh::default()),
            Delivery::Discard(_)
        ));
    }
}
