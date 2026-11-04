//! The one error type of the legacy path.
//!
//! There is no `Error: From<Everything>` here on purpose. Every failure on this path is
//! something a player can be told about in a sentence ("glVertexAttribDivisor is missing on
//! your GPU driver: install the latest driver for your ATI Radeon X1650"), and the sentence
//! is the error. A driver that dies without a message leaves a log line, not an error
//! (docs/gl21-dev-env.md §5).

use std::fmt;

/// A failure the renderer reports to the game: never a panic, because a panic in a GL
/// callback or on the render thread takes the process down with no message on Windows 7.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The machine cannot run the legacy path at all (no GL library, no context, too old).
    /// Carries the sentence shown to the player.
    Unsupported(String),
    /// A driver call failed. `call` is the GL entry point, so the log can name it.
    Gl { call: &'static str, message: String },
    /// A shader did not compile or link. `log` is the driver's own text, which on old ATI
    /// drivers quotes the line, so the source is attached separately by `shader.rs`.
    Shader { stage: &'static str, log: String },
    /// A shader or a pass asks for something outside the target's budget (512 uniform
    /// components, 44 varying floats, 16 samplers, no vertex textures). Strict mode turns
    /// what the driver would silently accept into this.
    Budget { what: String, detail: String },
    /// A pass asked for a [`crate::caps::Feature`] the context does not have.
    Missing(crate::caps::Feature),
    /// The renderer was used after the device was lost (driver reset, TDR on Windows).
    DeviceLost,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported(msg) => write!(f, "{msg}"),
            Error::Gl { call, message } => write!(f, "{call} failed: {message}"),
            Error::Shader { stage, log } => write!(f, "{stage} shader: {log}"),
            Error::Budget { what, detail } => write!(f, "over the GL 2.1 budget: {what}: {detail}"),
            Error::Missing(feature) => write!(
                f,
                "{} is not available on this GPU ({}): {}",
                feature_name(*feature),
                feature.source(),
                feature.why()
            ),
            Error::DeviceLost => write!(f, "the graphics device was lost"),
        }
    }
}

fn feature_name(feature: crate::caps::Feature) -> String {
    let debug = format!("{feature:?}");
    let mut out = String::new();
    for (i, c) in debug.char_indices() {
        if c.is_uppercase() && i > 0 {
            out.push(' ');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

impl std::error::Error for Error {}

impl Error {
    /// A GL call failed; the caller passes the entry point's name.
    pub fn gl(call: &'static str, message: impl Into<String>) -> Self {
        Error::Gl {
            call,
            message: message.into(),
        }
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Error::Unsupported(message.into())
    }

    pub fn budget(what: impl Into<String>, detail: impl Into<String>) -> Self {
        Error::Budget {
            what: what.into(),
            detail: detail.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::Feature;

    #[test]
    fn a_missing_feature_says_what_and_where() {
        let e = Error::Missing(Feature::UniformBuffers);
        let s = e.to_string();
        assert!(s.contains("uniform buffers"), "{s}");
        assert!(s.contains("GL_ARB_uniform_buffer_object"), "{s}");
    }

    #[test]
    fn a_budget_failure_says_which_budget() {
        let e = Error::budget("terrain program", "936 uniform components, limit 512");
        assert!(e.to_string().contains("936"));
    }
}
