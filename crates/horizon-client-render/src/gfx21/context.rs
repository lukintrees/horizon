//! Validated description of a legacy GL 2.1/2.0 context (PR 3).
//!
//! This is still pure Rust: it consumes the same [`Report`] that
//! `tools/gl21_caps.py` writes and decides which shader dialect and FBO entry
//! point the renderer may use. The real GLX/WGL `glGet*` queries, window and
//! glow loader now run in the `gl21_clear` example (PR 3, stage 2); nothing
//! here issues a GL call.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use super::caps::{FBO_REQUIREMENT, Report, Tier, classify};

/// OpenGL 2.1 is the main target; OpenGL 2.0 is the second shader variant.
pub const TARGET_GL: (u32, u32) = (2, 1);
pub const MIN_GL: (u32, u32) = (2, 0);
pub const TARGET_GLSL: u32 = 120;
pub const MIN_GLSL: u32 = 110;

/// Which source language the legacy programs are compiled from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderDialect {
    /// `#version 120`, the L1 path on Catalyst 9.3 / Mesa `r300g`.
    Glsl120,
    /// `#version 110`, the L0 path.
    Glsl110,
    /// No usable GLSL compiler: ARB programs or fixed function (F0).
    FixedFunction,
}

impl ShaderDialect {
    pub fn glsl_version(self) -> Option<u32> {
        match self {
            Self::Glsl120 => Some(TARGET_GLSL),
            Self::Glsl110 => Some(MIN_GLSL),
            Self::FixedFunction => None,
        }
    }

    pub fn version_directive(self) -> &'static str {
        match self {
            Self::Glsl120 => "#version 120\n",
            Self::Glsl110 => "#version 110\n",
            Self::FixedFunction => "",
        }
    }

    pub fn needs_glsl(self) -> bool {
        !matches!(self, Self::FixedFunction)
    }
}

/// Both extensions are alternatives; prefer the ARB form when both exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FboKind {
    Ext,
    Arb,
}

impl FboKind {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Ext => "GL_EXT_framebuffer_object",
            Self::Arb => "GL_ARB_framebuffer_object",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextError {
    UnsupportedTier(Tier),
    MissingVersion,
    MissingFbo,
    MissingGlsl,
}

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedTier(tier) => write!(f, "tier {} is not usable", tier.name()),
            Self::MissingVersion => write!(f, "GL_VERSION could not be read"),
            Self::MissingFbo => write!(f, "missing {FBO_REQUIREMENT}"),
            Self::MissingGlsl => write!(f, "GLSL version is missing for a shader tier"),
        }
    }
}

impl Error for ContextError {}

/// A classified, validated context. `report` stays public so PR 9 can log the
/// exact caps line without re-parsing strings.
#[derive(Clone, Debug)]
pub struct GlContext {
    pub report: Report,
    pub dialect: ShaderDialect,
    pub fbo: FboKind,
}

impl GlContext {
    /// Build from the four values a real context would query.
    pub fn new(
        version: Option<(u32, u32)>,
        glsl: Option<u32>,
        extensions: &[String],
        limits: &BTreeMap<String, u64>,
    ) -> Result<Self, ContextError> {
        Self::from_report(classify(version, glsl, extensions, limits.clone()))
    }

    pub fn from_report(report: Report) -> Result<Self, ContextError> {
        if !report.tier.usable() {
            return Err(ContextError::UnsupportedTier(report.tier));
        }
        if report.version.is_none() {
            return Err(ContextError::MissingVersion);
        }
        let dialect = match report.tier {
            Tier::L1 => ShaderDialect::Glsl120,
            Tier::L0 => ShaderDialect::Glsl110,
            Tier::F0 => ShaderDialect::FixedFunction,
            Tier::Unsupported => return Err(ContextError::UnsupportedTier(Tier::Unsupported)),
        };
        if dialect.needs_glsl() && report.glsl.is_none() {
            return Err(ContextError::MissingGlsl);
        }
        let fbo = if report
            .extensions
            .iter()
            .any(|name| name == "GL_ARB_framebuffer_object")
        {
            FboKind::Arb
        } else if report
            .extensions
            .iter()
            .any(|name| name == "GL_EXT_framebuffer_object")
        {
            FboKind::Ext
        } else {
            return Err(ContextError::MissingFbo);
        };
        Ok(Self {
            report,
            dialect,
            fbo,
        })
    }

    pub fn tier(&self) -> Tier {
        self.report.tier
    }

    pub fn max_texture_size(&self) -> Option<u64> {
        self.report.limits.get("GL_MAX_TEXTURE_SIZE").copied()
    }

    pub fn required_missing(&self) -> &[String] {
        &self.report.required_missing
    }

    pub fn problems(&self) -> Vec<String> {
        self.report.problems()
    }

    /// One-line startup diagnostics for PR 9; no driver calls here.
    pub fn startup_note(&self) -> String {
        let version = self
            .report
            .version
            .map(|(a, b)| format!("{a}.{b}"))
            .unwrap_or_else(|| "?".to_owned());
        let glsl = self
            .report
            .glsl
            .map(|v| v.to_string())
            .unwrap_or_else(|| "none".to_owned());
        format!(
            "legacy-gl: GL {version}, GLSL {glsl}, tier {}, dialect {:?}, FBO {}, max2d {:?}",
            self.report.tier.name(),
            self.dialect,
            self.fbo.extension(),
            self.max_texture_size()
        )
    }
}

/// Parse `GL_VERSION` ("2.1.8545 Release", "4.6 (Core Profile) Mesa 26") into (major, minor).
pub fn parse_gl_version(text: &str) -> Option<(u32, u32)> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|b| b.is_ascii_digit())?;
    let rest = &text[start..];
    let mut parts = rest.split(|c: char| !c.is_ascii_digit());
    let major = parts.next()?.parse::<u32>().ok()?;
    let after_major = &rest[major.to_string().len()..];
    if !after_major.starts_with('.') {
        return None;
    }
    let minor_text: String = after_major[1..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let minor = minor_text.parse::<u32>().ok()?;
    Some((major, minor))
}

/// Parse `GL_SHADING_LANGUAGE_VERSION` ("1.20", "1.10.8") into 120/110.
pub fn parse_glsl_version(text: &str) -> Option<u32> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|b| b.is_ascii_digit())?;
    let rest = &text[start..];
    let mut chars = rest.chars();
    let major = chars.next()?.to_digit(10)?;
    if chars.next() != Some('.') {
        return None;
    }
    let minor: String = chars.take_while(|c| c.is_ascii_digit()).collect();
    let minor = minor.parse::<u32>().ok()?;
    Some(major * 100 + minor.min(99))
}

/// `GL_EXTENSIONS` is a space-separated list; keep only GL_* tokens.
pub fn parse_extensions(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|name| name.starts_with("GL_"))
        .map(str::to_owned)
        .collect()
}

/// Classify the four values a real GL context reports, including limits.
pub fn classify_strings(
    version: &str,
    glsl: &str,
    extensions: &str,
    limits: BTreeMap<String, u64>,
) -> Result<GlContext, ContextError> {
    GlContext::new(
        parse_gl_version(version),
        parse_glsl_version(glsl),
        &parse_extensions(extensions),
        &limits,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx21::caps::{REQUIRED_BASE, REQUIRED_FBO};

    fn extensions() -> Vec<String> {
        let mut names: Vec<String> = REQUIRED_BASE
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        names.extend(REQUIRED_FBO.iter().map(|name| (*name).to_owned()));
        names
    }

    fn limits() -> BTreeMap<String, u64> {
        let mut map = BTreeMap::new();
        map.insert("GL_MAX_TEXTURE_SIZE".to_owned(), 4096);
        map
    }

    #[test]
    fn real_gl_strings_parse_into_a_validated_context() {
        let names = extensions().join(" ");
        let ctx = classify_strings("2.1.8545 Release", "1.20 Mesa", &names, limits()).unwrap();
        assert_eq!(ctx.tier(), Tier::L1);
        assert_eq!(ctx.dialect, ShaderDialect::Glsl120);
        assert_eq!(parse_gl_version("4.6 (Core Profile) Mesa 26"), Some((4, 6)));
        assert_eq!(parse_gl_version("not a version"), None);
        assert_eq!(parse_glsl_version("1.10.8"), Some(110));
        assert_eq!(parse_glsl_version("1.20"), Some(120));
        assert_eq!(parse_glsl_version("no glsl"), None);
        assert_eq!(
            parse_extensions("GL_ARB_multitexture junk GL_EXT_framebuffer_object").len(),
            2
        );
    }

    #[test]
    fn l1_selects_glsl_120_and_arb_fbo() {
        let ctx = GlContext::new(Some((2, 1)), Some(120), &extensions(), &limits()).unwrap();
        assert_eq!(ctx.tier(), Tier::L1);
        assert_eq!(ctx.dialect, ShaderDialect::Glsl120);
        assert_eq!(ctx.dialect.version_directive(), "#version 120\n");
        assert_eq!(ctx.fbo, FboKind::Arb);
        assert_eq!(ctx.max_texture_size(), Some(4096));
        assert!(ctx.problems().is_empty());
        assert!(ctx.startup_note().contains("tier L1"));
    }

    #[test]
    fn l0_selects_glsl_110() {
        let ctx = GlContext::new(Some((2, 0)), Some(110), &extensions(), &limits()).unwrap();
        assert_eq!(ctx.tier(), Tier::L0);
        assert_eq!(ctx.dialect, ShaderDialect::Glsl110);
        assert_eq!(ctx.dialect.glsl_version(), Some(110));
    }

    #[test]
    fn unsupported_caps_are_rejected() {
        let err = GlContext::new(Some((2, 1)), Some(120), &[], &limits()).unwrap_err();
        assert_eq!(err, ContextError::UnsupportedTier(Tier::Unsupported));
        assert!(err.to_string().contains("unsupported"));
    }

    #[test]
    fn f0_uses_fixed_function_without_glsl() {
        let ctx = GlContext::new(Some((2, 1)), None, &extensions(), &limits()).unwrap();
        assert_eq!(ctx.tier(), Tier::F0);
        assert_eq!(ctx.dialect, ShaderDialect::FixedFunction);
        assert!(!ctx.dialect.needs_glsl());
        assert_eq!(ctx.dialect.version_directive(), "");
    }
}
