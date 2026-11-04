//! Reading the `GL_VERSION` string, to refuse drivers that cannot run the frame.

/// The lowest version the frame runs on: GLSL 1.20 needs OpenGL 2.1.
pub const FRAME_MINIMUM: (u32, u32) = (2, 1);

/// `(major, minor)` from a `GL_VERSION` string such as `2.1 Mesa 26.0.8`, `4.6.0 NVIDIA 550.1`
/// or `1.1.0`. `None` when the string does not start with a version.
pub fn parse(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.split_whitespace().next()?.split('.');
    let major = leading_number(parts.next()?)?;
    let minor = leading_number(parts.next().unwrap_or("0"))?;
    Some((major, minor))
}

/// Whether a driver's `GL_VERSION` string is at least `FRAME_MINIMUM`.
pub fn runs_frame(text: &str) -> bool {
    parse(text).is_some_and(|version| version >= FRAME_MINIMUM)
}

fn leading_number(text: &str) -> Option<u32> {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_versions_drivers_report() {
        assert_eq!(parse("2.1 Mesa 26.0.8"), Some((2, 1)));
        assert_eq!(parse("4.6.0 NVIDIA 550.1"), Some((4, 6)));
        assert_eq!(parse("1.1.0"), Some((1, 1)));
        assert_eq!(parse("3.0"), Some((3, 0)));
        assert_eq!(parse("2.1 (Compatibility Profile)"), Some((2, 1)));
        assert_eq!(parse(""), None);
        assert_eq!(parse("Mesa"), None);
    }

    #[test]
    fn only_2_1_and_later_run_the_frame() {
        assert!(runs_frame("2.1 Mesa 26.0.8"));
        assert!(runs_frame("2.10 something"));
        assert!(!runs_frame("2.0 Mesa"));
        assert!(!runs_frame("1.1.0 GDI Generic"));
        assert!(!runs_frame("garbage"));
    }
}
