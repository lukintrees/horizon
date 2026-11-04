//! Finding the GL library and its entry points, at run time.
//!
//! The game links no graphics library, so a machine without a driver must open the window,
//! say what is missing and exit cleanly, rather than fail in the loader before `main` with a
//! missing-DLL dialog. The wgpu path does the same for Vulkan (`gfx.rs`'s `NO_GPU_DRIVER`);
//! this is the legacy path's version of it.
//!
//! Two facts shape the code:
//!
//! * **On Windows, `opengl32.dll` exports only GL 1.1.** Everything newer (and on this path
//!   almost everything is newer: VBOs are 1.5, shaders are 2.0) comes from
//!   `wglGetProcAddress`, which returns `NULL` for a function the driver has not got —
//!   sometimes also for a 1.1 function it exports but does not list. So the lookup tries
//!   `wglGetProcAddress` first, then the module's own exports, and treats `NULL` as "this
//!   driver has not got it", which is a sentence for the player, not a crash.
//! * **On Linux, GLX is the way in, not the library's exports.** `glXGetProcAddressARB`
//!   answers for everything, including the 1.0 and 1.1 functions that `libGL.so.1` also
//!   exports. Where `libGL.so.1` cannot be opened at all (no Mesa, no X), the error names
//!   the package to install.
//!
//! Nothing here decides what the renderer may use: a function that exists is not a feature.
//! [`crate::caps`] is the only place that answers that question.

use std::ffi::{c_void, CString};

/// A graphics library that could not be opened, with the sentence the game shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoGlLibrary {
    pub tried: Vec<String>,
    pub message: String,
}

impl NoGlLibrary {
    fn new(tried: Vec<String>, message: impl Into<String>) -> Self {
        Self {
            tried,
            message: message.into(),
        }
    }
}

/// An opened GL library, kept alive for as long as the context built from it.
///
/// The library must outlive the function pointers glow copies out of it: dropping it while a
/// context still exists is a use-after-free that crashes on the first frame, so the context
/// holds this struct and this struct holds the loaded library.
pub struct GlLibrary {
    /// The loaded library. `None` only if support for this platform is compiled out.
    lib: Option<libloading::Library>,
    /// `wglGetProcAddress` or `glXGetProcAddressARB`, when the library has one.
    get_proc: Option<unsafe extern "C" fn(*const i8) -> *const c_void>,
    name: &'static str,
}

impl std::fmt::Debug for GlLibrary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlLibrary").field("name", &self.name).finish()
    }
}

impl GlLibrary {
    /// Opens the system's GL library: `opengl32.dll` on Windows, `libGL.so.1` on Linux and
    /// the BSDs. macOS is not a target of this path: Metal is its only backend and every Mac
    /// that runs Horizon has it.
    pub fn open() -> Result<Self, NoGlLibrary> {
        #[cfg(windows)]
        {
            Self::open_windows()
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            Self::open_unix()
        }
        #[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
        {
            let _ = CString::new("glGetString");
            Err(NoGlLibrary::new(
                Vec::new(),
                "This system has no OpenGL 2.1 path in Horizon: the legacy renderer covers \
                 Windows and Linux",
            ))
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn open_unix() -> Result<Self, NoGlLibrary> {
        // Sonames in the order a distribution is likely to have them; the first that loads
        // wins. `libGL.so.1` is the ABI-stable soname: never `libGL.so`, which is a
        // development symlink absent on players' machines.
        let candidates = ["libGL.so.1", "libGL.so"];
        let mut tried = Vec::new();
        for name in candidates {
            match unsafe { libloading::Library::new(name) } {
                Ok(lib) => {
                    let get_proc = unsafe {
                        lib.get::<unsafe extern "C" fn(*const i8) -> *const c_void>(
                            b"glXGetProcAddressARB\0",
                        )
                        .or_else(|_| {
                            lib.get::<unsafe extern "C" fn(*const i8) -> *const c_void>(
                                b"glXGetProcAddress\0",
                            )
                        })
                        .map(|f| *f)
                        .ok()
                    };
                    return Ok(Self {
                        lib: Some(lib),
                        get_proc,
                        name,
                    });
                }
                Err(e) => tried.push(format!("{name}: {e}")),
            }
        }
        Err(NoGlLibrary::new(
            tried,
            "No OpenGL library: install a GL driver and its library (Mesa: libgl1-mesa-dri \
             and libgl1, or the vendor's driver), then start the game again",
        ))
    }

    #[cfg(windows)]
    fn open_windows() -> Result<Self, NoGlLibrary> {
        let mut tried = Vec::new();
        match unsafe { libloading::Library::new("opengl32.dll") } {
            Ok(lib) => {
                let get_proc = unsafe {
                    lib.get::<unsafe extern "C" fn(*const i8) -> *const c_void>(
                        b"wglGetProcAddress\0",
                    )
                    .map(|f| *f)
                    .ok()
                };
                Ok(Self {
                    lib: Some(lib),
                    get_proc,
                    name: "opengl32.dll",
                })
            }
            Err(e) => {
                tried.push(format!("opengl32.dll: {e}"));
                Err(NoGlLibrary::new(
                    tried,
                    "No OpenGL library: opengl32.dll did not load. Install the latest driver \
                     for your graphics card and start the game again",
                ))
            }
        }
    }

    /// One entry point by name: `wglGetProcAddress`/`glXGetProcAddressARB` first, the
    /// library's exports second. `null` means the driver has not got it.
    pub fn symbol(&self, name: &str) -> *const c_void {
        let Ok(c_name) = CString::new(name) else {
            // A name with a NUL in it is a bug in this crate, not a driver limitation.
            return std::ptr::null();
        };
        if let Some(get_proc) = self.get_proc {
            let ptr = unsafe { get_proc(c_name.as_ptr().cast()) };
            if !ptr.is_null() {
                return ptr;
            }
        }
        match &self.lib {
            Some(lib) => match unsafe { lib.get::<*const c_void>(c_name.as_bytes_with_nul()) } {
                Ok(ptr) => *ptr,
                Err(_) => std::ptr::null(),
            },
            None => std::ptr::null(),
        }
    }

    /// Whether the driver has this entry point. Used to decide whether the code may call a
    /// function at all (the extension string says it should be there; this says it is).
    pub fn has(&self, name: &str) -> bool {
        !self.symbol(name).is_null()
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The ones of [`ESSENTIAL`] this driver has not got, so the refusal can name them.
    pub fn missing_essentials(&self) -> Vec<&'static str> {
        ESSENTIAL
            .iter()
            .copied()
            .filter(|name| !self.has(name))
            .collect()
    }
}

/// The entry points a 2.1 context cannot work without. Checked before glow is built, because
/// glow caches the pointers and calls them without checking: a null one is a segfault in the
/// driver, which leaves no message at all (docs/gl21-dev-env.md §5).
pub const ESSENTIAL: &[&str] = &[
    "glGetString",
    "glGetIntegerv",
    "glGetError",
    "glCreateShader",
    "glShaderSource",
    "glCompileShader",
    "glGetShaderiv",
    "glGetShaderInfoLog",
    "glCreateProgram",
    "glAttachShader",
    "glLinkProgram",
    "glGetProgramiv",
    "glGetProgramInfoLog",
    "glUseProgram",
    "glDeleteShader",
    "glDeleteProgram",
    "glGetUniformLocation",
    "glUniform1i",
    "glUniform1f",
    "glUniform2fv",
    "glUniform3fv",
    "glUniform4fv",
    "glUniformMatrix4fv",
    "glGenBuffers",
    "glBindBuffer",
    "glBufferData",
    "glBufferSubData",
    "glDeleteBuffers",
    "glVertexAttribPointer",
    "glEnableVertexAttribArray",
    "glDrawElements",
    "glDrawArrays",
    "glGenTextures",
    "glBindTexture",
    "glTexImage2D",
    "glTexParameteri",
    "glDeleteTextures",
    "glGenFramebuffers",
    "glBindFramebuffer",
    "glFramebufferTexture2D",
    "glGenRenderbuffers",
    "glBindRenderbuffer",
    "glRenderbufferStorage",
    "glFramebufferRenderbuffer",
    "glCheckFramebufferStatus",
    "glDeleteFramebuffers",
    "glDeleteRenderbuffers",
    "glEnable",
    "glDisable",
    "glBlendFunc",
    "glDepthFunc",
    "glDepthMask",
    "glCullFace",
    "glFrontFace",
    "glViewport",
    "glClear",
    "glClearColor",
    "glScissor",
    "glPixelStorei",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_essential_list_has_no_duplicates_and_names_the_2_1_entry_points() {
        let mut seen = std::collections::BTreeSet::new();
        for name in ESSENTIAL {
            assert!(seen.insert(*name), "{name} is listed twice");
            assert!(name.starts_with("gl"), "{name} is not a GL entry point");
        }
        // Shaders and VBOs are the two things a 2.1 context must have for this renderer.
        assert!(ESSENTIAL.contains(&"glCreateShader"));
        assert!(ESSENTIAL.contains(&"glGenBuffers"));
        // ...and the ones the target does *not* have must never be required here.
        assert!(!ESSENTIAL.contains(&"glVertexAttribDivisor"));
        assert!(!ESSENTIAL.contains(&"glGenVertexArrays"));
        assert!(!ESSENTIAL.contains(&"glDispatchCompute"));
        assert!(!ESSENTIAL.contains(&"glBindBufferRange"));
    }

    /// Whether this machine can open a GL library at all. Not a failure: a build machine
    /// without a driver still compiles and runs every test that does not draw.
    #[test]
    fn opening_the_system_library_says_why_when_it_cannot() {
        match GlLibrary::open() {
            Ok(lib) => {
                assert!(!lib.name().is_empty());
                let missing = lib.missing_essentials();
                if !missing.is_empty() {
                    eprintln!(
                        "{} opened but lacks {} entry point(s): {missing:?}",
                        lib.name(),
                        missing.len()
                    );
                }
            }
            Err(e) => {
                assert!(!e.message.is_empty());
                assert!(!e.tried.is_empty(), "a refusal must say what it tried");
            }
        }
    }

    #[test]
    fn an_unknown_symbol_is_null_and_says_so_rather_than_panicking() {
        // On a machine with a library this asks the driver about a name no driver has; on a
        // machine without one the open fails and there is nothing to ask.
        if let Ok(lib) = GlLibrary::open() {
            assert!(!lib.has("glDefinitelyNotAFunction_Horizon"));
            assert!(lib.symbol("glDefinitelyNotAFunction_Horizon").is_null());
        }
    }
}
