//! What a graphics context must offer before the GL 2.1 renderer opens a window, and what to tell a
//! player whose driver does not.
//!
//! `docs/opengl21-plan.md` §9.2 is the checklist; this is it as data, so that the check runs in CI
//! (`tests` at the bottom, on made-up contexts) rather than on the one machine that has the card.
//! The plan's Phase 1 asks for the failure to be a sentence, not a panic: a player on a driver
//! without framebuffer objects should read what is missing and what they can do, not get a crash.
//!
//! Two kinds of requirement, and the difference matters on a 2006 driver:
//!
//! - **Required**: OpenGL 2.1, GLSL 1.20, a texture big enough for a sprite atlas, the vertex
//!   attributes the terrain layout uses, the texture units the shader binds, framebuffer objects for
//!   screenshots and post-processing. Without one of these there is no picture.
//! - **Wanted**: extensions that make the frame faster or the picture closer to the vanilla one
//!   (occlusion queries, `ARB_draw_buffers`, `EXT_texture_lod_bias`) or that the path does not need
//!   at all (non-power-of-two textures with mips: every atlas is a power of two). A missing one is a
//!   log line, never a failure.

use crate::gl21::Profile;

/// What a context reports about itself, in the terms the checklist asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// (major, minor) of `GL_VERSION`.
    pub gl: (u8, u8),
    /// `GL_SHADING_LANGUAGE_VERSION` in hundreds: 120, 130, 330.
    pub glsl: u16,
    /// `GL_MAX_TEXTURE_SIZE`.
    pub max_texture_size: u32,
    /// `GL_MAX_VERTEX_ATTRIBS`.
    pub max_vertex_attributes: u32,
    /// `GL_MAX_TEXTURE_IMAGE_UNITS`.
    pub max_texture_units: u32,
    /// `GL_ARB_framebuffer_object` (or GL 3.0, where it is core).
    pub framebuffer_objects: bool,
    /// `GL_ARB_draw_buffers` (or GL 2.0).
    pub draw_buffers: bool,
    /// `GL_ARB_occlusion_query` (or GL 1.5).
    pub occlusion_query: bool,
    /// `GL_ARB_texture_non_power_of_two`: wanted off, not on — the atlases are powers of two.
    pub non_power_of_two: bool,
    /// `GL_EXT_texture_lod_bias`: the mip-chain bias the fog and the LOD path want.
    pub texture_lod_bias: bool,
}

impl Capabilities {
    /// The X1650 under Catalyst 9.3, as its report lists it: what a passing context looks like.
    pub const X1650: Capabilities = Capabilities {
        gl: (2, 1),
        glsl: 120,
        max_texture_size: 4096,
        max_vertex_attributes: 16,
        max_texture_units: 16,
        framebuffer_objects: true,
        draw_buffers: true,
        occlusion_query: true,
        non_power_of_two: true,
        texture_lod_bias: true,
    };

    /// What `profile` promises: a context made to pretend it is that card (`HORIZON_GL_PROFILE`).
    pub fn of(profile: Profile) -> Capabilities {
        Capabilities {
            gl: profile.gl,
            glsl: profile.glsl,
            max_texture_size: profile.max_texture_size,
            max_vertex_attributes: profile.max_vertex_attributes,
            max_texture_units: profile.max_texture_units,
            framebuffer_objects: profile.framebuffer_objects,
            draw_buffers: profile.draw_buffers > 0,
            occlusion_query: true,
            non_power_of_two: !profile.npot_mipmaps,
            texture_lod_bias: true,
        }
    }

    /// What this context cannot run the renderer with, each as a sentence naming what is missing and
    /// what the player can do. Empty when the context is enough.
    ///
    /// The floors are the renderer's own, not vanilla's: 2.1 and GLSL 1.20 are the shaders' version,
    /// a 1024-texel texture is the smallest sprite atlas that holds vanilla's block sprites (the
    /// plan hopes for 4096), four vertex attributes is `gl21::vertex`'s layout, and two texture units
    /// are the sprite atlas and the lightmap.
    pub fn missing(&self) -> Vec<String> {
        let mut missing = Vec::new();
        if self.gl < (2, 1) {
            missing.push(format!(
                "OpenGL 2.1 is needed and this driver reports {}.{}",
                self.gl.0, self.gl.1
            ));
        }
        if self.glsl < 120 {
            missing.push(format!(
                "GLSL 1.20 is needed and this driver compiles {}",
                glsl_name(self.glsl)
            ));
        }
        if self.max_texture_size < 1024 {
            missing.push(format!(
                "a 1024-texel texture is needed for the sprite atlas and this driver's largest \
                 is {}",
                self.max_texture_size
            ));
        }
        if self.max_vertex_attributes < 4 {
            missing.push(format!(
                "four vertex attributes are needed for terrain and this driver allows {}",
                self.max_vertex_attributes
            ));
        }
        if self.max_texture_units < 2 {
            missing.push(format!(
                "two texture units are needed (sprites, lightmap) and this driver has {}",
                self.max_texture_units
            ));
        }
        if !self.framebuffer_objects && self.gl < (3, 0) {
            missing.push(
                "framebuffer objects (GL_ARB_framebuffer_object) are needed for screenshots and \
                 post-processing; update the graphics driver"
                    .to_string(),
            );
        }
        missing
    }

    /// What this context has that costs speed or a feature, though not the picture. Never a reason
    /// to refuse to start: each line is for the log.
    ///
    /// An extension is only worth a line when the context's OpenGL version does not already promise
    /// it (occlusion queries are core in 1.5, draw buffers in 2.0, the LOD bias in 1.4), so a driver
    /// that reports 2.1 and no `GL_ARB_occlusion_query` has it anyway.
    pub fn wanted(&self) -> Vec<String> {
        let mut wanted = Vec::new();
        if self.max_texture_size < 4096 {
            wanted.push(format!(
                "the sprite atlas is limited to {} texels a side instead of 4096: fewer \
                 sprites fit one texture, so more atlas binds per frame",
                self.max_texture_size
            ));
        }
        if !self.occlusion_query && self.gl < (1, 5) {
            wanted.push(
                "no occlusion queries (GL_ARB_occlusion_query): the whole view distance is drawn"
                    .to_string(),
            );
        }
        if !self.draw_buffers && self.gl < (2, 0) {
            wanted.push("no draw buffers: post-processing is limited to one target".to_string());
        }
        if !self.texture_lod_bias && self.gl < (1, 4) {
            wanted.push("no texture LOD bias (GL_EXT_texture_lod_bias)".to_string());
        }
        // Nothing warns about non-power-of-two textures: the atlases are powers of two (§5.2).
        let _ = self.non_power_of_two;
        wanted
    }

    /// Whether the context can run the renderer at all: `missing` is empty.
    pub fn enough(&self) -> bool {
        self.missing().is_empty()
    }
}

/// `120` as `1.20`, `330` as `3.30`: the way a driver's version strings are read out loud.
pub fn glsl_name(glsl: u16) -> String {
    format!("{}.{:02}", glsl / 100, glsl % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_target_card_passes_its_own_checklist() {
        assert!(Capabilities::X1650.enough());
        assert!(Capabilities::X1650.missing().is_empty());
        assert_eq!(Capabilities::X1650.wanted(), Vec::<String>::new());
    }

    #[test]
    fn a_profile_as_a_context_asks_for_nothing_it_does_not_have() {
        // The X1650's profile, read as the context it pretends to be, passes the same checklist.
        assert_eq!(Capabilities::of(Profile::X1650), Capabilities::X1650);
        assert!(Capabilities::of(Profile::X1650).enough());
    }

    #[test]
    fn an_open_gl_1_4_driver_is_told_what_is_missing_in_sentences() {
        let old = Capabilities {
            gl: (1, 4),
            glsl: 110,
            max_texture_size: 512,
            max_vertex_attributes: 2,
            max_texture_units: 1,
            framebuffer_objects: false,
            draw_buffers: false,
            occlusion_query: false,
            non_power_of_two: false,
            texture_lod_bias: false,
        };
        let missing = old.missing();
        assert_eq!(missing.len(), 6, "{missing:?}");
        assert!(missing[0].contains("2.1") && missing[0].contains("1.4"));
        assert!(missing[1].contains("1.10"));
        assert!(missing[2].contains("1024") && missing[2].contains("512"));
        assert!(missing[3].contains("attribute"));
        assert!(missing[4].contains("texture unit"));
        assert!(missing[5].contains("framebuffer"));
        // What only costs speed is a separate list, not a reason to refuse: the small texture, the
        // two features 1.4 does not have — and not the LOD bias, which 1.4 made core.
        let wanted = old.wanted();
        assert_eq!(wanted.len(), 3, "{wanted:?}");
        assert!(wanted[0].contains("512"));
        assert!(wanted[1].contains("occlusion"));
        assert!(wanted[2].contains("draw buffers"));
    }

    #[test]
    fn a_context_that_passes_warns_about_nothing_it_promises() {
        // A 2.1 driver that lists no extension at all still has occlusion queries and draw buffers
        // as core, and a 4096-texel texture: nothing to log.
        let plain = Capabilities {
            occlusion_query: false,
            draw_buffers: false,
            texture_lod_bias: false,
            non_power_of_two: false,
            ..Capabilities::X1650
        };
        assert!(plain.enough(), "{:?}", plain.missing());
        assert_eq!(plain.wanted(), Vec::<String>::new());
    }

    #[test]
    fn a_smaller_texture_than_the_plan_hopes_for_is_one_line() {
        let small = Capabilities {
            max_texture_size: 2048,
            ..Capabilities::X1650
        };
        assert!(small.enough(), "2048 is still enough to start");
        let wanted = small.wanted();
        assert_eq!(wanted.len(), 1, "{wanted:?}");
        assert!(wanted[0].contains("2048") && wanted[0].contains("4096"));
    }

    #[test]
    fn framebuffer_objects_come_free_from_open_gl_3() {
        let core = Capabilities {
            gl: (3, 0),
            glsl: 130,
            framebuffer_objects: false,
            ..Capabilities::X1650
        };
        assert!(core.enough(), "GL 3.0 has them in core");
        // 2.1 without the extension, though: that is one sentence, about framebuffer objects.
        let older = Capabilities {
            framebuffer_objects: false,
            ..Capabilities::X1650
        };
        assert_eq!(older.missing().len(), 1);
        assert!(older.missing()[0].contains("framebuffer"));
    }

    #[test]
    fn glsl_versions_read_as_a_driver_prints_them() {
        assert_eq!(glsl_name(120), "1.20");
        assert_eq!(glsl_name(330), "3.30");
        assert_eq!(glsl_name(110), "1.10");
        // A driver that reports only a major version.
        assert_eq!(glsl_name(100), "1.00");
    }
}
