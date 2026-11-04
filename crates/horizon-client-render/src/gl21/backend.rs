//! Where the GL 2.1 path's function pointers come from, and how the window's context is asked for.
//!
//! Two things Phase 1 of `docs/opengl21-plan.md` needs before a single pixel: a context that is
//! OpenGL **2.1 compatibility** (`glutin`, in `gl21::backend`) and the hundreds of function pointers
//! `glow` calls. This module is the second, and it is written so that everything but the loading
//! itself is decided here, without a driver:
//!
//! - **The profile to ask for.** GLX and WGL both take a request and answer with whatever they can
//!   give, and the driver's answer is a *higher* version more often than a lower one: asking for 2.1
//!   on a machine with 4.6 gets a 4.6 core-ish compatibility context, whose vertex-array objects and
//!   glslang front end behave differently from the r300g the path is written for. So the request is
//!   built here, with a version mask, and the answer is only accepted when its version matches what
//!   the profile says — the same rule `HORIZON_GL_PROFILE` uses, and the reason a run on a modern
//!   machine measures the old one.
//! - **The loader.** `glutin` hands over a `load_with`-style function already; on a driver that only
//!   has GLX 1.2 (`R500` under old Mesa), `glXGetProcAddressARB` returns `NULL` for core 1.1 names
//!   such as `glGetString`. The fallback below is explicit about it, so the failure on a 2006 driver
//!   is a sentence rather than a null call three frames in.
//!
//! The loading itself needs `unsafe` (a function pointer and a C string), and
//! `horizon-client-render` forbids unsafe code, so it lives behind [`Loader`]: this crate names the
//! symbols and decides what to do when one is missing, and a crate that may write `unsafe` provides
//! the pointer.

use crate::gl21::Capabilities;

/// The GL version the GL 2.1 path asks a window system for.
///
/// `major`/`minor` are the request; `compatibility` is glutin's `ContextApi::OpenGl` profile, whose
/// `None` on a 2.1 request is what a 2006 driver needs (a *core* profile is 3.2 and later only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub major: u8,
    pub minor: u8,
    /// Whether the answer's version is allowed to be newer than the request. It is not: see the
    /// module's note on why a 4.6 context is not the same renderer as a 2.1 one.
    pub accept_newer: bool,
    /// Whether the answer must be a compatibility profile, as opposed to a core one.
    pub compatibility: bool,
}

impl Request {
    /// What the X1650 gets: OpenGL 2.1, no profile mask (2.1 has none), compatibility by default.
    pub const X1650: Request = Request {
        major: 2,
        minor: 1,
        accept_newer: false,
        compatibility: true,
    };

    /// The request for `HORIZON_GL_PROFILE=x1650` (`gl21::selected`), else the best the driver has.
    ///
    /// The second case is not a fallback for players: a machine without the card that asks for the
    /// GL 2.1 path should get a context the path can run in, and a driver that offers 3.3 core is one
    /// it can run in (the shaders are 1.20, and every 3.3 driver still compiles those).
    pub fn for_profile(profile: Option<crate::gl21::Profile>) -> Request {
        match profile {
            Some(profile) => Request {
                major: profile.gl.0,
                minor: profile.gl.1,
                ..Request::X1650
            },
            None => Request {
                accept_newer: true,
                ..Request::X1650
            },
        }
    }

    /// Whether a context that answered with `gl` is the one this request asked for.
    pub fn accepts(&self, gl: (u8, u8)) -> bool {
        if self.accept_newer {
            gl >= (self.major, self.minor)
        } else {
            gl == (self.major, self.minor)
        }
    }

    /// What to tell a player whose window system answered with something else.
    pub fn refusal(&self, gl: (u8, u8)) -> String {
        format!(
            "Horizon's OpenGL 2.1 renderer asked this driver for OpenGL {}.{} and got {}.{}: the \
             renderer is written for the card's own driver (r300g or Catalyst 9.3), and a newer \
             context drives the card differently. Start with HORIZON_GL_PROFILE unset to use the \
             driver's own version.",
            self.major, self.minor, gl.0, gl.1
        )
    }
}

/// A GL function, by name, the way a window system hands one over.
pub type Proc = Option<extern "C" fn()>;

/// Where the GL entry points come from: the caller supplies the loading, this module decides what
/// may be missing.
///
/// The one method is the whole contract: a name in, a function pointer out, `None` when the driver
/// has no such entry point. `glutin`'s `Display::get_proc_address` closure is exactly this.
pub trait Loader {
    /// The pointer for `name`, or `None`.
    fn proc(&self, name: &str) -> Proc;
}

/// A function pointer from `loader`, or a sentence naming what the driver is missing.
///
/// GL 1.1's own entry points (`glGetString`, `glTexImage2D`, …) are exported by the GL library
/// itself and may not be reachable through `wglGetProcAddress`/`glXGetProcAddress`: a loader that
/// returns `None` for one of them is not broken, and the caller has to know that the fallback is to
/// look in the library's own symbols. Which names are in that set is fixed for 1.1, so it is a
/// constant here — the alternative is a null call, which on a 2006 driver is a silent crash.
pub fn require(loader: &dyn Loader, name: &str) -> Result<extern "C" fn(), String> {
    match loader.proc(name) {
        Some(proc) => Ok(proc),
        None if core_1_1(name) => Err(format!(
            "{name} is an OpenGL 1.1 entry point and the window system returned no pointer for it: \
             load it from the GL library's own symbols (libGL or opengl32) rather than through \
             glXGetProcAddress/wglGetProcAddress"
        )),
        None => Err(format!(
            "this driver has no {name}: it is newer than the context, or the renderer asked for a \
             feature the card does not have"
        )),
    }
}

/// Whether `name` is one of the OpenGL 1.1 entry points, which a GL library exports itself.
///
/// The list is the 1.1 functions the renderer calls (the rest of 1.1 is unused by `glow`'s GL 2.1
/// path), plus the two the context check reads first: `glGetString` for the version and
/// `glGetError` for everything else.
pub fn core_1_1(name: &str) -> bool {
    const NAMES: [&str; 24] = [
        "glGetString",
        "glGetError",
        "glGetIntegerv",
        "glEnable",
        "glDisable",
        "glClear",
        "glClearColor",
        "glViewport",
        "glScissor",
        "glBlendFunc",
        "glDepthFunc",
        "glDepthMask",
        "glTexImage2D",
        "glTexSubImage2D",
        "glTexParameteri",
        "glTexEnvi",
        "glBindTexture",
        "glGenTextures",
        "glDeleteTextures",
        "glDrawArrays",
        "glDrawElements",
        "glReadPixels",
        "glPixelStorei",
        "glFinish",
    ];
    NAMES.contains(&name)
}

/// What a context answered, in the terms `gl21::Capabilities` asks: the strings `glGetString`
/// returns, parsed.
///
/// Kept as data — the strings and the two integers a context reports — so that a test can build the
/// answer of a driver it has never seen, which is the whole point of checking the checklist in CI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reported<'a> {
    pub version: &'a str,
    pub glsl_version: &'a str,
    pub max_texture_size: u32,
    pub max_vertex_attributes: u32,
    pub max_texture_units: u32,
}

/// `"4.6.0 NVIDIA 535.104.05"` as `(4, 6)`, `"2.1 Mesa 7.11"` as `(2, 1)`, `"1.4 (2.1 Mesa)"` as
/// `(1, 4)`: the first two numbers of the string GL_VERSION starts with. `None` when the driver
/// reports nothing that looks like a version, which is what the message to the player has to say.
pub fn parse_version(version: &str) -> Option<(u8, u8)> {
    let mut numbers = version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u8>().ok());
    let major = numbers.next()?;
    let minor = numbers.next()?;
    Some((major, minor))
}

/// `"1.20"` as `120`, `"1.20 Mesa"` as `120`, `"4.60 NVIDIA"` as `460`: `GL_SHADING_LANGUAGE_VERSION`
/// in hundreds.
pub fn parse_glsl(version: &str) -> Option<u16> {
    let (major, minor) = parse_version(version)?;
    Some(u16::from(major) * 100 + u16::from(minor) * 10)
}

/// The checklist for a context that reported `reported` and lists `extensions`.
///
/// The extensions are the ones the checklist asks about; a driver that lists none still has what
/// its version made core (2.0's draw buffers, 1.5's occlusion queries, 1.4's LOD bias), which is why
/// the version goes in beside them.
pub fn capabilities(reported: Reported<'_>, extensions: &[&str]) -> Result<Capabilities, String> {
    let version = parse_version(reported.version).ok_or_else(|| {
        format!(
            "this driver reports no readable OpenGL version ({:?}): Horizon cannot tell what it can \
             run",
            reported.version
        )
    })?;
    let glsl = parse_glsl(reported.glsl_version).ok_or_else(|| {
        format!(
            "this driver reports no readable GLSL version ({:?})",
            reported.glsl_version
        )
    })?;
    let has = |name: &str| extensions.iter().any(|list| *list == name);
    Ok(Capabilities {
        gl: version,
        glsl,
        max_texture_size: reported.max_texture_size,
        max_vertex_attributes: reported.max_vertex_attributes,
        max_texture_units: reported.max_texture_units,
        framebuffer_objects: has("GL_ARB_framebuffer_object") || version >= (3, 0),
        draw_buffers: has("GL_ARB_draw_buffers") || version >= (2, 0),
        occlusion_query: has("GL_ARB_occlusion_query") || version >= (1, 5),
        non_power_of_two: has("GL_ARB_texture_non_power_of_two"),
        texture_lod_bias: has("GL_EXT_texture_lod_bias") || version >= (1, 4),
    })
}

/// The whole Phase 1 check, in the order the renderer opens: a context's request, then what its
/// driver answered, as the lines for the log and the sentences for the player. `Ok` is what to log
/// before the first frame; `Err` is why the window closes.
///
/// A `wanted` line is not a failure (see `Capabilities::wanted`), so the log is the union of the two
/// lists and the failure is exactly `missing`.
pub fn open(
    request: Request,
    reported: Reported<'_>,
    extensions: &[&str],
) -> Result<Vec<String>, String> {
    let capabilities = capabilities(reported, extensions)?;
    if !request.accepts(capabilities.gl) {
        return Err(request.refusal(capabilities.gl));
    }
    let missing = capabilities.missing();
    if !missing.is_empty() {
        let mut message = String::from(
            "Horizon's OpenGL 2.1 renderer cannot start on this driver:\n",
        );
        for line in &missing {
            message.push_str(&format!("  - {line}\n"));
        }
        message.push_str(
            "Install the newest graphics driver your card and operating system allow; the renderer \
             needs nothing a 2007 driver does not have.",
        );
        return Err(message);
    }
    let mut log = vec![format!(
        "Horizon: OpenGL {}.{}, GLSL {} (asked for {}.{}), {} texels, {} vertex attributes, {} \
         texture units",
        capabilities.gl.0,
        capabilities.gl.1,
        crate::gl21::glsl_name(capabilities.glsl),
        request.major,
        request.minor,
        capabilities.max_texture_size,
        capabilities.max_vertex_attributes,
        capabilities.max_texture_units
    )];
    log.extend(capabilities.wanted());
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A loader that knows `names` and nothing else.
    struct Known(Vec<&'static str>);

    impl Loader for Known {
        fn proc(&self, name: &str) -> Proc {
            if self.0.contains(&name) {
                Some(nothing)
            } else {
                None
            }
        }
    }

    extern "C" fn nothing() {}

    /// The X1650's report: r300g on Mesa 7.11, which is what the target machine runs.
    const X1650: Reported<'static> = Reported {
        version: "2.1 Mesa 7.11",
        glsl_version: "1.20",
        max_texture_size: 4096,
        max_vertex_attributes: 16,
        max_texture_units: 16,
    };

    const X1650_EXTENSIONS: [&str; 3] = [
        "GL_ARB_framebuffer_object",
        "GL_ARB_occlusion_query",
        "GL_ARB_draw_buffers",
    ];

    #[test]
    fn the_target_drivers_strings_parse_and_pass() {
        let capabilities = capabilities(X1650, &X1650_EXTENSIONS).expect("a plain report");
        assert_eq!(capabilities.gl, (2, 1));
        assert_eq!(capabilities.glsl, 120);
        assert!(capabilities.framebuffer_objects);
        assert!(capabilities.draw_buffers, "core in 2.0 as well");
        assert!(capabilities.occlusion_query, "core in 1.5 as well");
        assert!(capabilities.enough(), "{:?}", capabilities.missing());
        assert_eq!(capabilities, Capabilities::X1650);
    }

    #[test]
    fn a_context_of_the_right_version_is_accepted() {
        let request = Request::for_profile(Some(crate::gl21::Profile::X1650));
        assert_eq!(request, Request::X1650);
        assert!(request.accepts((2, 1)));
        // The card's own driver, which is what the request exists for.
        let log = open(request, X1650, &X1650_EXTENSIONS).expect("2.1 is what was asked for");
        assert!(log[0].contains("OpenGL 2.1"), "{log:?}");
        assert!(log[0].contains("GLSL 1.20"), "{log:?}");
        assert_eq!(log.len(), 1, "nothing to warn about: {log:?}");
    }

    #[test]
    fn a_newer_context_is_refused_when_the_card_was_asked_for() {
        // A machine with 4.6 asked to pretend it is the X1650: the context it would get is not the
        // renderer's, so the request says so instead of drawing something subtly different.
        let newer = Reported {
            version: "4.6.0 NVIDIA 535.104.05",
            glsl_version: "4.60 NVIDIA",
            ..X1650
        };
        let refused = open(Request::X1650, newer, &X1650_EXTENSIONS).unwrap_err();
        assert!(refused.contains("2.1"), "{refused}");
        assert!(refused.contains("4.6"), "{refused}");
        assert!(refused.contains("HORIZON_GL_PROFILE"), "{refused}");
        // With no profile asked for, the same driver is fine: the renderer runs in it.
        let request = Request::for_profile(None);
        assert!(request.accepts((4, 6)));
        assert!(open(request, newer, &X1650_EXTENSIONS).is_ok());
    }

    #[test]
    fn a_driver_below_the_floor_is_told_every_missing_piece() {
        let old = Reported {
            version: "1.4 Mesa 6.2",
            glsl_version: "1.10",
            max_texture_size: 512,
            max_vertex_attributes: 4,
            max_texture_units: 2,
        };
        let refused = open(Request::X1650, old, &[]).unwrap_err();
        assert!(refused.contains("cannot start"), "{refused}");
        assert!(refused.contains("OpenGL 2.1 is needed"), "{refused}");
        assert!(refused.contains("GLSL 1.20"), "{refused}");
        assert!(refused.contains("1024"), "{refused}");
        assert!(refused.contains("framebuffer"), "{refused}");
        assert!(refused.contains("newest graphics driver"), "{refused}");
    }

    #[test]
    fn unreadable_versions_are_a_sentence_not_a_guess() {
        let silent = Reported {
            version: "(null)",
            glsl_version: "1.20",
            ..X1650
        };
        let error = capabilities(silent, &[]).unwrap_err();
        assert!(error.contains("no readable OpenGL version"), "{error}");
        let no_glsl = Reported {
            glsl_version: "",
            ..X1650
        };
        let error = capabilities(no_glsl, &[]).unwrap_err();
        assert!(error.contains("GLSL version"), "{error}");
    }

    #[test]
    fn the_versions_parse_the_way_drivers_print_them() {
        assert_eq!(parse_version("2.1 Mesa 7.11"), Some((2, 1)));
        assert_eq!(parse_version("4.6.0 NVIDIA 535.104.05"), Some((4, 6)));
        assert_eq!(parse_version("1.4 (2.1 Mesa 7.11)"), Some((1, 4)));
        assert_eq!(parse_version("OpenGL 3.3"), Some((3, 3)));
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("Mesa"), None);
        assert_eq!(parse_version("3"), None);
        assert_eq!(parse_glsl("1.20"), Some(120));
        assert_eq!(parse_glsl("1.20 Mesa"), Some(120));
        assert_eq!(parse_glsl("4.60 NVIDIA"), Some(460));
        assert_eq!(parse_glsl("1.1"), Some(110));
        assert_eq!(parse_glsl("nonsense"), None);
    }

    #[test]
    fn the_loader_says_where_a_missing_pointer_comes_from() {
        let loader = Known(vec!["glTexImage2D", "glDrawArrays", "glGenVertexArrays"]);
        // A 1.1 entry point the driver's getProcAddress does not answer for: load it from libGL.
        let error = require(&loader, "glTexImage2D").unwrap_err();
        assert!(error.contains("OpenGL 1.1"), "{error}");
        assert!(error.contains("libGL"), "{error}");
        // A function that is there.
        assert!(require(&loader, "glGenVertexArrays").is_ok());
        // A function that is not: the driver's answer, not a null call.
        let error = require(&loader, "glMapBuffer").unwrap_err();
        assert!(error.contains("no glMapBuffer"), "{error}");
    }

    #[test]
    fn the_core_1_1_list_is_names_and_not_prefixes() {
        assert!(core_1_1("glGetString"));
        assert!(!core_1_1("glGetStringi"), "that one is 3.0");
        assert!(!core_1_1("glTexImage3D"));
    }
}
