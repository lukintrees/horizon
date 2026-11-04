//! Capability classification for the legacy OpenGL 2.1/2.0 renderer.
//!
//! This is a pure-Rust mirror of `tools/gl21_caps.py`: no GL calls, no `wgpu`,
//! no `naga`, no window handles. PR 3 (`context.rs`) fills `Strings`, `Limits`
//! and `Extensions` from a real GLX/WGL context and calls [`classify`]. Keeping
//! the rules here means the dev tool, the renderer and CI agree on what L1/L0/F0
//! mean, and this module can be added to the default build without changing it.

use std::collections::{BTreeMap, BTreeSet};

/// The absolute floor for the F0 fixed-function / ARB-program profile.
pub const REQUIRED_BASE: &[&str] = &[
    "GL_ARB_vertex_buffer_object",
    "GL_ARB_multitexture",
    "GL_ARB_texture_non_power_of_two",
];
/// Either extension supplies offscreen colour and depth targets.
pub const REQUIRED_FBO: &[&str] = &["GL_EXT_framebuffer_object", "GL_ARB_framebuffer_object"];
/// How a missing FBO extension is reported (the two are alternatives).
pub const FBO_REQUIREMENT: &str = "GL_EXT_framebuffer_object|GL_ARB_framebuffer_object";

/// Useful, but the renderer degrades when one is missing.
pub const OPTIONAL: &[&str] = &[
    "GL_ARB_depth_texture",
    "GL_ARB_framebuffer_object",
    "GL_ARB_map_buffer_range",
    "GL_ARB_occlusion_query",
    "GL_ARB_pixel_buffer_object",
    "GL_ARB_shader_texture_lod",
    "GL_ARB_shading_language_100",
    "GL_ARB_shadow",
    "GL_ARB_texture_cube_map",
    "GL_ARB_vertex_array_object",
    "GL_EXT_blend_func_separate",
    "GL_EXT_framebuffer_blit",
    "GL_EXT_framebuffer_multisample",
    "GL_EXT_multi_draw_arrays",
    "GL_EXT_packed_depth_stencil",
    "GL_EXT_texture_filter_anisotropic",
    "GL_EXT_texture_sRGB",
];

/// `glGetIntegerv` values the legacy tier wants to log. A missing one is a
/// note, not a failure: the real query happens in PR 3.
pub const EXPECTED_LIMITS: &[&str] = &[
    "GL_MAX_TEXTURE_SIZE",
    "GL_MAX_TEXTURE_IMAGE_UNITS",
    "GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS",
    "GL_MAX_VERTEX_ATTRIBS",
    "GL_MAX_VERTEX_UNIFORM_COMPONENTS",
    "GL_MAX_VARYING_FLOATS",
    "GL_MAX_DRAW_BUFFERS",
    "GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// OpenGL 2.1 / GLSL 1.20: the main legacy path.
    L1,
    /// OpenGL 2.0 / GLSL 1.10: the second shader variant.
    L0,
    /// No usable GLSL compiler: fixed-function / ARB-program fallback.
    F0,
    /// A required extension is missing; do not start the legacy renderer.
    Unsupported,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Self::L1 => "L1",
            Self::L0 => "L0",
            Self::F0 => "F0",
            Self::Unsupported => "unsupported",
        }
    }

    pub fn usable(self) -> bool {
        !matches!(self, Self::Unsupported)
    }
}

#[derive(Clone, Debug)]
pub struct Report {
    pub version: Option<(u32, u32)>,
    pub glsl: Option<u32>,
    pub extensions: Vec<String>,
    pub limits: BTreeMap<String, u64>,
    pub required_missing: Vec<String>,
    pub optional_present: Vec<String>,
    pub optional_missing: Vec<String>,
    pub notes: Vec<String>,
    pub tier: Tier,
}

impl Report {
    /// Problems that make `--check` / a startup check fail.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if !self.tier.usable() {
            problems.push(format!("tier {} is not usable", self.tier.name()));
        }
        for name in &self.required_missing {
            problems.push(format!("missing required extension {name}"));
        }
        problems
    }
}

/// Choose the legacy tier from what the driver reported.
///
/// `version` is `(major, minor)` from `GL_VERSION`, `glsl` is `major * 100 +
/// minor` from `GL_SHADING_LANGUAGE_VERSION` (`1.20` -> `120`).
pub fn classify(
    version: Option<(u32, u32)>,
    glsl: Option<u32>,
    extensions: &[String],
    limits: BTreeMap<String, u64>,
) -> Report {
    let set: BTreeSet<&str> = extensions.iter().map(String::as_str).collect();

    let mut required_missing = Vec::new();
    for name in REQUIRED_BASE {
        if !set.contains(name) {
            required_missing.push((*name).to_owned());
        }
    }
    if !REQUIRED_FBO.iter().any(|name| set.contains(name)) {
        required_missing.push(FBO_REQUIREMENT.to_owned());
    }

    let mut optional_present = Vec::new();
    let mut optional_missing = Vec::new();
    for name in OPTIONAL {
        if set.contains(name) {
            optional_present.push((*name).to_owned());
        } else {
            optional_missing.push((*name).to_owned());
        }
    }

    let mut notes = Vec::new();
    if !set.contains("GL_ARB_shading_language_100") {
        notes.push(
            "GL_ARB_shading_language_100 not listed; GLSL version string is authoritative"
                .to_owned(),
        );
    }
    for name in EXPECTED_LIMITS {
        if !limits.contains_key(*name) {
            notes.push(format!("limit {name} not reported; query it at runtime"));
        }
    }
    if version.is_none() {
        notes.push("GL_VERSION could not be parsed".to_owned());
    }

    let tier = if !required_missing.is_empty() {
        Tier::Unsupported
    } else {
        match (version, glsl) {
            (Some((major, minor)), Some(lang)) if (major, minor) >= (2, 1) && lang >= 120 => {
                Tier::L1
            }
            (Some((major, minor)), Some(lang)) if (major, minor) >= (2, 0) && lang >= 110 => {
                Tier::L0
            }
            _ => Tier::F0,
        }
    };

    Report {
        version,
        glsl,
        extensions: extensions.to_vec(),
        limits,
        required_missing,
        optional_present,
        optional_missing,
        notes,
        tier,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extensions() -> Vec<String> {
        [
            "GL_ARB_vertex_buffer_object",
            "GL_ARB_multitexture",
            "GL_ARB_texture_non_power_of_two",
            "GL_EXT_framebuffer_object",
            "GL_ARB_framebuffer_object",
            "GL_EXT_multi_draw_arrays",
            "GL_ARB_shader_texture_lod",
            "GL_ARB_shading_language_100",
        ]
        .iter()
        .map(|name| (*name).to_owned())
        .collect()
    }

    fn limits() -> BTreeMap<String, u64> {
        let mut limits = BTreeMap::new();
        for name in EXPECTED_LIMITS {
            limits.insert((*name).to_owned(), 4096);
        }
        limits
    }

    #[test]
    fn catalyst_93_reference_is_l1() {
        let report = classify(Some((2, 1)), Some(120), &extensions(), limits());
        assert_eq!(report.tier, Tier::L1);
        assert!(report.required_missing.is_empty());
        assert!(report.problems().is_empty());
        assert!(
            report
                .optional_present
                .contains(&"GL_EXT_multi_draw_arrays".to_owned())
        );
        assert!(report.notes.is_empty());
    }

    #[test]
    fn gl_20_with_glsl_110_is_l0() {
        let report = classify(Some((2, 0)), Some(110), &extensions(), limits());
        assert_eq!(report.tier, Tier::L0);
    }

    #[test]
    fn missing_fbo_is_unsupported() {
        let extensions: Vec<String> = extensions()
            .into_iter()
            .filter(|name| !REQUIRED_FBO.contains(&name.as_str()))
            .collect();
        let report = classify(Some((2, 1)), Some(120), &extensions, limits());
        assert_eq!(report.tier, Tier::Unsupported);
        assert!(
            report
                .required_missing
                .contains(&FBO_REQUIREMENT.to_owned())
        );
        assert!(report.problems().len() >= 2);
    }

    #[test]
    fn no_usable_glsl_falls_back_to_f0() {
        let report = classify(Some((1, 5)), None, &extensions(), limits());
        assert_eq!(report.tier, Tier::F0);
    }
}
