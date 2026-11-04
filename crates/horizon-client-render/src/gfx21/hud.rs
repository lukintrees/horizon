//! PR 7: HUD batches for the GL2 `hud` program.
//!
//! The title, inventory, chat and hotbar are plain screen rectangles here; a
//! real font atlas is wired in by the facade later. This module is pure Rust
//! and reproduces a readable reference HUD on the CPU for tests.

use super::blend::BlendMode;
use super::sections::OverlayRect;
use super::state::{GL_TRIANGLES, GL_UNSIGNED_SHORT, GlCommand};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HudElement {
    pub rect: OverlayRect,
    pub blend: BlendMode,
    pub label: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HudBatch {
    pub elements: Vec<HudElement>,
}

fn blend_channel(dst: u8, src: u8, alpha: u8) -> u8 {
    let a = u32::from(alpha);
    ((u32::from(src) * a + u32::from(dst) * (255 - a)) / 255) as u8
}

fn fill_element(out: &mut [u8], width: u32, height: u32, element: &HudElement) {
    let rect = &element.rect;
    let x0 = (rect.x0.max(0) as u32).min(width);
    let y0 = (rect.y0.max(0) as u32).min(height);
    let x1 = (rect.x1.max(0) as u32).min(width);
    let y1 = (rect.y1.max(0) as u32).min(height);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let color = rect.color;
    for y in y0..y1 {
        for x in x0..x1 {
            let at = ((y * width + x) * 4) as usize;
            if element.blend.needs_sort() {
                out[at] = blend_channel(out[at], color[0], color[3]);
                out[at + 1] = blend_channel(out[at + 1], color[1], color[3]);
                out[at + 2] = blend_channel(out[at + 2], color[2], color[3]);
            } else {
                out[at..at + 4].copy_from_slice(&color);
            }
        }
    }
}

impl HudBatch {
    /// Four readable UI areas at 80×60, matching PR 7's acceptance gate.
    pub fn reference() -> Self {
        Self {
            elements: vec![
                HudElement {
                    rect: OverlayRect {
                        x0: 10,
                        y0: 8,
                        x1: 70,
                        y1: 18,
                        color: [220, 220, 220, 255],
                    },
                    blend: BlendMode::Opaque,
                    label: "title",
                },
                HudElement {
                    rect: OverlayRect {
                        x0: 20,
                        y0: 20,
                        x1: 60,
                        y1: 40,
                        color: [90, 90, 90, 255],
                    },
                    blend: BlendMode::Opaque,
                    label: "inventory",
                },
                HudElement {
                    rect: OverlayRect {
                        x0: 4,
                        y0: 34,
                        x1: 36,
                        y1: 46,
                        color: [40, 40, 40, 200],
                    },
                    blend: BlendMode::Alpha,
                    label: "chat",
                },
                HudElement {
                    rect: OverlayRect {
                        x0: 24,
                        y0: 48,
                        x1: 56,
                        y1: 56,
                        color: [160, 160, 160, 255],
                    },
                    blend: BlendMode::Opaque,
                    label: "hotbar",
                },
            ],
        }
    }

    pub fn draw_call_count(&self) -> u32 {
        self.elements.len() as u32
    }

    pub fn gl_commands(&self) -> Vec<GlCommand> {
        let mut commands = vec![GlCommand::UseProgram(4), GlCommand::BindTexture(0, 2)];
        for (index, element) in self.elements.iter().enumerate() {
            commands.extend(element.blend.gl_commands());
            commands.push(GlCommand::DrawRangeElements {
                mode: GL_TRIANGLES,
                start: 0,
                end: 3,
                count: 6,
                ty: GL_UNSIGNED_SHORT,
                offset: (index * 12) as u32,
            });
        }
        commands
    }

    /// RGBA8, top-down rows, deterministic integer arithmetic only.
    pub fn rasterize(&self, width: u32, height: u32) -> Vec<u8> {
        let mut out = vec![0_u8; (width * height * 4) as usize];
        for (index, byte) in out.iter_mut().enumerate() {
            if index % 4 == 3 {
                *byte = 255;
            }
        }
        for element in &self.elements {
            fill_element(&mut out, width, height, element);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * width + x) * 4) as usize;
        rgba[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn reference_hud_has_four_named_areas() {
        let batch = HudBatch::reference();
        assert_eq!(batch.draw_call_count(), 4);
        assert_eq!(
            batch.elements.iter().map(|e| e.label).collect::<Vec<_>>(),
            vec!["title", "inventory", "chat", "hotbar"]
        );
        assert_eq!(batch.elements[2].blend, BlendMode::Alpha);
    }

    #[test]
    fn hud_pixels_have_independent_expected_values() {
        let rgba = HudBatch::reference().rasterize(80, 60);
        assert_eq!(pixel(&rgba, 80, 0, 0), [0, 0, 0, 255]);
        assert_eq!(pixel(&rgba, 80, 11, 9), [220, 220, 220, 255]);
        assert_eq!(pixel(&rgba, 80, 21, 21), [90, 90, 90, 255]);
        assert_eq!(pixel(&rgba, 80, 5, 35), [31, 31, 31, 255]);
        assert_eq!(pixel(&rgba, 80, 25, 49), [160, 160, 160, 255]);
    }

    #[test]
    fn hud_commands_use_the_hud_program_and_emit_one_draw_each() {
        let commands = HudBatch::reference().gl_commands();
        assert_eq!(commands[0], GlCommand::UseProgram(4));
        assert_eq!(commands[1], GlCommand::BindTexture(0, 2));
        assert_eq!(
            commands
                .iter()
                .filter(|c| matches!(c, GlCommand::DrawRangeElements { .. }))
                .count(),
            4
        );
    }
}
