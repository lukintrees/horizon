//! GLSL 110/120 program sources and static validation (PR 4, stage 1).
//!
//! The real `glCreateShader`/`glLinkProgram` calls live in the `gl21_clear`
//! example (PR 3, stage 2). This module is pure Rust, so the additive `gfx21`
//! scaffold stays free of the optional GL dependencies.

use std::error::Error;
use std::fmt;

use super::context::ShaderDialect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlslStage {
    Vertex,
    Fragment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderSource {
    pub name: &'static str,
    pub dialect: ShaderDialect,
    pub stage: GlslStage,
    pub source: &'static str,
}

pub const SMOKE_120_VERT: &str = include_str!("shaders/smoke_120.vert");
pub const SMOKE_120_FRAG: &str = include_str!("shaders/smoke_120.frag");
pub const SMOKE_110_VERT: &str = include_str!("shaders/smoke_110.vert");
pub const SMOKE_110_FRAG: &str = include_str!("shaders/smoke_110.frag");

pub const SHADERS: &[ShaderSource] = &[
    ShaderSource {
        name: "smoke_120.vert",
        dialect: ShaderDialect::Glsl120,
        stage: GlslStage::Vertex,
        source: SMOKE_120_VERT,
    },
    ShaderSource {
        name: "smoke_120.frag",
        dialect: ShaderDialect::Glsl120,
        stage: GlslStage::Fragment,
        source: SMOKE_120_FRAG,
    },
    ShaderSource {
        name: "smoke_110.vert",
        dialect: ShaderDialect::Glsl110,
        stage: GlslStage::Vertex,
        source: SMOKE_110_VERT,
    },
    ShaderSource {
        name: "smoke_110.frag",
        dialect: ShaderDialect::Glsl110,
        stage: GlslStage::Fragment,
        source: SMOKE_110_FRAG,
    },
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
    "gl_VertexID",
    "gl_InstanceID",
    "sampler2DArray",
    "image2D",
    "sampler3D",
    "uniform buffer",
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

/// Validate every shipped source; the example and CI call this before linking.
pub fn validate_all() -> Result<(), String> {
    for shader in SHADERS {
        validate(shader.source, shader.dialect, shader.stage)
            .map_err(|err| format!("{}: {err}", shader.name))?;
    }
    Ok(())
}

/// The smoke triangle's vertex/fragment pair for the selected dialect. F0 has
/// no GLSL program; its fixed-function path arrives with the full renderer.
pub fn smoke_program(dialect: ShaderDialect) -> Option<(&'static str, &'static str)> {
    match dialect {
        ShaderDialect::Glsl120 => Some((SMOKE_120_VERT, SMOKE_120_FRAG)),
        ShaderDialect::Glsl110 => Some((SMOKE_110_VERT, SMOKE_110_FRAG)),
        ShaderDialect::FixedFunction => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_source_validates() {
        assert_eq!(validate_all(), Ok(()));
        assert_eq!(SHADERS.len(), 4);
    }

    #[test]
    fn both_dialects_have_the_same_triangle_program() {
        let (v120, f120) = smoke_program(ShaderDialect::Glsl120).unwrap();
        let (v110, f110) = smoke_program(ShaderDialect::Glsl110).unwrap();
        assert!(v120.starts_with("#version 120"));
        assert!(v110.starts_with("#version 110"));
        assert!(f120.contains("gl_FragColor"));
        assert!(f110.contains("gl_FragColor"));
    }

    #[test]
    fn fixed_function_has_no_glsl_program() {
        assert_eq!(smoke_program(ShaderDialect::FixedFunction), None);
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
}
