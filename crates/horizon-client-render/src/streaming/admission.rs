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
    fn request(&mut self, key: SectionKey) -> Admission;
    fn complete(
        &mut self,
        key: SectionKey,
        stamp: Option<SectionStamp>,
        mesh: SectionMesh,
    ) -> Delivery;
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
