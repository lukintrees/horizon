//! CPU animated terrain sprites for the experimental GL atlas. This is not yet
//! connected to the game's render loop.
use super::atlas::{Atlas, AtlasError};
use crate::sprites::{FrameBlend, SpriteAnimation};
use horizon_legacy_gl::Region;

struct Sprite {
    animation: SpriteAnimation,
    shown: Option<FrameBlend>,
}

/// Updates the exact same animation clips as the existing renderer, without
/// compute shaders or vertex texture fetch. Pending page regions remain dirty
/// until the owning GL driver reports that their upload succeeded.
pub struct SpriteAnimator {
    sprites: Vec<Sprite>,
    dirty: Vec<Option<Region>>,
}
impl SpriteAnimator {
    /// The caller must initially upload every atlas page before using `dirty`.
    pub fn new(animations: &[SpriteAnimation], atlas: &Atlas) -> Self {
        Self {
            sprites: animations
                .iter()
                .cloned()
                .map(|animation| Sprite {
                    animation,
                    shown: None,
                })
                .collect(),
            dirty: vec![None; atlas.pages().len()],
        }
    }

    /// Update changed sprites. The optional bitset uses the game's animated
    /// layer indices; inactive sprites keep their prior state and catch up when
    /// visible again. Earlier successful updates stay pending if a later one
    /// fails. The atlas must be the same one passed to `new`.
    pub fn advance(
        &mut self,
        atlas: &mut Atlas,
        tick: u64,
        active: Option<&[u64]>,
    ) -> Result<(), AtlasError> {
        if atlas.pages().len() != self.dirty.len() {
            return Err(AtlasError::LayerOutOfRange);
        }
        for sprite in &mut self.sprites {
            let layer = sprite.animation.layer;
            if let Some(active) = active {
                let word = (layer / 64) as usize;
                let bit = 1u64 << (layer % 64);
                if active.get(word).is_none_or(|word| word & bit == 0) {
                    continue;
                }
            }
            let blend = sprite.animation.blend(tick);
            if sprite.shown == Some(blend) {
                continue;
            }
            let a = sprite.animation.pixels(blend.frame);
            let mut mixed = Vec::new();
            let pixels = if blend.millis == 0 {
                a
            } else {
                let b = sprite.animation.pixels(blend.next);
                if a.len() != b.len() {
                    return Err(AtlasError::WrongPixelCount {
                        expected: a.len(),
                        actual: b.len(),
                    });
                }
                mixed
                    .try_reserve_exact(a.len())
                    .map_err(|_| AtlasError::AllocationFailed)?;
                mixed.extend(a.iter().zip(b).map(|(&a, &b)| {
                    ((u32::from(a) * (1000 - blend.millis) + u32::from(b) * blend.millis + 500)
                        / 1000) as u8
                }));
                &mixed
            };
            let (page, region) = atlas.update_region(layer, pixels)?;
            let slot = &mut self.dirty[page];
            *slot = Some(match *slot {
                None => region,
                Some(old) => {
                    let x = old.x.min(region.x);
                    let y = old.y.min(region.y);
                    let right = (old.x + old.width).max(region.x + region.width);
                    let bottom = (old.y + old.height).max(region.y + region.height);
                    Region {
                        x,
                        y,
                        width: right - x,
                        height: bottom - y,
                    }
                }
            });
            sprite.shown = Some(blend);
        }
        Ok(())
    }

    pub fn dirty(&self) -> impl Iterator<Item = (usize, Region)> + '_ {
        self.dirty
            .iter()
            .enumerate()
            .filter_map(|(page, region)| region.map(|region| (page, region)))
    }

    /// Acknowledge only after successful GPU upload, without advancing again
    /// between collecting the region and acknowledging it.
    pub fn uploaded(&mut self, page: usize) -> Result<(), AtlasError> {
        let slot = self
            .dirty
            .get_mut(page)
            .ok_or(AtlasError::LayerOutOfRange)?;
        *slot = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn animation_mixes_frames_and_retains_dirty_pages() {
        let frames = vec![vec![0, 10, 20, 255], vec![100, 110, 120, 255]];
        let animation = crate::sprites::test_frames(
            0,
            frames,
            r#"{"frametime":2,"frames":[0,1],"interpolate":true}"#,
        );
        let mut atlas = Atlas::new(1, 1, 4, &[0, 10, 20, 255]).unwrap();
        let mut animator = SpriteAnimator::new(&[animation], &atlas);
        animator.advance(&mut atlas, 0, Some(&[0])).unwrap();
        assert_eq!(animator.dirty().count(), 0);
        animator.advance(&mut atlas, 1, Some(&[1])).unwrap();
        assert_eq!(&atlas.pages()[0].pixels[0..4], &[50, 60, 70, 255]);
        assert_eq!(
            animator.dirty().collect::<Vec<_>>(),
            vec![(
                0,
                Region {
                    x: 0,
                    y: 0,
                    width: 3,
                    height: 3
                }
            )]
        );
        animator.advance(&mut atlas, 1, Some(&[1])).unwrap();
        assert_eq!(animator.dirty().count(), 1); // failed upload is retried
        animator.uploaded(0).unwrap();
        animator.advance(&mut atlas, 2, Some(&[0])).unwrap();
        assert_eq!(animator.dirty().count(), 0);
        animator.advance(&mut atlas, 2, Some(&[1])).unwrap();
        assert_eq!(&atlas.pages()[0].pixels[0..4], &[100, 110, 120, 255]);
        assert_eq!(animator.dirty().count(), 1);
        assert_eq!(animator.uploaded(1), Err(AtlasError::LayerOutOfRange));
    }
}
