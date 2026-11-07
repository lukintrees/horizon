//! Qualify the context before attempting to load GL2-only entry points.
pub fn qualify(version: &str, renderer: &str) -> Result<(), String> {
    if renderer.to_ascii_lowercase().contains("gdi generic") {
        return Err("Microsoft GDI Generic is not a GL2 vendor ICD; check the installed driver and its x86/x64 architecture".into());
    }
    if version.starts_with("OpenGL ES") {
        return Err("expected desktop GL, not OpenGL ES".into());
    }
    let mut numbers = version.split_whitespace().next().unwrap_or("").split('.');
    let major = numbers.next().and_then(|n| n.parse::<u32>().ok());
    let minor = numbers.next().and_then(|n| n.parse::<u32>().ok());
    match (major, minor) {
        (Some(major), Some(_)) if major >= 2 => Ok(()),
        (Some(_), Some(_)) => Err(format!(
            "desktop OpenGL 2.0 required, driver reports {version}"
        )),
        _ => Err(format!("unrecognized desktop GL version: {version:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_desktop_driver_version_suffixes() {
        for version in [
            "2.0 Mesa 23.1.4",
            "2.1.8545 Release",
            "4.5 (Compatibility Profile) Mesa",
        ] {
            assert!(qualify(version, "Radeon X1650").is_ok());
        }
    }
    #[test]
    fn old_context_reports_the_requirement_not_a_missing_shader_symbol() {
        let error = qualify("1.5", "old driver").unwrap_err();
        assert!(error.contains("OpenGL 2.0 required"));
    }
    #[test]
    fn missing_icd_gles_and_malformed_versions_are_not_accepted() {
        for (version, renderer) in [
            ("1.1.0", "GDI Generic"),
            ("2.0", "Microsoft GDI Generic"),
            ("OpenGL ES 2.0 Mesa", "llvmpipe"),
            ("", "unknown"),
            ("2.x", "unknown"),
        ] {
            assert!(
                qualify(version, renderer).is_err(),
                "{version} / {renderer}"
            );
        }
    }
}
