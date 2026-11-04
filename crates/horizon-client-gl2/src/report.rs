//! The JSON report of a window run. CI reads it, so the field names are a contract.

use horizon_gl2::report::Check;

#[derive(Debug, Default)]
pub struct Report {
    pub gl_version: Option<String>,
    pub renderer: Option<String>,
    /// The window's client area in physical pixels, from the last frame.
    pub window: Option<(u32, u32)>,
    /// Frames presented to the screen.
    pub frames: u32,
    pub error: Option<String>,
    /// The gate-G1 checks of the last frame.
    pub checks: Vec<Check>,
}

impl Report {
    /// Passing needs no error, at least one frame, and every required check passing.
    pub fn passed(&self) -> bool {
        self.error.is_none()
            && self.frames > 0
            && !self.checks.is_empty()
            && self.checks.iter().all(|check| check.ok || !check.required)
    }

    pub fn to_json(&self) -> String {
        let value = serde_json::json!({
            "pass": self.passed(),
            "error": self.error,
            "gl_version": self.gl_version,
            "renderer": self.renderer,
            "window": self.window.map(|(width, height)| serde_json::json!({ "width": width, "height": height })),
            "frames": self.frames,
            "checks": self.checks.iter().map(|check| serde_json::json!({
                "name": check.name,
                "required": check.required,
                "ok": check.ok,
                "detail": check.detail,
            })).collect::<Vec<_>>(),
        });
        serde_json::to_string_pretty(&value).expect("a JSON value always serialises")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(name: &'static str, required: bool, ok: bool) -> Check {
        Check { name, required, ok, detail: String::new() }
    }

    #[test]
    fn a_run_needs_frames_and_passing_required_checks() {
        let mut report = Report::default();
        assert!(!report.passed(), "no frame, no checks");

        report.frames = 3;
        report.checks = vec![check("gate_g1_clear", true, true), check("optional", false, false)];
        assert!(report.passed(), "an optional check may fail");

        report.checks[0].ok = false;
        assert!(!report.passed(), "a required check failed");
        assert!(report.to_json().contains("\"pass\": false"));
    }

    #[test]
    fn an_error_fails_the_run_even_with_frames() {
        let report = Report {
            frames: 5,
            error: Some("SwapBuffers refused the swap".into()),
            checks: vec![check("gate_g1_clear", true, true)],
            ..Report::default()
        };
        assert!(!report.passed());
        assert!(report.to_json().contains("SwapBuffers refused the swap"));
    }
}
