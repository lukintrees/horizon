//! PR 9: startup report and debug `glGetError` bookkeeping.
//!
//! The report is built from the reference caps fixture and the PR 6 CPU frame
//! plan. It is deliberately independent of the GL context: the future GL layer
//! can push real `glGetError` codes into `DebugGlErrorLog` and reuse the same
//! budget checks. No GL calls are made by this module.

use super::atlas::MAX_SAMPLERS;
use super::frame::reference_frame;
use super::passes::{FramePlan, MAX_DRAWS};

pub const GL_NO_ERROR: u32 = 0x0000;
pub const GL_INVALID_ENUM: u32 = 0x0500;
pub const GL_INVALID_VALUE: u32 = 0x0501;
pub const GL_INVALID_OPERATION: u32 = 0x0502;
pub const GL_STACK_OVERFLOW: u32 = 0x0503;
pub const GL_STACK_UNDERFLOW: u32 = 0x0504;
pub const GL_OUT_OF_MEMORY: u32 = 0x0505;
pub const MAX_VRAM_MIB: u32 = 160;

/// Renderer methods the GL2 path has no modern equivalent for. The game loop
/// never calls them; the startup report and facade expose the list instead.
pub const UNAVAILABLE_METHODS: &[&str] = &[
    "set_shader_pack",
    "define_pack_macros",
    "set_hdr",
    "set_frame_generation",
    "set_gpu_culling",
    "queue_lods",
    "upload_lods",
    "set_pass_timing",
    "gpu_memory_by_label",
    "set_world_post",
    "set_texture_filtering",
    "set_mipmap_levels",
];

pub fn unavailable_methods() -> &'static [&'static str] {
    UNAVAILABLE_METHODS
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticReport {
    pub tier: &'static str,
    pub gl_version: String,
    pub glsl_version: String,
    pub renderer: String,
    pub vendor: String,
    pub draw_calls: u32,
    pub samplers: u32,
    pub vram_mib: u32,
    pub gl_errors: u32,
    pub disabled_features: Vec<&'static str>,
}

impl DiagnosticReport {
    pub fn from_plan(plan: &FramePlan) -> Self {
        Self {
            tier: "L1",
            gl_version: "2.1.8545 Release".to_owned(),
            glsl_version: "1.20".to_owned(),
            renderer: "Radeon X1600/X1650 Series".to_owned(),
            vendor: "ATI Technologies Inc.".to_owned(),
            draw_calls: plan.draw_call_count(),
            samplers: plan.atlas.sampler_count(),
            vram_mib: 48,
            gl_errors: 0,
            disabled_features: vec!["HDR", "SSBO", "compute culling", "texture arrays"],
        }
    }

    pub fn reference() -> Self {
        Self::from_plan(&reference_frame())
    }

    pub fn startup_lines(&self) -> Vec<String> {
        vec![
            format!("legacy-gl tier: {}", self.tier),
            format!("GL_VERSION: {}", self.gl_version),
            format!("GLSL_VERSION: {}", self.glsl_version),
            format!("GL_RENDERER: {}", self.renderer),
            format!("GL_VENDOR: {}", self.vendor),
            format!("draw calls: {}/{}", self.draw_calls, MAX_DRAWS),
            format!("samplers: {}/{}", self.samplers, MAX_SAMPLERS),
            format!("VRAM estimate: {}/{} MiB", self.vram_mib, MAX_VRAM_MIB),
            format!("glGetError count: {}", self.gl_errors),
            format!("disabled features: {}", self.disabled_features.join(", ")),
        ]
    }

    pub fn within_budget(&self) -> bool {
        self.draw_calls <= MAX_DRAWS
            && self.samplers <= MAX_SAMPLERS
            && self.vram_mib <= MAX_VRAM_MIB
            && self.gl_errors == 0
    }
}

pub fn gl_error_name(code: u32) -> &'static str {
    match code {
        GL_NO_ERROR => "GL_NO_ERROR",
        GL_INVALID_ENUM => "GL_INVALID_ENUM",
        GL_INVALID_VALUE => "GL_INVALID_VALUE",
        GL_INVALID_OPERATION => "GL_INVALID_OPERATION",
        GL_STACK_OVERFLOW => "GL_STACK_OVERFLOW",
        GL_STACK_UNDERFLOW => "GL_STACK_UNDERFLOW",
        GL_OUT_OF_MEMORY => "GL_OUT_OF_MEMORY",
        _ => "UNKNOWN_GL_ERROR",
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DebugGlErrorLog {
    pub codes: Vec<u32>,
}

impl DebugGlErrorLog {
    pub fn record(&mut self, code: u32) {
        self.codes.push(code);
    }

    pub fn has_error(&self) -> bool {
        self.codes.iter().any(|code| *code != GL_NO_ERROR)
    }

    pub fn last(&self) -> Option<u32> {
        self.codes.last().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_report_has_explicit_caps_and_budget() {
        let report = DiagnosticReport::reference();
        assert_eq!(report.tier, "L1");
        assert_eq!(report.gl_version, "2.1.8545 Release");
        assert_eq!(report.glsl_version, "1.20");
        assert_eq!(report.renderer, "Radeon X1600/X1650 Series");
        assert_eq!(report.vendor, "ATI Technologies Inc.");
        assert_eq!(report.draw_calls, 5);
        assert_eq!(report.samplers, 4);
        assert_eq!(report.vram_mib, 48);
        assert_eq!(report.gl_errors, 0);
        assert_eq!(report.disabled_features.len(), 4);
        assert!(report.within_budget());
    }

    #[test]
    fn startup_lines_are_deterministic() {
        let lines = DiagnosticReport::reference().startup_lines();
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0], "legacy-gl tier: L1");
        assert_eq!(lines[5], "draw calls: 5/200");
        assert_eq!(lines[6], "samplers: 4/4");
        assert_eq!(lines[7], "VRAM estimate: 48/160 MiB");
        assert!(lines[9].contains("HDR"));
    }

    #[test]
    fn unavailable_methods_are_explicit_and_stable() {
        assert_eq!(UNAVAILABLE_METHODS.len(), 12);
        assert!(unavailable_methods().contains(&"set_shader_pack"));
        assert!(unavailable_methods().contains(&"gpu_memory_by_label"));
    }

    #[test]
    fn error_names_and_debug_log_are_explicit() {
        assert_eq!(gl_error_name(GL_NO_ERROR), "GL_NO_ERROR");
        assert_eq!(gl_error_name(GL_INVALID_OPERATION), "GL_INVALID_OPERATION");
        assert_eq!(gl_error_name(0xDEAD), "UNKNOWN_GL_ERROR");
        let mut log = DebugGlErrorLog::default();
        log.record(GL_NO_ERROR);
        assert!(!log.has_error());
        log.record(GL_INVALID_OPERATION);
        assert!(log.has_error());
        assert_eq!(log.last(), Some(GL_INVALID_OPERATION));
    }

    #[test]
    fn excess_draws_or_errors_break_the_budget() {
        let mut report = DiagnosticReport::reference();
        report.draw_calls = 201;
        assert!(!report.within_budget());
        report.draw_calls = 5;
        report.gl_errors = 1;
        assert!(!report.within_budget());
    }
}
