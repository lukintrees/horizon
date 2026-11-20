//! CPU backing for a virtual HUD atlas. Store independent LZ4 blocks rather than
//! retain a second full-size RGBA image. Page decompression is fixed-size/bounded.
//! A page that was just patched stays decoded in a small bounded cache instead of
//! being re-compressed and decoded again for the same frame's upload: on the legacy
//! backend that round trip was over half of the frame's CPU.
//! No wgpu or GL object is named here: the legacy backend pages the HUD atlas
//! through `horizon_render_api::paged_gui`, and the modern one keeps its own copy.
use crate::font::Patch;
use horizon_render_api::paged_gui::Layout;
use std::collections::BTreeSet;

/// Pages kept decoded between the patch that changed them and the frame that draws
/// them. A frame edits every animated sprite's page and then draws the sprite, entity
/// and glyph pages it touches; the shipped atlas spreads its animations over about a
/// dozen 1 MiB pages and a world frame's entities and glyphs over several more. Kept
/// in step with `Limits::hud_resident_pages`, because a page evicted from the GPU
/// cache is decoded again by the very next upload -- the cost this cache removes --
/// and a page decoded here is never read while it is not resident anyway. Below the
/// shipped 128 MiB backing budget (64 MiB here).
const DECODED_PAGES: usize = 64;

pub struct Backing {
    layout: Layout,
    /// LZ4 blocks; empty while the page is decoded.
    pages: Vec<Vec<u8>>,
    /// Decoded pages, least recently patched first.
    decoded: Vec<(usize, Vec<u8>)>,
    /// What the backing occupies: decoded pages at full size, the rest compressed.
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
            decoded: Vec::new(),
            bytes,
            limit,
        })
    }
    /// What the backing holds now, decoded pages charged their full size.
    pub fn held_bytes(&self) -> usize {
        self.bytes
    }
    pub fn set_limit(&mut self, limit: usize) -> Result<(), String> {
        if self.bytes > limit {
            return Err("HUD backing limit below resident bytes".into());
        }
        self.limit = limit;
        Ok(())
    }
    pub fn layout(&self) -> Layout {
        self.layout
    }
    pub fn load(&self, page: usize) -> Result<Vec<u8>, String> {
        if let Some((_, pixels)) = self.decoded.iter().find(|(p, _)| *p == page) {
            return Ok(pixels.clone());
        }
        self.decode(page)
    }
    fn decode(&self, page: usize) -> Result<Vec<u8>, String> {
        let compressed = self.pages.get(page).ok_or("invalid HUD backing page")?;
        if compressed.is_empty() {
            return Err("HUD backing page is decoded, not stored".into());
        }
        let pixels = lz4_flex::decompress(compressed, self.layout.page_bytes())
            .map_err(|e| e.to_string())?;
        if pixels.len() != self.layout.page_bytes() {
            return Err("invalid HUD backing length".into());
        }
        Ok(pixels)
    }
    /// Copy only sprite interiors, decoding at most one bounded page at a time.
    /// Rows/UV orientation are preserved; gutters belong to paging, not sprites.
    pub fn extract(&self, rect: [u32; 4], byte_limit: usize) -> Result<Vec<u8>, String> {
        let [x, y, width, height] = rect;
        let pages = self
            .layout
            .affected([x, y], [width, height])
            .map_err(|e| format!("screen rect: {e:?}"))?;
        let bytes = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .filter(|&n| n <= byte_limit)
            .ok_or("screen sprite byte budget exceeded")?;
        let mut result = Vec::new();
        result
            .try_reserve_exact(bytes)
            .map_err(|_| "screen allocation failed")?;
        result.resize(bytes, 0);
        let span = self.layout.page_size() - 2;
        for page in pages {
            let origin = self
                .layout
                .origin(page)
                .map_err(|e| format!("screen page: {e:?}"))?;
            let lo = [x.max(origin[0]), y.max(origin[1])];
            let hi = [
                (x + width).min(origin[0].saturating_add(span)),
                (y + height).min(origin[1].saturating_add(span)),
            ];
            if lo[0] >= hi[0] || lo[1] >= hi[1] {
                continue;
            }
            let pixels = self.load(page)?;
            for row in lo[1]..hi[1] {
                let src = ((row - origin[1] + 1) as usize * self.layout.page_size() as usize
                    + (lo[0] - origin[0] + 1) as usize)
                    * 4;
                let dst = ((row - y) as usize * width as usize + (lo[0] - x) as usize) * 4;
                let len = (hi[0] - lo[0]) as usize * 4;
                result[dst..dst + len].copy_from_slice(&pixels[src..src + len]);
            }
        }
        Ok(result)
    }
    /// All patches are validated and charged before anything is mutated. The working
    /// budget bounds incoming pixels plus the pages decoded to receive them; each page
    /// is decoded once even when many glyphs touch it, and a page that is already
    /// decoded is edited in place. Patch order is preserved. Patched pages stay
    /// decoded, so the same frame's upload reads them without re-compressing and
    /// decoding again; the oldest decoded page is compressed only to make room.
    pub fn patch(&mut self, patches: &[Patch], working_limit: usize) -> Result<Vec<usize>, String> {
        let mut incoming = 0usize;
        let mut touched = BTreeSet::new();
        let mut writes = 0usize;
        for p in patches {
            let length = (p.width as usize)
                .checked_mul(p.height as usize)
                .and_then(|n| n.checked_mul(4));
            if length != Some(p.rgba.len()) {
                return Err("invalid HUD patch RGBA length".into());
            }
            let pages = self
                .layout
                .affected([p.x, p.y], [p.width, p.height])
                .map_err(|e| format!("HUD patch: {e:?}"))?;
            incoming = incoming
                .checked_add(p.rgba.len())
                .filter(|&n| n <= working_limit)
                .ok_or("HUD patch byte budget exceeded")?;
            for page in pages {
                touched.insert(page);
                writes = writes
                    .checked_add(self.write_bytes(page, p)?)
                    .filter(|&n| n <= working_limit)
                    .ok_or("HUD patch write-work budget exceeded")?;
            }
        }
        let decode = touched
            .iter()
            .filter(|page| !self.is_decoded(**page))
            .count();
        let decoded = decode
            .checked_mul(self.layout.page_bytes())
            .ok_or("HUD patch decoded-page overflow")?;
        if incoming
            .checked_add(decoded)
            .is_none_or(|n| n > working_limit)
        {
            return Err("HUD patch decoded-page budget exceeded".into());
        }
        // Decoding a page trades its block for the full image, which is usually
        // larger; an incompressible page can be the other way round.
        let held = touched.iter().fold(0i64, |held, &page| {
            if self.is_decoded(page) {
                held
            } else {
                held + self.layout.page_bytes() as i64 - self.pages[page].len() as i64
            }
        });
        let bytes = usize::try_from(self.bytes as i64 + held)
            .ok()
            .filter(|&n| n <= self.limit)
            .ok_or("HUD patch backing budget exceeded")?;
        let changed: Vec<usize> = touched.into_iter().collect();
        for &page in &changed {
            if !self.is_decoded(page) {
                let pixels = self.decode(page)?;
                self.pages[page].clear();
                self.decoded.push((page, pixels));
            }
        }
        for p in patches {
            for page in self
                .layout
                .affected([p.x, p.y], [p.width, p.height])
                .map_err(|e| format!("HUD patch: {e:?}"))?
            {
                let at = self
                    .decoded
                    .iter()
                    .position(|(held, _)| *held == page)
                    .expect("every touched page was decoded above");
                self.write_page(at, p)?;
                // Most recently patched last: eviction drops the coldest page.
                let entry = self.decoded.remove(at);
                self.decoded.push(entry);
            }
        }
        self.bytes = bytes;
        // The cache is bounded twice over: by the pages a frame edits and draws, and
        // by the backing budget. A page that no longer fits either is compressed again.
        while self.decoded.len() > DECODED_PAGES || self.bytes > self.limit {
            let (page, pixels) = self.decoded.remove(0);
            let compressed = lz4_flex::compress(&pixels);
            self.bytes = self
                .bytes
                .saturating_sub(pixels.len())
                .saturating_add(compressed.len());
            self.pages[page] = compressed;
        }
        Ok(changed)
    }
    fn is_decoded(&self, page: usize) -> bool {
        self.decoded.iter().any(|(p, _)| *p == page)
    }
    /// The texels one patch writes into one page. Only the dirty rectangle is
    /// visited, not a whole tile per glyph; source-edge texels alias the padded
    /// tail of a tile, so the rectangle widens at the image's edges.
    fn write_bytes(&self, page: usize, p: &Patch) -> Result<usize, String> {
        let (horizontal, vertical) = self.write_ranges(page, p)?;
        Ok((horizontal.end - horizontal.start) as usize
            * (vertical.end - vertical.start) as usize
            * 4)
    }
    fn write_ranges(
        &self,
        page: usize,
        p: &Patch,
    ) -> Result<(std::ops::Range<u32>, std::ops::Range<u32>), String> {
        let side = self.layout.page_size();
        let origin = self
            .layout
            .origin(page)
            .map_err(|e| format!("HUD patch: {e:?}"))?;
        let range = |axis: usize, start: u32, length: u32| {
            let local = |v: u32| {
                (i64::from(v) - i64::from(origin[axis]) + 1).clamp(0, i64::from(side)) as u32
            };
            let lo = if start == 0 { 0 } else { local(start) };
            let hi = if start + length == self.layout.size()[axis] {
                side
            } else {
                local(start + length)
            };
            lo..hi
        };
        Ok((range(0, p.x, p.width), range(1, p.y, p.height)))
    }
    fn write_page(&mut self, at: usize, p: &Patch) -> Result<(), String> {
        let page = self.decoded[at].0;
        let side = self.layout.page_size();
        let (horizontal, vertical) = self.write_ranges(page, p)?;
        let pixels = &mut self.decoded[at].1;
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
                let src = ((sy - p.y) as usize * p.width as usize + (sx - p.x) as usize) * 4;
                let dst = (y as usize * side as usize + x as usize) * 4;
                pixels[dst..dst + 4].copy_from_slice(&p.rgba[src..src + 4]);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backing_limit_changes_are_checked_before_mutation() {
        let layout = Layout::new([3, 1], 4, 2).unwrap();
        let mut backing = Backing::new(layout, &[255; 12], 1024).unwrap();
        let bytes = backing.held_bytes();
        assert!(backing.set_limit(bytes - 1).is_err());
        assert_eq!(backing.limit, 1024);
        backing.set_limit(bytes).unwrap();
        assert_eq!(backing.limit, bytes);
        assert_eq!(backing.load(0).unwrap(), vec![255; 64]);
        backing.set_limit(2048).unwrap();
        assert_eq!(backing.limit, 2048);
    }
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
    /// The reason the decoded cache exists: a page that changes every tick is
    /// decoded once and stays decoded, so patching it again costs no LZ4 work and
    /// does not change what the backing holds.
    #[test]
    fn a_patched_page_stays_decoded_and_is_not_recompressed_for_the_next_upload() {
        let layout = Layout::new([5, 1], 4, 3).unwrap();
        let mut backing = Backing::new(layout, &[0; 20], 4096).unwrap();
        let patch = |value: u8| Patch {
            x: 1,
            y: 0,
            width: 1,
            height: 1,
            rgba: vec![value; 4],
        };
        backing.patch(&[patch(9)], 4096).unwrap();
        let after_first = backing.held_bytes();
        backing.patch(&[patch(7)], 4096).unwrap();
        assert_eq!(backing.held_bytes(), after_first);
        assert_eq!(&backing.load(0).unwrap()[8..12], &[7; 4]);
    }
    /// More patched pages than the cache holds: the oldest is compressed again and
    /// still reads back its edits.
    #[test]
    fn pages_past_the_decoded_cache_are_compressed_again_and_keep_their_edits() {
        let size = 2 * (DECODED_PAGES as u32 + 2);
        let layout = Layout::new([size, 1], 4, size.div_ceil(2) as usize).unwrap();
        let mut backing = Backing::new(layout, &vec![0; (size * 4) as usize], 1 << 20).unwrap();
        for x in 0..size {
            backing
                .patch(
                    &[Patch {
                        x,
                        y: 0,
                        width: 1,
                        height: 1,
                        rgba: vec![x as u8, 1, 2, 255],
                    }],
                    1 << 20,
                )
                .unwrap();
        }
        assert!(backing.decoded.len() <= DECODED_PAGES);
        for page in 0..layout.page_count() {
            let pixels = backing.load(page).unwrap();
            let origin = layout.origin(page).unwrap();
            for x in origin[0]..(origin[0] + 2).min(size) {
                let at = (x - origin[0] + 1) as usize * 4;
                assert_eq!(
                    &pixels[at..at + 4],
                    &[x as u8, 1, 2, 255],
                    "page {page} x {x}"
                );
            }
        }
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
