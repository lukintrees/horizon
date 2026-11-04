//! What the device uploads, and how a texture's sizes are checked.
//!
//! The GL 2.1 renderer's textures come from the game: the sprite atlas (`gl21::Atlas`), the 16×16
//! lightmap, the glyph atlas the HUD patches (`font::Patch`) go into, and the frame texture the
//! device owns (a GL 2.1 driver cannot bind a texture to a sampler object, so the vignette's
//! bilinear copy is a second upload of the same pixels with `LINEAR` set).
//!
//! Everything here is data with a check: a mip chain that does not start at the size it claims, a
//! level whose pixel count is not `width × height × 4`, a texture at the wrong unit. The device
//! issues the calls; the sizes are decided here so that a mistake is a failing test in CI rather
//! than a driver reading past the end of a buffer on a player's machine.

use horizon_client_render::gl21::{Texture, program};

/// A texture the renderer binds, by the name the device knows it by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureId {
    /// The sprite atlas, one flat RGBA8 texture (`gl21::Atlas`).
    SpriteAtlas,
    /// The same pixels again, sampled with bilinear filtering: the vignette's blur, which a wgpu
    /// pipeline sets a sampler object for (`hud::State::linear`).
    SpriteAtlasLinear,
    /// The 16×16 lightmap: 256 texels, `GL_NEAREST`, and the shader samples a level's centre.
    Lightmap,
    /// The glyph atlas: the HUD's text and the pack's GUI sprites.
    Glyphs,
    /// The repeating glint texture an enchanted item's HUD sprite samples (`hud.frag`'s
    /// `v_uv.x >= 2` branch), on `u_glint`'s own unit.
    Glint,
}

impl TextureId {
    /// The unit the shader reads it on, as `program`'s `set_by` says: the terrain and the HUD each
    /// name their own samplers, and the units are what `driver::units` sets.
    pub fn unit(self) -> u32 {
        match self {
            TextureId::SpriteAtlas | TextureId::Glyphs => 0,
            // `u_lightmap` for the world, `u_glint` for the overlay.
            TextureId::Lightmap | TextureId::Glint => 1,
            // `u_linear`, the vignette's bilinear copy.
            TextureId::SpriteAtlasLinear => 2,
        }
    }

    /// The `gl21::Texture` this stands for: what the renderer binds when it binds one of these.
    pub fn as_gl21(self) -> Option<Texture> {
        match self {
            TextureId::SpriteAtlas => Some(Texture::SpriteAtlas),
            TextureId::SpriteAtlasLinear => Some(Texture::SpriteAtlas),
            TextureId::Lightmap => Some(Texture::Lightmap),
            TextureId::Glyphs => Some(Texture::Glyphs),
            // The renderer binds the overlay's glint with its atlas, not on its own.
            TextureId::Glint => None,
        }
    }

    /// Whether the texture is sampled with bilinear filtering: the atlas's mips want `LINEAR_MIPMAP`
    /// and the lightmap must not be smoothed across its levels.
    pub fn linear(self) -> bool {
        !matches!(self, TextureId::Lightmap)
    }

    /// Whether the texture clamps at its edges. The atlas does, so that a mip level does not average
    /// one sprite into the next; the glint is a repeating 8×8 tile that wraps by design
    /// (`hud.wgsl`'s `GLINT_TEXTURING`), so it is the one that repeats.
    pub fn clamped(self) -> bool {
        !matches!(self, TextureId::Glint)
    }
}

/// One mip level of an upload: the level's own size and its RGBA8 pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Level<'a> {
    /// 0 for the base level, 1 the half-sized one, and so on.
    pub level: u32,
    pub width: u32,
    pub height: u32,
    /// `width × height × 4` bytes, red first.
    pub pixels: &'a [u8],
}

impl<'a> Level<'a> {
    /// The level's bytes, as `glTexImage2D`/`glTexSubImage2D` take them.
    pub fn bytes(&self) -> usize {
        self.pixels.len()
    }

    /// Whether the pixels are exactly the size the level claims.
    pub fn fits(&self) -> bool {
        self.pixels.len() == self.width as usize * self.height as usize * 4
    }
}

/// One texture, whole: its base level and the mip levels under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Upload<'a> {
    pub texture: TextureId,
    /// The levels, base first. An empty list clears the slot without making a texture.
    pub levels: &'a [Level<'a>],
}

impl<'a> Upload<'a> {
    /// The base level's size, which is the texture's.
    pub fn size(&self) -> Option<(u32, u32)> {
        self.levels.first().map(|level| (level.width, level.height))
    }

    /// What is wrong with this upload, as a sentence: the levels out of order, a level whose pixels
    /// are not its size, a level that does not halve the one above it, or no levels at all.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.levels.is_empty() {
            problems.push(format!(
                "{:?}: no levels to upload: a texture needs at least its base level",
                self.texture
            ));
            return problems;
        }
        for (index, level) in self.levels.iter().enumerate() {
            if level.level != index as u32 {
                problems.push(format!(
                    "{:?}: level {} is at place {index}; the chain must start at 0 and count up",
                    self.texture, level.level
                ));
            }
            if !level.fits() {
                problems.push(format!(
                    "{:?}: level {} is {}×{} texels and carries {} bytes, not {}",
                    self.texture,
                    level.level,
                    level.width,
                    level.height,
                    level.bytes(),
                    level.width as usize * level.height as usize * 4
                ));
            }
        }
        for pair in self.levels.windows(2) {
            let (above, below) = (pair[0], pair[1]);
            let want = ((above.width / 2).max(1), (above.height / 2).max(1));
            if (below.width, below.height) != want {
                problems.push(format!(
                    "{:?}: level {} is {}×{} and its level above is {}×{}, so it has to be {}×{}",
                    self.texture, below.level, below.width, below.height, above.width, above.height,
                    want.0, want.1
                ));
            }
        }
        problems
    }
}

/// How many mip levels a square texture of `size` texels has, the base level included: what
/// `glTexImage2D` is called once for in a full chain.
pub fn levels_of(size: u32) -> u32 {
    size.max(1).ilog2() + 1
}

/// Which shaders a program compiles, from `program::SOURCES`: (file name, source).
pub fn sources(program: gl21::Program) -> Vec<(&'static str, &'static str)> {
    let wanted = match program {
        gl21::Program::Terrain => ["terrain.vert", "terrain.frag"],
        gl21::Program::Hud => ["hud.vert", "hud.frag"],
        gl21::Program::Present => ["present.vert", "present.frag"],
    };
    program::SOURCES
        .iter()
        .filter(|(name, _)| wanted.contains(name))
        .copied()
        .collect()
}

/// The attributes a program is bound with, in slot order: `glBindAttribLocation` and then
/// `glVertexAttribPointer`, both before the program links.
pub fn attributes(program: gl21::Program) -> &'static [gl21::Attribute] {
    match program {
        gl21::Program::Terrain => &program::TERRAIN,
        gl21::Program::Hud => &program::HUD,
        gl21::Program::Present => &program::PRESENT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client_render::gl21;

    fn level(level: u32, width: u32, height: u32) -> Level<'static> {
        // Leaked on purpose: these are tests of the arithmetic, and a `Level` borrows its pixels.
        let texels = (width * height * 4) as usize;
        let pixels: &'static [u8] = Box::leak(vec![0u8; texels].into_boxed_slice());
        Level {
            level,
            width,
            height,
            pixels,
        }
    }

    #[test]
    fn a_whole_chain_has_no_problems() {
        let levels = [level(0, 64, 64), level(1, 32, 32), level(2, 16, 16)];
        let upload = Upload { texture: TextureId::SpriteAtlas, levels: &levels };
        assert_eq!(upload.problems(), Vec::<String>::new());
        assert_eq!(upload.size(), Some((64, 64)));
        // A texture's chain is a power of two's own: 64 texels have seven levels including 1×1.
        assert_eq!(levels_of(64), 7);
        assert_eq!(levels_of(1), 1);
        assert_eq!(levels_of(4096), 13);
    }

    #[test]
    fn a_level_of_the_wrong_size_or_order_is_a_sentence() {
        let pixels = level(0, 4, 4);
        let wrong_bytes = Level { pixels: &pixels.pixels[..4], ..pixels };
        let short = Upload { texture: TextureId::Lightmap, levels: &[wrong_bytes] };
        let problems = short.problems();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("carries 4 bytes"), "{problems:?}");

        // A chain that skips a level: 4×4 above a 1×1.
        let skipped = [level(0, 4, 4), level(2, 1, 1)];
        let problems = Upload { texture: TextureId::SpriteAtlas, levels: &skipped }.problems();
        let out_of_order = problems.iter().any(|p| p.contains("place 1"));
        assert!(out_of_order, "{problems:?}");

        // And one with no levels at all, which is a slot being cleared rather than a texture.
        let empty = Upload { texture: TextureId::Glyphs, levels: &[] };
        let problems = empty.problems();
        assert!(problems[0].contains("no levels"), "{problems:?}");
        assert_eq!(empty.size(), None);
    }

    #[test]
    fn a_texture_knows_its_unit_and_whether_it_is_smoothed() {
        assert_eq!(TextureId::SpriteAtlas.unit(), 0);
        assert_eq!(TextureId::Glyphs.unit(), 0);
        assert_eq!(TextureId::SpriteAtlasLinear.unit(), 2);
        assert_eq!(TextureId::Lightmap.unit(), 1);
        assert_eq!(TextureId::Glint.unit(), 1);
        // The lightmap is a table of levels, not a picture: smoothing across it would light a block
        // at a level it is not at.
        assert!(!TextureId::Lightmap.linear());
        assert!(TextureId::SpriteAtlas.linear());
        // The atlas clamps, so that a mip level does not average two sprites together; the glint is
        // a repeating tile and wraps.
        assert!(TextureId::SpriteAtlas.clamped());
        assert!(!TextureId::Glint.clamped());
        // The linear copy is the same pixels and the same unit as the atlas it blurs.
        assert_eq!(TextureId::SpriteAtlasLinear.as_gl21(), Some(gl21::Texture::SpriteAtlas));
    }

    #[test]
    fn every_program_has_two_stages_and_its_own_attributes() {
        for program in [gl21::Program::Terrain, gl21::Program::Hud, gl21::Program::Present] {
            let sources = sources(program);
            assert_eq!(sources.len(), 2, "{program:?}");
            assert!(sources[0].0.ends_with(".vert"), "{program:?}");
            assert!(sources[1].0.ends_with(".frag"), "{program:?}");
            assert!(!attributes(program).is_empty(), "{program:?}");
        }
        // The terrain's attributes are `gl21::program`'s own table: the same slots the tests there
        // check against the vertex layout.
        assert_eq!(attributes(gl21::Program::Terrain).len(), 4);
        assert_eq!(attributes(gl21::Program::Hud).len(), 5);
        assert_eq!(attributes(gl21::Program::Present).len(), 1);
    }
}
