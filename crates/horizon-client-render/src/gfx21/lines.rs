//! PR 7: line batches for the GL2 `line` program.
//!
//! Selection outlines and the crosshair are axis-aligned rectangles with a
//! one-pixel thickness. The CPU expands each segment to an `OverlayRect`; no
//! geometry shader or wide-line extension is required.

use super::sections::OverlayRect;
use super::state::{GL_BLEND, GL_TRIANGLES, GL_UNSIGNED_SHORT, GlCommand};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineSegment {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub color: [u8; 4],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineBatch {
    pub segments: Vec<LineSegment>,
}

impl LineBatch {
    pub fn reference() -> Self {
        Self {
            segments: vec![
                LineSegment {
                    x0: 36,
                    y0: 30,
                    x1: 39,
                    y1: 30,
                    color: [255, 255, 255, 255],
                },
                LineSegment {
                    x0: 41,
                    y0: 30,
                    x1: 44,
                    y1: 30,
                    color: [255, 255, 255, 255],
                },
                LineSegment {
                    x0: 40,
                    y0: 26,
                    x1: 40,
                    y1: 29,
                    color: [255, 255, 255, 255],
                },
                LineSegment {
                    x0: 40,
                    y0: 31,
                    x1: 40,
                    y1: 34,
                    color: [255, 255, 255, 255],
                },
            ],
        }
    }

    pub fn to_rects(&self) -> Vec<OverlayRect> {
        let mut rects = Vec::new();
        for segment in &self.segments {
            if segment.y0 == segment.y1 {
                let x0 = segment.x0.min(segment.x1);
                let x1 = segment.x0.max(segment.x1) + 1;
                rects.push(OverlayRect {
                    x0,
                    y0: segment.y0,
                    x1,
                    y1: segment.y0 + 1,
                    color: segment.color,
                });
            } else if segment.x0 == segment.x1 {
                let y0 = segment.y0.min(segment.y1);
                let y1 = segment.y0.max(segment.y1) + 1;
                rects.push(OverlayRect {
                    x0: segment.x0,
                    y0,
                    x1: segment.x0 + 1,
                    y1,
                    color: segment.color,
                });
            }
        }
        rects
    }

    pub fn draw_call_count(&self) -> u32 {
        self.to_rects().len() as u32
    }

    pub fn gl_commands(&self) -> Vec<GlCommand> {
        let mut commands = vec![
            GlCommand::UseProgram(5),
            GlCommand::Disable(GL_BLEND),
            GlCommand::DepthMask(false),
        ];
        for index in 0..self.draw_call_count() {
            commands.push(GlCommand::DrawRangeElements {
                mode: GL_TRIANGLES,
                start: 0,
                end: 3,
                count: 6,
                ty: GL_UNSIGNED_SHORT,
                offset: index * 12,
            });
        }
        commands
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_crosshair_expands_to_four_explicit_rects() {
        let batch = LineBatch::reference();
        assert_eq!(batch.draw_call_count(), 4);
        assert_eq!(
            batch.to_rects(),
            vec![
                OverlayRect {
                    x0: 36,
                    y0: 30,
                    x1: 40,
                    y1: 31,
                    color: [255, 255, 255, 255]
                },
                OverlayRect {
                    x0: 41,
                    y0: 30,
                    x1: 45,
                    y1: 31,
                    color: [255, 255, 255, 255]
                },
                OverlayRect {
                    x0: 40,
                    y0: 26,
                    x1: 41,
                    y1: 30,
                    color: [255, 255, 255, 255]
                },
                OverlayRect {
                    x0: 40,
                    y0: 31,
                    x1: 41,
                    y1: 35,
                    color: [255, 255, 255, 255]
                },
            ]
        );
    }

    #[test]
    fn line_commands_use_the_line_program() {
        let commands = LineBatch::reference().gl_commands();
        assert_eq!(commands[0], GlCommand::UseProgram(5));
        assert_eq!(
            commands
                .iter()
                .filter(|c| matches!(c, GlCommand::DrawRangeElements { .. }))
                .count(),
            4
        );
    }
}
