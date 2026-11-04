//! CPU backing for a virtual HUD atlas. Store independent LZ4 blocks rather than
//! retain a second full-size RGBA image. Page decompression is fixed-size/bounded.
use crate::font::Patch;
use horizon_render_api::paged_gui::Layout;
use std::collections::BTreeMap;

pub(crate) struct Backing {
    layout: Layout,
    pages: Vec<Vec<u8>>,
    bytes: usize,
    limit: usize,
}
impl Backing {
    pub fn new(layout: Layout, rgba: &[u8], limit: usize) -> Result<Self, String> {
        let mut pages = Vec::new();
        let mut bytes = 0usize;
        for page in 0..layout.page_count() {
            let pixels = layout
                .pixels(page, rgba)
                .map_err(|e| format!("HUD page: {e:?}"))?;
            let compressed = lz4_flex::compress(&pixels);
            bytes = bytes
                .checked_add(compressed.len())
                .filter(|&n| n <= limit)
                .ok_or("compressed HUD backing budget exceeded")?;
            pages.push(compressed);
        }
        Ok(Self {
            layout,
            pages,
            bytes,
            limit,
        })
    }
    pub fn layout(&self) -> Layout {
        self.layout
    }
    pub fn load(&self, page: usize) -> Result<Vec<u8>, String> {
        let compressed = self.pages.get(page).ok_or("invalid HUD backing page")?;
        let pixels = lz4_flex::decompress(compressed, self.layout.page_bytes())
            .map_err(|e| e.to_string())?;
        if pixels.len() != self.layout.page_bytes() {
            return Err("invalid HUD backing length".into());
        }
        Ok(pixels)
    }
    /// All patches are validated/staged before committing compressed blocks. The
    /// working budget bounds incoming pixels plus decoded dirty pages; each page
    /// is decoded once even when many glyphs touch it. Patch order is preserved.
    pub fn patch(&mut self, patches: &[Patch], working_limit: usize) -> Result<Vec<usize>, String> {
        let mut incoming = 0usize;
        for p in patches {
            let length = (p.width as usize)
                .checked_mul(p.height as usize)
                .and_then(|n| n.checked_mul(4));
            if length != Some(p.rgba.len()) {
                return Err("invalid HUD patch RGBA length".into());
            }
            self.layout
                .affected([p.x, p.y], [p.width, p.height])
                .map_err(|e| format!("HUD patch: {e:?}"))?;
            incoming = incoming
                .checked_add(p.rgba.len())
                .filter(|&n| n <= working_limit)
                .ok_or("HUD patch byte budget exceeded")?;
        }
        let mut working: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
        let mut decoded_bytes = incoming;
        let mut written_bytes = 0usize;
        for p in patches {
            for page in self
                .layout
                .affected([p.x, p.y], [p.width, p.height])
                .map_err(|e| format!("HUD patch: {e:?}"))?
            {
                if let std::collections::btree_map::Entry::Vacant(entry) = working.entry(page) {
                    decoded_bytes = decoded_bytes
                        .checked_add(self.layout.page_bytes())
                        .filter(|&n| n <= working_limit)
                        .ok_or("HUD patch decoded-page budget exceeded")?;
                    entry.insert(self.load(page)?);
                }
                let pixels = working.get_mut(&page).unwrap();
                let side = self.layout.page_size();
                let origin = self
                    .layout
                    .origin(page)
                    .map_err(|e| format!("HUD patch: {e:?}"))?;
                // Only visit the dirty rectangle, not a whole tile per glyph.
                // Source-edge texels can alias the entire padded tail of a tile.
                let range = |axis: usize, start: u32, length: u32| {
                    let local = |v: u32| {
                        (i64::from(v) - i64::from(origin[axis]) + 1).clamp(0, i64::from(side))
                            as u32
                    };
                    let lo = if start == 0 { 0 } else { local(start) };
                    let hi = if start + length == self.layout.size()[axis] {
                        side
                    } else {
                        local(start + length)
                    };
                    lo..hi
                };
                let horizontal = range(0, p.x, p.width);
                let vertical = range(1, p.y, p.height);
                let writes = (horizontal.end - horizontal.start) as usize
                    * (vertical.end - vertical.start) as usize
                    * 4;
                written_bytes = written_bytes
                    .checked_add(writes)
                    .filter(|&n| n <= working_limit)
                    .ok_or("HUD patch write-work budget exceeded")?;
                for y in vertical {
                    let [_, sy] = self
                        .layout
                        .source_texel(page, [0, y])
                        .map_err(|e| format!("HUD patch: {e:?}"))?;
                    for x in horizontal.clone() {
                        let [sx, _] = self
                            .layout
                            .source_texel(page, [x, y])
                            .map_err(|e| format!("HUD patch: {e:?}"))?;
                        let src =
                            ((sy - p.y) as usize * p.width as usize + (sx - p.x) as usize) * 4;
                        let dst = (y as usize * side as usize + x as usize) * 4;
                        pixels[dst..dst + 4].copy_from_slice(&p.rgba[src..src + 4]);
                    }
                }
            }
        }
        let mut bytes = self.bytes;
        for &page in working.keys() {
            bytes -= self.pages[page].len();
        }
        let mut staged = Vec::new();
        for (page, pixels) in working {
            let compressed = lz4_flex::compress(&pixels);
            bytes = bytes
                .checked_add(compressed.len())
                .filter(|&n| n <= self.limit)
                .ok_or("compressed HUD patch budget exceeded")?;
            staged.push((page, compressed));
        }
        let changed = staged.iter().map(|(p, _)| *p).collect();
        for (page, compressed) in staged {
            self.pages[page] = compressed;
        }
        self.bytes = bytes;
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bridge_backing_edits_both_gutters_and_retains_unmodified_texels() {
        let layout = Layout::new([3, 1], 4, 2).unwrap();
        let mut backing = Backing::new(layout, &[0; 12], 1024).unwrap();
        let changed = backing
            .patch(
                &[Patch {
                    x: 1,
                    y: 0,
                    width: 1,
                    height: 1,
                    rgba: vec![1, 2, 3, 255],
                }],
                1024,
            )
            .unwrap();
        assert_eq!(changed, [0, 1]);
        let a = backing.load(0).unwrap();
        let b = backing.load(1).unwrap();
        assert_eq!(&a[8..12], &[1, 2, 3, 255]);
        assert_eq!(&b[..4], &[1, 2, 3, 255]);
        assert_eq!(&a[..8], &[0; 8]);
        assert_eq!(&b[..16], &b[48..]);
    }
    #[test]
    fn bridge_patch_budgets_and_bad_late_patches_leave_backing_unchanged() {
        let layout = Layout::new([3, 1], 4, 2).unwrap();
        let mut backing = Backing::new(layout, &[0; 12], 1024).unwrap();
        let before = backing.load(0).unwrap();
        let patch = Patch {
            x: 0,
            y: 0,
            width: 3,
            height: 1,
            rgba: vec![1, 2, 3, 255, 7, 8, 9, 255, 42, 43, 44, 255],
        };
        assert!(backing.patch(std::slice::from_ref(&patch), 1).is_err());
        assert_eq!(backing.load(0).unwrap(), before);
        let mut bad = patch.clone();
        bad.x = 3;
        assert!(backing.patch(&[patch.clone(), bad], 1024).is_err());
        assert_eq!(backing.load(0).unwrap(), before);
        backing.limit = backing.bytes;
        assert!(backing.patch(&[patch], 1024).is_err());
        assert_eq!(backing.load(0).unwrap(), before);
    }
    #[test]
    fn bridge_patch_rectangles_match_full_rebuild_including_all_edge_aliases() {
        let layout = Layout::new([5, 3], 4, 6).unwrap();
        let original: Vec<u8> = (0..60).collect();
        for y in 0..3 {
            for x in 0..5 {
                for height in 1..=3 - y {
                    for width in 1..=5 - x {
                        let mut backing = Backing::new(layout, &original, 4096).unwrap();
                        let patch = Patch {
                            x,
                            y,
                            width,
                            height,
                            rgba: vec![240; (width * height * 4) as usize],
                        };
                        backing.patch(&[patch], 4096).unwrap();
                        let mut expected = original.clone();
                        for row in y..y + height {
                            let offset = ((row * 5 + x) * 4) as usize;
                            expected[offset..offset + width as usize * 4].fill(240);
                        }
                        for page in 0..layout.page_count() {
                            assert_eq!(
                                backing.load(page).unwrap(),
                                layout.pixels(page, &expected).unwrap()
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn bridge_patch_work_is_bounded_even_for_repeated_tiny_edge_edits() {
        let layout = Layout::new([1, 1], 4, 1).unwrap();
        let mut backing = Backing::new(layout, &[0; 4], 1024).unwrap();
        let patch = Patch {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            rgba: vec![255; 4],
        };
        // Incoming+decoded bytes fit (72), but 128 bytes of repeated writes do not.
        assert!(backing.patch(&[patch.clone(), patch], 80).is_err());
        assert_eq!(backing.load(0).unwrap(), vec![0; 64]);
    }
}
