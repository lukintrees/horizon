//! Thread-confined OpenGL 2.0 / GLSL 1.10 SDR backend, independent of wgpu.
mod driver;
#[cfg(target_os = "linux")]
mod egl;
#[cfg(target_os = "linux")]
mod glx;
mod keyboard;
#[cfg(target_os = "linux")]
mod osmesa;
mod raw;
mod renderer;
mod window;
use std::cell::Cell;
pub use window::{MouseButton, WindowEvent, WindowEvents};
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
    fn set_title(&self, _title: &str) -> Result<(), String> {
        Err("context has no window title".into())
    }
    fn resize(&self, _size: [u32; 2]) -> Result<(), String> {
        Err("resize unsupported by this context".into())
    }
    fn is_windowed(&self) -> bool {
        false
    }
    fn drawable_size(&self) -> Option<[u32; 2]> {
        None
    }
    fn set_pointer_mode(
        &self,
        mode: horizon_render_api::platform_contract::PointerMode,
    ) -> Result<(), String> {
        match mode {
            horizon_render_api::platform_contract::PointerMode::Free => Ok(()),
            _ => Err("context has no relative mouse producer".into()),
        }
    }
    fn poll_events(&self) -> Result<WindowEvents, String> {
        Ok(WindowEvents::default())
    }
    fn present(&self) -> Result<bool, String> {
        Ok(false)
    }
}

/// Owns the native context. Not Send/Sync; renderer methods rebind it so that
/// multiple contexts on the same thread cannot silently target one another.
pub struct Context {
    inner: Box<dyn ContextImpl>,
    size: Cell<[u32; 2]>,
}
impl Context {
    pub fn new(kind: &str, size: [u32; 2]) -> Result<Self, String> {
        validate_size(size)?;
        let inner: Box<dyn ContextImpl> = match kind {
            #[cfg(target_os = "linux")]
            "egl" => Box::new(egl::Context::new(size)?),
            #[cfg(target_os = "linux")]
            "glx" => Box::new(glx::Context::new(size)?),
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
        Ok(Self {
            inner,
            size: Cell::new(size),
        })
    }
    /// Offscreen completion must not be reported as window presentation.
    pub fn is_windowed(&self) -> bool {
        self.inner.is_windowed()
    }
    pub fn size(&self) -> [u32; 2] {
        self.inner.drawable_size().unwrap_or(self.size.get())
    }
    /// Caller-supplied (localised) window title; offscreen contexts reject this.
    pub fn set_title(&self, title: &str) -> Result<(), String> {
        if title.contains('\0') {
            return Err("window title contains NUL".into());
        }
        self.inner.set_title(title)
    }
    /// Resize the drawable without replacing the GL context or GPU resources.
    /// Colour/depth contents are unspecified afterwards; redraw the whole frame.
    /// Invalid sizes leave the old surface untouched. Native failures propagate.
    pub fn resize(&self, size: [u32; 2]) -> Result<(), String> {
        validate_size(size)?;
        if self.size() == size {
            return Ok(());
        }
        self.make_current()?;
        let address = self.address(c"glFinish")?;
        // SAFETY: fixed GL1 ABI and current live context. Complete old-buffer
        // work before an offscreen backing allocation can be released.
        unsafe {
            std::mem::transmute::<*const c_void, unsafe extern "system" fn()>(address)();
        }
        self.inner.resize(size)?;
        self.size.set(size);
        Ok(())
    }
    /// Bounded own-window pump; offscreen contexts report no events. Native
    /// modal moving/sizing may temporarily block inside DispatchMessage.
    /// On overflow/focus loss the game must release its held input state.
    pub fn set_pointer_mode(
        &self,
        mode: horizon_render_api::platform_contract::PointerMode,
    ) -> Result<(), String> {
        self.inner.set_pointer_mode(mode)
    }
    pub fn poll_events(&self) -> Result<WindowEvents, String> {
        self.inner.poll_events()
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
        validate_size(self.size())
            .map_err(|_| "surface suspended or exceeds size limit".to_string())?;
        self.make_current()?;
        self.inner.present()
    }
    pub fn qualify(version: &str, renderer: &str) -> Result<(), String> {
        driver::qualify(version, renderer)
    }
}

fn validate_size(size: [u32; 2]) -> Result<(), String> {
    if size.iter().any(|&n| n == 0 || n > 4096) {
        Err("context dimensions must be 1..=4096".into())
    } else {
        Ok(())
    }
}
#[cfg(test)]
mod surface_tests {
    #[test]
    fn rejects_empty_and_unbounded_surfaces_before_native_allocation() {
        for size in [[0, 1], [1, 0], [4097, 1], [1, u32::MAX]] {
            assert!(super::validate_size(size).is_err());
        }
        for size in [[1; 2], [4096; 2], [96, 48]] {
            assert!(super::validate_size(size).is_ok());
        }
    }
}
