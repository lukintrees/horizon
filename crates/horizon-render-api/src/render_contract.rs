//! C01 lifecycle vocabulary. No graphics API, decoder or native window types.
//! This does not replace the existing draw contract or duplicate Game geometry.

pub const CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Modern,
    LegacyGl,
}

/// Result of one submission, not a promise that a window displayed an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameOutcome {
    Presented,
    /// Rendering completed offscreen; never treat this as a displayed frame.
    Offscreen,
    /// Minimized, occluded, timed out or otherwise not presented; retry later.
    Deferred,
}
impl FrameOutcome {
    pub fn was_presented(self) -> bool {
        self == Self::Presented
    }
}

/// Layout identity changes only when the owner commits a complete resource reload.
/// Producers borrow pixels; the adapter owns uploaded resources. A failed reload
/// must not publish a new generation, even if temporary staging succeeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceGeneration(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WorldGeneration(pub u64);

/// Stamp a streaming job before dispatch. Revision ordering is per section,
/// including unload tombstones: unloading must not permit a late resurrection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionStamp {
    pub world: WorldGeneration,
    pub resources: ResourceGeneration,
    pub revision: u64,
}
impl SectionStamp {
    pub fn accepts(self, incoming: Self) -> bool {
        self.world == incoming.world
            && self.resources == incoming.resources
            && incoming.revision > self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generations_and_strict_revision_protect_unload_tombstones() {
        let current = SectionStamp {
            world: WorldGeneration(2),
            resources: ResourceGeneration(3),
            revision: 8,
        };
        assert!(current.accepts(SectionStamp {
            revision: 9,
            ..current
        }));
        for stale in [
            current,
            SectionStamp {
                revision: 7,
                ..current
            },
            SectionStamp {
                world: WorldGeneration(1),
                revision: 90,
                ..current
            },
            SectionStamp {
                resources: ResourceGeneration(2),
                revision: 90,
                ..current
            },
        ] {
            assert!(!current.accepts(stale));
        }
    }
    #[test]
    fn completion_is_not_presentation() {
        assert!(FrameOutcome::Presented.was_presented());
        assert!(!FrameOutcome::Offscreen.was_presented());
        assert!(!FrameOutcome::Deferred.was_presented());
    }
}

/// Implemented feature families, NOT measured GL limits or an automatic quality
/// profile. Native device limits are validated by the native context/backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    Panorama,
    GpuPoses,
    ShaderPacks,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub backend: Backend,
    pub features: &'static [Feature],
}
impl Capabilities {
    /// Negotiate before generating geometry. Unsupported requests are returned
    /// to the producer, never erased from a completed Frame inside an adapter.
    pub fn require(self, requested: &[Feature]) -> Result<(), Feature> {
        for feature in requested {
            if !self.features.contains(feature) {
                return Err(*feature);
            }
        }
        Ok(())
    }
}
