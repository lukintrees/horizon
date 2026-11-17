//! Bounded SDR readback preparation. Input is RGBA8 in GL row order;
//! output is top-down, opaque RGBA8 for a screenshot or centre-cropped icon.
//! No graphics handles, filesystem or image codec live in this module.

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidSize,
    Upscale,
    Budget,
    Length,
    Allocation,
}

pub struct Plan {
    source: [u32; 2],
    output: Option<[u32; 2]>,
    source_bytes: usize,
}

fn bytes([w, h]: [u32; 2]) -> Result<usize, Error> {
    if w == 0 || h == 0 {
        return Err(Error::InvalidSize);
    }
    (w as usize)
        .checked_mul(h as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|n| *n <= isize::MAX as usize)
        .ok_or(Error::InvalidSize)
}

impl Plan {
    /// Validate before GPU readback allocates. Budget includes both the source
    /// and, for an icon, the separate output buffer (not PNG codec overhead).
    pub fn new(source: [u32; 2], output: Option<[u32; 2]>, budget: usize) -> Result<Self, Error> {
        let source_bytes = bytes(source)?;
        let extra = match output {
            Some(size) => {
                let n = bytes(size)?;
                if size.into_iter().any(|side| side > source[0].min(source[1])) {
                    return Err(Error::Upscale);
                }
                n
            }
            None => 0,
        };
        if source_bytes.checked_add(extra).is_none_or(|n| n > budget) {
            return Err(Error::Budget);
        }
        Ok(Self {
            source,
            output,
            source_bytes,
        })
    }

    pub fn size(&self) -> [u32; 2] {
        self.output.unwrap_or(self.source)
    }

    /// Flip in place, then reuse the buffer for a full-frame capture or average
    /// the centre square into a separately bounded icon. Wide sums avoid u32
    /// overflow even for a large frame averaged to one pixel.
    pub fn prepare(&self, mut pixels: Vec<u8>) -> Result<Vec<u8>, Error> {
        if pixels.len() != self.source_bytes {
            return Err(Error::Length);
        }
        let [width, height] = self.source.map(|n| n as usize);
        let row = width * 4;
        for y in 0..height / 2 {
            let (top, bottom) = pixels.split_at_mut((height - y - 1) * row);
            top[y * row..(y + 1) * row].swap_with_slice(&mut bottom[..row]);
        }
        let Some(size) = self.output else {
            for pixel in pixels.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
            return Ok(pixels);
        };
        let mut out = Vec::new();
        let len = bytes(size)?;
        out.try_reserve_exact(len).map_err(|_| Error::Allocation)?;
        out.resize(len, 0);
        let [tw, th] = size.map(|n| n as usize);
        let side = width.min(height);
        let (cx, cy) = ((width - side) / 2, (height - side) / 2);
        for ty in 0..th {
            let (y0, y1) = (cy + ty * side / th, cy + (ty + 1) * side / th);
            for tx in 0..tw {
                let (x0, x1) = (cx + tx * side / tw, cx + (tx + 1) * side / tw);
                let mut sum = [0u64; 3];
                for y in y0..y1 {
                    for x in x0..x1 {
                        let p = (y * width + x) * 4;
                        for channel in 0..3 {
                            sum[channel] += u64::from(pixels[p + channel]);
                        }
                    }
                }
                let count = ((x1 - x0) * (y1 - y0)) as u64;
                let p = (ty * tw + tx) * 4;
                for channel in 0..3 {
                    out[p + channel] = (sum[channel] / count) as u8;
                }
                out[p + 3] = 255;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_frame_is_top_down_opaque_and_reuses_the_allocation() {
        let pixels = vec![1, 2, 3, 0, 4, 5, 6, 7, 8, 9, 10, 0, 11, 12, 13, 14];
        let ptr = pixels.as_ptr();
        let plan = Plan::new([2, 2], None, 16).unwrap();
        assert_eq!(plan.size(), [2, 2]);
        let image = plan.prepare(pixels).unwrap();
        assert_eq!(image.as_ptr(), ptr);
        assert_eq!(
            image,
            [8, 9, 10, 255, 11, 12, 13, 255, 1, 2, 3, 255, 4, 5, 6, 255]
        );
    }
    #[test]
    fn icons_crop_the_centre_square_and_average_without_alpha() {
        let mut pixels = Vec::new();
        // Bottom row then top row; side columns are excluded by the centre crop.
        for r in [255, 10, 30, 255, 255, 50, 70, 255] {
            pixels.extend_from_slice(&[r, 0, 0, 0]);
        }
        let plan = Plan::new([4, 2], Some([1, 2]), 40).unwrap();
        assert_eq!(
            plan.prepare(pixels.clone()).unwrap(),
            [60, 0, 0, 255, 20, 0, 0, 255]
        );
        assert_eq!(
            Plan::new([4, 2], Some([1, 1]), 36)
                .unwrap()
                .prepare(pixels)
                .unwrap(),
            [40, 0, 0, 255]
        );
        // Portrait crop discards the top and bottom rows as well.
        let portrait: Vec<_> = [255, 50, 30, 255]
            .into_iter()
            .flat_map(|r| [r, 0, 0, 0, r, 0, 0, 0])
            .collect();
        assert_eq!(
            Plan::new([2, 4], Some([1, 1]), 36)
                .unwrap()
                .prepare(portrait)
                .unwrap(),
            [40, 0, 0, 255]
        );
    }
    #[test]
    fn invalid_overflow_upscale_and_combined_peak_are_rejected_before_readback() {
        for size in [[0, 1], [1, 0], [u32::MAX; 2]] {
            assert!(matches!(
                Plan::new(size, None, usize::MAX),
                Err(Error::InvalidSize)
            ));
        }
        assert!(matches!(
            Plan::new([4, 2], Some([0, 1]), 100),
            Err(Error::InvalidSize)
        ));
        assert!(matches!(
            Plan::new([4, 2], Some([3, 1]), 100),
            Err(Error::Upscale)
        ));
        assert!(matches!(
            Plan::new([4, 2], Some([1, 1]), 35),
            Err(Error::Budget)
        ));
        assert!(matches!(Plan::new([4, 2], None, 31), Err(Error::Budget)));
        assert_eq!(
            Plan::new([4, 2], None, 32).unwrap().prepare(vec![0; 31]),
            Err(Error::Length)
        );
    }
}
