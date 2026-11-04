//! GLSL 110/120 program sources and static validation (PR 4, full set).
//!
//! The real `glCreateShader`/`glLinkProgram` calls live in the `gl21_clear`
//! example (PR 3 stage 2). This module is pure Rust, so the additive `gfx21`
//! scaffold stays free of the optional GL dependencies. The six pass programs
//! are shipped in both GLSL dialects; F0 uses fixed function only.

use std::error::Error;
use std::fmt;

use super::context::ShaderDialect;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GlslStage {
    Vertex,
    Fragment,
}

/// The PR 4 pass programs. Terrain/entity/HUD/line/sky programs share the
/// GL2 attribute/varying conventions; smoke stays the minimal link/draw probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Pass {
    Smoke,
    Terrain,
    Entity,
    Sky,
    Hud,
    Line,
}

impl Pass {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Smoke => "smoke",
            Self::Terrain => "terrain",
            Self::Entity => "entity",
            Self::Sky => "sky",
            Self::Hud => "hud",
            Self::Line => "line",
        }
    }
}

pub const PASSES: [Pass; 6] = [
    Pass::Smoke,
    Pass::Terrain,
    Pass::Entity,
    Pass::Sky,
    Pass::Hud,
    Pass::Line,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderSource {
    pub name: &'static str,
    pub pass: Pass,
    pub dialect: ShaderDialect,
    pub stage: GlslStage,
    pub source: &'static str,
}

macro_rules! source {
    ($name:literal, $pass:expr, $dialect:expr, $stage:expr, $path:literal) => {
        ShaderSource {
            name: $name,
            pass: $pass,
            dialect: $dialect,
            stage: $stage,
            source: include_str!($path),
        }
    };
}

pub const SMOKE_120_VERT: &str = include_str!("shaders/smoke_120.vert");
pub const SMOKE_120_FRAG: &str = include_str!("shaders/smoke_120.frag");
pub const SMOKE_110_VERT: &str = include_str!("shaders/smoke_110.vert");
pub const SMOKE_110_FRAG: &str = include_str!("shaders/smoke_110.frag");

pub const SHADERS: &[ShaderSource] = &[
    source!(
        "smoke_120.vert",
        Pass::Smoke,
        ShaderDialect::Glsl120,
        GlslStage::Vertex,
        "shaders/smoke_120.vert"
    ),
    source!(
        "smoke_120.frag",
        Pass::Smoke,
        ShaderDialect::Glsl120,
        GlslStage::Fragment,
        "shaders/smoke_120.frag"
    ),
    source!(
        "smoke_110.vert",
        Pass::Smoke,
        ShaderDialect::Glsl110,
        GlslStage::Vertex,
        "shaders/smoke_110.vert"
    ),
    source!(
        "smoke_110.frag",
        Pass::Smoke,
        ShaderDialect::Glsl110,
        GlslStage::Fragment,
        "shaders/smoke_110.frag"
    ),
    source!(
        "terrain_120.vert",
        Pass::Terrain,
        ShaderDialect::Glsl120,
        GlslStage::Vertex,
        "shaders/terrain_120.vert"
    ),
    source!(
        "terrain_120.frag",
        Pass::Terrain,
        ShaderDialect::Glsl120,
        GlslStage::Fragment,
        "shaders/terrain_120.frag"
    ),
    source!(
        "terrain_110.vert",
        Pass::Terrain,
        ShaderDialect::Glsl110,
        GlslStage::Vertex,
        "shaders/terrain_110.vert"
    ),
    source!(
        "terrain_110.frag",
        Pass::Terrain,
        ShaderDialect::Glsl110,
        GlslStage::Fragment,
        "shaders/terrain_110.frag"
    ),
    source!(
        "entity_120.vert",
        Pass::Entity,
        ShaderDialect::Glsl120,
        GlslStage::Vertex,
        "shaders/entity_120.vert"
    ),
    source!(
        "entity_120.frag",
        Pass::Entity,
        ShaderDialect::Glsl120,
        GlslStage::Fragment,
        "shaders/entity_120.frag"
    ),
    source!(
        "entity_110.vert",
        Pass::Entity,
        ShaderDialect::Glsl110,
        GlslStage::Vertex,
        "shaders/entity_110.vert"
    ),
    source!(
        "entity_110.frag",
        Pass::Entity,
        ShaderDialect::Glsl110,
        GlslStage::Fragment,
        "shaders/entity_110.frag"
    ),
    source!(
        "sky_120.vert",
        Pass::Sky,
        ShaderDialect::Glsl120,
        GlslStage::Vertex,
        "shaders/sky_120.vert"
    ),
    source!(
        "sky_120.frag",
        Pass::Sky,
        ShaderDialect::Glsl120,
        GlslStage::Fragment,
        "shaders/sky_120.frag"
    ),
    source!(
        "sky_110.vert",
        Pass::Sky,
        ShaderDialect::Glsl110,
        GlslStage::Vertex,
        "shaders/sky_110.vert"
    ),
    source!(
        "sky_110.frag",
        Pass::Sky,
        ShaderDialect::Glsl110,
        GlslStage::Fragment,
        "shaders/sky_110.frag"
    ),
    source!(
        "hud_120.vert",
        Pass::Hud,
        ShaderDialect::Glsl120,
        GlslStage::Vertex,
        "shaders/hud_120.vert"
    ),
    source!(
        "hud_120.frag",
        Pass::Hud,
        ShaderDialect::Glsl120,
        GlslStage::Fragment,
        "shaders/hud_120.frag"
    ),
    source!(
        "hud_110.vert",
        Pass::Hud,
        ShaderDialect::Glsl110,
        GlslStage::Vertex,
        "shaders/hud_110.vert"
    ),
    source!(
        "hud_110.frag",
        Pass::Hud,
        ShaderDialect::Glsl110,
        GlslStage::Fragment,
        "shaders/hud_110.frag"
    ),
    source!(
        "line_120.vert",
        Pass::Line,
        ShaderDialect::Glsl120,
        GlslStage::Vertex,
        "shaders/line_120.vert"
    ),
    source!(
        "line_120.frag",
        Pass::Line,
        ShaderDialect::Glsl120,
        GlslStage::Fragment,
        "shaders/line_120.frag"
    ),
    source!(
        "line_110.vert",
        Pass::Line,
        ShaderDialect::Glsl110,
        GlslStage::Vertex,
        "shaders/line_110.vert"
    ),
    source!(
        "line_110.frag",
        Pass::Line,
        ShaderDialect::Glsl110,
        GlslStage::Fragment,
        "shaders/line_110.frag"
    ),
];

/// GLSL 3.x/4.x constructs that must never reach an X1650 driver. `texture2D`
/// is still valid in 110/120; `has_word` deliberately does not match it.
const FORBIDDEN: &[&str] = &[
    "in",
    "out",
    "layout",
    "uint",
    "flat",
    "texelFetch",
    "texture",
    "textureLod",
    "textureGrad",
    "textureSize",
    "gl_VertexID",
    "gl_InstanceID",
    "glDrawID",
    "gl_BaseVertex",
    "sampler2DArray",
    "samplerCubeArray",
    "image2D",
    "imageCube",
    "atomicCounter",
    "sampler3D",
    "uniform buffer",
    "switch",
    "case",
    "default",
    "double",
    "dvec2",
    "dvec3",
    "dvec4",
    "uvec2",
    "uvec3",
    "uvec4",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShaderError {
    EmptySource,
    NoGlsl,
    WrongVersion { found: String, expected: u32 },
    ForbiddenToken(&'static str),
    MissingVertexAttribute,
    MissingVertexOutput,
    MissingFragColor,
    MissingMain,
}

impl fmt::Display for ShaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySource => write!(f, "shader source is empty"),
            Self::NoGlsl => write!(f, "the F0 tier has no GLSL source"),
            Self::WrongVersion { found, expected } => {
                write!(f, "expected #version {expected}, found `{found}`")
            }
            Self::ForbiddenToken(token) => write!(f, "GL3+ token `{token}` is forbidden"),
            Self::MissingVertexAttribute => write!(f, "vertex shader has no `attribute`"),
            Self::MissingVertexOutput => write!(f, "vertex shader does not write gl_Position"),
            Self::MissingFragColor => write!(f, "fragment shader does not write gl_FragColor"),
            Self::MissingMain => write!(f, "shader has no `void main`"),
        }
    }
}

impl Error for ShaderError {}

fn has_word(source: &str, word: &str) -> bool {
    source.match_indices(word).any(|(i, _)| {
        let before = source[..i].chars().next_back();
        let after = source[i + word.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
            && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

/// Check one source against the dialect selected by `GlContext`.
pub fn validate(source: &str, dialect: ShaderDialect, stage: GlslStage) -> Result<(), ShaderError> {
    if source.is_empty() {
        return Err(ShaderError::EmptySource);
    }
    let first = source.lines().next().unwrap_or("").trim();
    let found = first
        .strip_prefix("#version ")
        .unwrap_or(first)
        .trim()
        .parse::<u32>()
        .ok();
    let expected = dialect.glsl_version().ok_or(ShaderError::NoGlsl)?;
    if found != Some(expected) {
        return Err(ShaderError::WrongVersion {
            found: first.to_owned(),
            expected,
        });
    }
    for &token in FORBIDDEN {
        if has_word(source, token) {
            return Err(ShaderError::ForbiddenToken(token));
        }
    }
    if !source.contains("void main") {
        return Err(ShaderError::MissingMain);
    }
    match stage {
        GlslStage::Vertex => {
            if !source.contains("attribute") {
                return Err(ShaderError::MissingVertexAttribute);
            }
            if !source.contains("gl_Position") {
                return Err(ShaderError::MissingVertexOutput);
            }
        }
        GlslStage::Fragment => {
            if !source.contains("gl_FragColor") {
                return Err(ShaderError::MissingFragColor);
            }
        }
    }
    Ok(())
}

/// Validate every shipped source and every pass/dialect pair.
pub fn validate_all() -> Result<(), String> {
    for shader in SHADERS {
        validate(shader.source, shader.dialect, shader.stage)
            .map_err(|err| format!("{}: {err}", shader.name))?;
    }
    for &pass in &PASSES {
        for &dialect in &[ShaderDialect::Glsl120, ShaderDialect::Glsl110] {
            if program_for(pass, dialect).is_none() {
                return Err(format!("missing {} program for {dialect:?}", pass.name()));
            }
        }
    }
    Ok(())
}

/// The vertex/fragment pair for one pass in the selected dialect.
pub fn program_for(pass: Pass, dialect: ShaderDialect) -> Option<(&'static str, &'static str)> {
    let glsl = dialect.glsl_version()?;
    let find = |stage| {
        SHADERS
            .iter()
            .find(|s| s.pass == pass && s.dialect.glsl_version() == Some(glsl) && s.stage == stage)
            .map(|s| s.source)
    };
    Some((find(GlslStage::Vertex)?, find(GlslStage::Fragment)?))
}

/// All six pass programs for one dialect, in `PASSES` order. The example links
/// each pair through real GL and keeps only the smoke program for its draw.
pub fn all_programs(
    dialect: ShaderDialect,
) -> Vec<(Pass, &'static str, &'static str, &'static str)> {
    PASSES
        .iter()
        .copied()
        .filter_map(|pass| {
            let (vertex, fragment) = program_for(pass, dialect)?;
            Some((pass, pass.name(), vertex, fragment))
        })
        .collect()
}

/// The smoke triangle's vertex/fragment pair for the selected dialect. F0 has
/// no GLSL program; its fixed-function path arrives with the full renderer.
pub fn smoke_program(dialect: ShaderDialect) -> Option<(&'static str, &'static str)> {
    program_for(Pass::Smoke, dialect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_source_validates() {
        assert_eq!(validate_all(), Ok(()));
        assert_eq!(SHADERS.len(), 24);
    }

    #[test]
    fn full_program_set_has_six_passes_in_both_dialects() {
        for dialect in [ShaderDialect::Glsl120, ShaderDialect::Glsl110] {
            let programs = all_programs(dialect);
            assert_eq!(programs.len(), 6);
            assert!(
                programs
                    .iter()
                    .all(|(_, _, v, f)| !v.is_empty() && !f.is_empty())
            );
            let (v, f) = program_for(Pass::Terrain, dialect).unwrap();
            assert!(v.starts_with(dialect_version_prefix(dialect)));
            assert!(f.contains("gl_FragColor"));
        }
        assert_eq!(all_programs(ShaderDialect::FixedFunction).len(), 0);
    }

    #[test]
    fn smoke_program_matches_the_registry() {
        assert_eq!(
            smoke_program(ShaderDialect::Glsl120),
            program_for(Pass::Smoke, ShaderDialect::Glsl120)
        );
        assert_eq!(
            smoke_program(ShaderDialect::Glsl110),
            program_for(Pass::Smoke, ShaderDialect::Glsl110)
        );
        assert_eq!(smoke_program(ShaderDialect::FixedFunction), None);
    }

    #[test]
    fn fixed_function_has_no_glsl_program() {
        assert_eq!(
            validate(
                "void main() {}",
                ShaderDialect::FixedFunction,
                GlslStage::Vertex
            ),
            Err(ShaderError::NoGlsl)
        );
    }

    #[test]
    fn modern_glsl_is_rejected_even_with_the_right_version() {
        let source = "#version 120\nin vec2 position;\nvoid main() { gl_Position = vec4(position, 0.0, 1.0); }";
        assert_eq!(
            validate(source, ShaderDialect::Glsl120, GlslStage::Vertex),
            Err(ShaderError::ForbiddenToken("in"))
        );
    }

    #[test]
    fn texture_lod_is_rejected_but_texture2d_is_allowed() {
        assert_eq!(
            validate(
                "#version 120\nuniform sampler2D atlas;\nvoid main() { gl_FragColor = texture2D(atlas, vec2(0.0)); }",
                ShaderDialect::Glsl120,
                GlslStage::Fragment
            ),
            Ok(())
        );
        assert_eq!(
            validate(
                "#version 120\nuniform sampler2D atlas;\nvoid main() { gl_FragColor = textureLod(atlas, vec2(0.0), 0.0); }",
                ShaderDialect::Glsl120,
                GlslStage::Fragment
            ),
            Err(ShaderError::ForbiddenToken("textureLod"))
        );
    }

    #[test]
    fn version_must_match_the_selected_dialect() {
        let source = "#version 110\nattribute vec2 position;\nvoid main() { gl_Position = vec4(position, 0.0, 1.0); }";
        assert_eq!(
            validate(source, ShaderDialect::Glsl120, GlslStage::Vertex),
            Err(ShaderError::WrongVersion {
                found: "#version 110".to_owned(),
                expected: 120,
            })
        );
    }

    fn dialect_version_prefix(dialect: ShaderDialect) -> &'static str {
        match dialect {
            ShaderDialect::Glsl120 => "#version 120",
            ShaderDialect::Glsl110 => "#version 110",
            ShaderDialect::FixedFunction => unreachable!(),
        }
    }
}
