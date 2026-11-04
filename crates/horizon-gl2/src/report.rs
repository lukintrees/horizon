//! What the probe found, and its JSON form (written by hand: no serde in this crate).

use std::fmt::Write;

/// One thing the probe checked. Required checks decide the exit status; the others inform.
#[derive(Debug, Clone)]
pub struct Check {
    pub name: &'static str,
    pub required: bool,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Default)]
pub struct Report {
    pub gl_version: String,
    pub glsl_version: String,
    pub renderer: String,
    pub vendor: String,
    pub extensions: Vec<String>,
    pub limits: Vec<(&'static str, i32)>,
    pub checks: Vec<Check>,
    pub first_pixel: [u8; 4],
    pub pixels_checked: u32,
    pub pixels_mismatched: u32,
}

impl Report {
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|check| !check.required || check.ok)
    }

    /// True when the renderer is a CPU rasteriser (llvmpipe, softpipe, swrast...), so the
    /// numbers say nothing about a real GPU.
    pub fn software_renderer(&self) -> bool {
        let name = self.renderer.to_ascii_lowercase();
        ["llvmpipe", "softpipe", "swrast", "software"].iter().any(|word| name.contains(word))
    }

    pub fn to_json(&self) -> String {
        let mut out = String::new();
        out.push_str("{\n");
        let _ = writeln!(out, "  \"pass\": {},", self.passed());
        let _ = writeln!(out, "  \"software_renderer\": {},", self.software_renderer());
        let _ = writeln!(out, "  \"gl_version\": {},", quote(&self.gl_version));
        let _ = writeln!(out, "  \"glsl_version\": {},", quote(&self.glsl_version));
        let _ = writeln!(out, "  \"renderer\": {},", quote(&self.renderer));
        let _ = writeln!(out, "  \"vendor\": {},", quote(&self.vendor));
        out.push_str("  \"limits\": {");
        for (i, (name, value)) in self.limits.iter().enumerate() {
            let sep = if i == 0 { "" } else { "," };
            let _ = write!(out, "{sep}\n    {}: {value}", quote(name));
        }
        out.push_str("\n  },\n");
        out.push_str("  \"extensions\": [");
        for (i, extension) in self.extensions.iter().enumerate() {
            let sep = if i == 0 { "" } else { "," };
            let _ = write!(out, "{sep}\n    {}", quote(extension));
        }
        out.push_str("\n  ],\n");
        out.push_str("  \"checks\": [");
        for (i, check) in self.checks.iter().enumerate() {
            let sep = if i == 0 { "" } else { "," };
            let _ = write!(
                out,
                "{sep}\n    {{\"name\": {}, \"required\": {}, \"ok\": {}, \"detail\": {}}}",
                quote(check.name),
                check.required,
                check.ok,
                quote(&check.detail)
            );
        }
        out.push_str("\n  ],\n");
        let [r, g, b, a] = self.first_pixel;
        let _ = writeln!(out, "  \"first_pixel\": [{r}, {g}, {b}, {a}],");
        let _ = writeln!(out, "  \"pixels_checked\": {},", self.pixels_checked);
        let _ = writeln!(out, "  \"pixels_mismatched\": {}", self.pixels_mismatched);
        out.push_str("}\n");
        out
    }

    /// The same facts for a person reading the CI log.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "GL {} | GLSL {} | {} ({})", self.gl_version, self.glsl_version, self.renderer, self.vendor);
        if self.software_renderer() {
            let _ = writeln!(out, "note: software renderer, not a GPU: results show what the API allows, not speed");
        }
        for check in &self.checks {
            let mark = match (check.ok, check.required) {
                (true, _) => "ok  ",
                (false, true) => "FAIL",
                (false, false) => "info",
            };
            let _ = writeln!(out, "[{mark}] {}: {}", check.name, check.detail);
        }
        let _ = writeln!(out, "result: {}", if self.passed() { "PASS" } else { "FAIL" });
        out
    }
}

/// A JSON string literal, escaped.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The leading `major.minor` of a version string: `"2.1 Mesa 25.2"` and `"1.20"` both work.
pub fn parse_version(text: &str) -> Option<(u32, u32)> {
    let token = text.split_whitespace().find(|t| t.starts_with(|c: char| c.is_ascii_digit()))?;
    let mut parts = token.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor: String = parts.next()?.chars().take_while(|c| c.is_ascii_digit()).collect();
    Some((major, minor.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_driver_version_strings() {
        assert_eq!(parse_version("2.1 Mesa 25.2.0"), Some((2, 1)));
        assert_eq!(parse_version("1.20"), Some((1, 20)));
        assert_eq!(parse_version("4.5 (Compatibility Profile) Mesa 24.0"), Some((4, 5)));
        assert_eq!(parse_version("3.30 NVIDIA"), Some((3, 30)));
        assert_eq!(parse_version("no version here"), None);
    }

    #[test]
    fn versions_compare_by_major_then_minor() {
        assert!((2, 1) >= (2, 0));
        assert!((1, 20) >= (1, 10));
        assert!(!((1, 9) >= (1, 10)), "1.9 is older than 1.10");
        assert!(!((2, 0) >= (2, 1)));
    }

    #[test]
    fn json_escapes_driver_strings() {
        assert_eq!(quote("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
    }

    #[test]
    fn llvmpipe_is_reported_as_software() {
        let report = Report { renderer: "llvmpipe (LLVM 17.0.6, 256 bits)".into(), ..Default::default() };
        assert!(report.software_renderer());
        let gpu = Report { renderer: "ATI Radeon X1650".into(), ..Default::default() };
        assert!(!gpu.software_renderer());
    }
}
