//! Thread-confined OpenGL 2.0 / GLSL 1.10 SDR backend, independent of wgpu.
mod driver;
#[cfg(target_os = "linux")]
mod egl;
#[cfg(target_os = "linux")]
mod osmesa;
mod raw;
mod renderer;
#[cfg(windows)]
mod wgl;
#[cfg(any(windows, test))]
mod wgl_address;
pub use renderer::{Budget, Renderer, Stats};
use std::ffi::{CStr, c_void};

trait ContextImpl {
    fn address(&self, name: &CStr) -> Result<*const c_void, String>;
    fn description(&self) -> &str;
    fn make_current(&self) -> Result<(), String>;
    fn present(&self) -> Result<bool, String> {
        Ok(false)
    }
}

/// Owns the native context. Not Send/Sync; renderer methods rebind it so that
/// multiple contexts on the same thread cannot silently target one another.
pub struct Context {
    inner: Box<dyn ContextImpl>,
    size: [u32; 2],
}
impl Context {
    pub fn new(kind: &str, size: [u32; 2]) -> Result<Self, String> {
        if size.iter().any(|&v| v == 0 || v > 4096) {
            return Err("context dimensions must be 1..=4096".into());
        }
        let inner: Box<dyn ContextImpl> = match kind {
            #[cfg(target_os = "linux")]
            "egl" => Box::new(egl::Context::new(size)?),
            #[cfg(target_os = "linux")]
            "osmesa" => Box::new(osmesa::Context::new(size)?),
            #[cfg(windows)]
            "wgl" => Box::new(wgl::Context::new(size)?),
            _ => {
                return Err(format!(
                    "unsupported context {kind} on {}",
                    std::env::consts::OS
                ));
            }
        };
        Ok(Self { inner, size })
    }
    pub fn size(&self) -> [u32; 2] {
        self.size
    }
    pub fn description(&self) -> &str {
        self.inner.description()
    }
    pub fn make_current(&self) -> Result<(), String> {
        self.inner.make_current()
    }
    /// Function addresses are only valid while this context is alive/current.
    /// Calling an address requires unsafe code and the correct GL ABI.
    pub fn address(&self, name: &CStr) -> Result<*const c_void, String> {
        self.inner.address(name)
    }
    pub fn present(&self) -> Result<bool, String> {
        self.make_current()?;
        self.inner.present()
    }
    pub fn qualify(version: &str, renderer: &str) -> Result<(), String> {
        driver::qualify(version, renderer)
    }
}
