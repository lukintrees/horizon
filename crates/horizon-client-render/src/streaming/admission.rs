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
