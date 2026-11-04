//! CPU glyph rasterization into one RGBA texture for the GL2 HUD shader.
//!
//! `ab_glyph` and the bundled Geist Mono face are already dependencies of the
//! client; no new crates, system fonts or network access are required.

use ab_glyph::{Font, FontRef, ScaleFont, point};

use super::ui::TextLine;

pub const MAX_TEXTURE_EDGE: u32 = 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct TextQuad {
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub color: [u8; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextAtlas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub quads: Vec<TextQuad>,
}

impl TextAtlas {
    pub fn build(lines: &[TextLine], width: u32, height: u32) -> Result<Self, String> {
        let width = width.clamp(1, MAX_TEXTURE_EDGE);
        let height = height.clamp(1, MAX_TEXTURE_EDGE);
        let font = FontRef::try_from_slice(horizon_theme::fonts::GEIST_MONO)
            .map_err(|err| format!("font: {err:?}"))?;
        let mut pixels = vec![0_u8; (width * height * 4) as usize];
        let mut quads = Vec::new();
        for line in lines {
            let scale = line.scale.clamp(8.0, 48.0);
            let scaled = font.as_scaled(scale);
            let baseline = line.y + scale * 0.82;
            let mut pen_x = line.x;
            for ch in line.text.chars() {
                let id = font.glyph_id(ch);
                let advance = scaled.h_advance(id);
                let glyph = id.with_scale_and_position(scale, point(pen_x, baseline));
                if let Some(outlined) = font.outline_glyph(glyph) {
                    let color = line.color;
                    outlined.draw(|x, y, coverage| {
                        if x >= width || y >= height || coverage <= 0.0 {
                            return;
                        }
                        let offset = ((y * width + x) * 4) as usize;
                        let alpha = (coverage.clamp(0.0, 1.0) * 255.0) as u8;
                        pixels[offset] = color[0];
                        pixels[offset + 1] = color[1];
                        pixels[offset + 2] = color[2];
                        pixels[offset + 3] = pixels[offset + 3].max(alpha);
                    });
                }
                pen_x += advance.max(1.0);
            }
            let x0 = line.x.max(0.0).min(width as f32);
            let y0 = line.y.max(0.0).min(height as f32);
            let x1 = pen_x.max(x0 + 1.0).min(width as f32);
            let y1 = (line.y + scale * 1.25).max(y0 + 1.0).min(height as f32);
            quads.push(TextQuad {
                rect: [x0, y0, x1, y1],
                uv: [
                    x0 / width as f32,
                    y0 / height as f32,
                    x1 / width as f32,
                    y1 / height as f32,
                ],
                color: line.color,
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
            quads,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_are_rasterized_into_one_texture() {
        let line = TextLine {
            x: 4.0,
            y: 4.0,
            text: "A1".to_owned(),
            scale: 16.0,
            color: [255, 255, 255, 255],
        };
        let atlas = TextAtlas::build(&[line], 128, 64).unwrap();
        assert_eq!(atlas.width, 128);
        assert_eq!(atlas.pixels.len(), 128 * 64 * 4);
        assert_eq!(atlas.quads.len(), 1);
        assert!(atlas.pixels.iter().any(|value| *value != 0));
    }
}
