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
    StaticMenu,
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

/// CPU visibility order; translucent quads within each section must already be
/// sorted by the producer. No backend discovers or silently reorders sections.
#[derive(Clone, Copy, Debug, Default)]
pub struct VisibleSections<'a> {
    pub solid: &'a [crate::scene::SectionKey],
    pub translucent: &'a [crate::scene::SectionKey],
}
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameOptions {
    pub gamma: f32,
    pub fullbright: bool,
}

/// CPU request; encoding/readback completion belongs to the selected adapter.
/// Game retains outstanding requests across device recreation.
pub struct ScreenshotRequest {
    pub path: std::path::PathBuf,
    pub size: Option<[u32; 2]>,
    pub scaled: bool,
}

/// Explicit menu policy, selected by the producer (not an adapter downgrade).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuBackground {
    Panorama,
    /// Gamma-encoded SDR framebuffer RGBA (non-sRGB target), finite in 0..=1.
    Static([f32; 4]),
}
impl MenuBackground {
    pub fn valid(self) -> bool {
        match self {
            Self::Panorama => true,
            Self::Static(color) => color
                .into_iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(&v)),
        }
    }
}

#[cfg(test)]
mod menu_tests {
    use super::*;
    #[test]
    fn explicit_backgrounds_validate_without_clamping_or_downgrading() {
        assert!(MenuBackground::Panorama.valid());
        assert!(MenuBackground::Static([0., 0.5, 1., 1.]).valid());
        for value in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            for index in 0..4 {
                let mut color = [0.; 4];
                color[index] = value;
                assert!(!MenuBackground::Static(color).valid());
            }
        }
    }
}
