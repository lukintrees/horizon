//! Safe ownership boundary for an experimental winit GLX/WGL renderer.
//! This owns the window, context and GL objects on the creating thread. It is
//! still not the game's selectable renderer; Wayland and macOS are unsupported.
use crate::{
    AtlasId, Batch, Draw, Fog, Page, Pass, Region, Renderer, Vertex, window::WindowContext,
};
use std::{
    ffi::{CStr, c_void},
    fmt,
    sync::Arc,
};
#[cfg(target_os = "linux")]
use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use winit::{
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};

#[derive(Debug)]
pub enum Error {
    Window(String),
    Context(crate::window::Error),
    Renderer(crate::Error),
    MissingFunction(&'static str),
    InvalidViewport,
    Gl(u32),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

type Viewport = unsafe extern "system" fn(i32, i32, i32, i32);
type ClearColor = unsafe extern "system" fn(f32, f32, f32, f32);
type ClearDepth = unsafe extern "system" fn(f64);
type Clear = unsafe extern "system" fn(u32);
type DepthMask = unsafe extern "system" fn(u8);
type ColorMask = unsafe extern "system" fn(u8, u8, u8, u8);
type Disable = unsafe extern "system" fn(u32);
type GetError = unsafe extern "system" fn() -> u32;
struct FrameFunctions {
    viewport: Viewport,
    clear_color: ClearColor,
    clear_depth: ClearDepth,
    clear: Clear,
    depth_mask: DepthMask,
    color_mask: ColorMask,
    disable: Disable,
    get_error: GetError,
}
impl FrameFunctions {
    unsafe fn load(context: &WindowContext) -> Result<Self, Error> {
        macro_rules! load {
            ($name:literal, $ty:ty) => {{
                let name = CStr::from_bytes_with_nul(concat!($name, "\0").as_bytes())
                    .expect("static GL name");
                // SAFETY: the caller made the owned context current.
                let address = unsafe { context.resolve(name) };
                if address.is_null() || address as usize <= 3 || address as usize == usize::MAX {
                    return Err(Error::MissingFunction($name));
                }
                // SAFETY: GL defines this named entry point's ABI and signature.
                unsafe { std::mem::transmute::<*const c_void, $ty>(address) }
            }};
        }
        Ok(Self {
            viewport: load!("glViewport", Viewport),
            clear_color: load!("glClearColor", ClearColor),
            clear_depth: load!("glClearDepth", ClearDepth),
            clear: load!("glClear", Clear),
            depth_mask: load!("glDepthMask", DepthMask),
            color_mask: load!("glColorMask", ColorMask),
            disable: load!("glDisable", Disable),
            get_error: load!("glGetError", GetError),
        })
    }
}

/// A window and its GL context, with all operations confined to this thread.
/// On X11 the window visual must support GLX RGBA, double buffering and depth;
/// on Windows its pixel format must not have been set incompatibly already.
/// Drop this surface on the event-loop thread before the window is destroyed.
pub struct Surface {
    renderer: Option<Renderer>,
    context: WindowContext,
    window: Arc<Window>,
    frame: FrameFunctions,
}
impl Surface {
    pub fn new(window: Arc<Window>) -> Result<Self, Error> {
        let handle = window
            .window_handle()
            .map_err(|e| Error::Window(e.to_string()))?;
        #[cfg(target_os = "linux")]
        let context = {
            let display = window
                .display_handle()
                .map_err(|e| Error::Window(e.to_string()))?;
            match (display.as_raw(), handle.as_raw()) {
                (RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(window)) => {
                    let display = display
                        .display
                        .ok_or_else(|| Error::Window("null Xlib display".into()))?;
                    // SAFETY: the retained Arc owns the live winit window and
                    // display, and the context is !Send and remains on this thread.
                    unsafe { WindowContext::new_xlib(display.as_ptr(), window.window) }
                        .map_err(Error::Context)?
                }
                _ => {
                    return Err(Error::Window(
                        "legacy GL requires X11/Xlib, not Wayland".into(),
                    ));
                }
            }
        };
        #[cfg(target_os = "windows")]
        let context = match handle.as_raw() {
            RawWindowHandle::Win32(window) => {
                // SAFETY: the retained Arc owns the live HWND; the context is !Send.
                unsafe { WindowContext::new_win32(window.hwnd.get() as *mut c_void) }
                    .map_err(Error::Context)?
            }
            _ => return Err(Error::Window("legacy GL requires a Win32 window".into())),
        };
        // SAFETY: the constructor made the context current; it is owned below.
        let renderer =
            unsafe { Renderer::new(|name| context.resolve(name)) }.map_err(Error::Renderer)?;
        // SAFETY: same current context, valid until after the renderer is destroyed.
        let frame = match unsafe { FrameFunctions::load(&context) } {
            Ok(frame) => frame,
            Err(error) => {
                // SAFETY: the context is still current on this thread.
                unsafe { renderer.destroy() };
                return Err(error);
            }
        };
        Ok(Self {
            renderer: Some(renderer),
            context,
            window,
            frame,
        })
    }

    fn current(&self) -> Result<(), Error> {
        // SAFETY: self owns the window and context on its creating thread.
        unsafe { self.context.make_current() }.map_err(Error::Context)
    }
    fn renderer(&mut self) -> &mut Renderer {
        self.renderer
            .as_mut()
            .expect("surface renderer remains until drop")
    }
    pub fn window(&self) -> &Window {
        &self.window
    }
    pub fn max_texture_size(&self) -> u32 {
        self.renderer
            .as_ref()
            .expect("surface renderer")
            .max_texture_size()
    }
    pub fn active_atlas(&self) -> AtlasId {
        self.renderer
            .as_ref()
            .expect("surface renderer")
            .active_atlas()
    }
    pub fn create_atlas(&mut self) -> Result<AtlasId, Error> {
        self.renderer().create_atlas().map_err(Error::Renderer)
    }
    pub fn select_atlas(&mut self, id: AtlasId) -> Result<(), Error> {
        self.renderer().select_atlas(id).map_err(Error::Renderer)
    }
    pub fn upload_pages(&mut self, pages: &[Page<'_>]) -> Result<(), Error> {
        self.current()?;
        // SAFETY: the owned context is current and pages remain valid during the call.
        unsafe { self.renderer().upload_pages(pages) }.map_err(Error::Renderer)
    }
    pub fn update_page_region(
        &mut self,
        index: usize,
        page: Page<'_>,
        region: Region,
    ) -> Result<(), Error> {
        self.current()?;
        // SAFETY: the owned context is current and page bytes remain live.
        unsafe { self.renderer().update_page_region(index, page, region) }.map_err(Error::Renderer)
    }
    /// Set the viewport and clear color/depth. Returns false for a minimized window.
    pub fn begin_frame(&mut self, color: [f32; 4]) -> Result<bool, Error> {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(false);
        }
        if size.width > i32::MAX as u32 || size.height > i32::MAX as u32 {
            return Err(Error::InvalidViewport);
        }
        self.current()?;
        // SAFETY: all function pointers belong to the current owned context.
        unsafe {
            let f = &self.frame;
            let previous = (f.get_error)();
            if previous != 0 {
                return Err(Error::Gl(previous));
            }
            (f.disable)(0x0c11); // GL_SCISSOR_TEST
            (f.depth_mask)(1);
            (f.color_mask)(1, 1, 1, 1);
            (f.viewport)(0, 0, size.width as i32, size.height as i32);
            (f.clear_color)(color[0], color[1], color[2], color[3]);
            (f.clear_depth)(1.0);
            (f.clear)(0x4000 | 0x0100);
            let error = (f.get_error)();
            if error != 0 {
                return Err(Error::Gl(error));
            }
        }
        Ok(true)
    }
    /// Clear the whole depth buffer before a separate first-person pass.
    /// This intentionally destroys world depth and must run after world draws.
    pub fn clear_depth(&mut self) -> Result<(), Error> {
        self.current()?;
        // SAFETY: these pointers were loaded for the owned current context.
        unsafe {
            let f = &self.frame;
            let previous = (f.get_error)();
            if previous != 0 {
                return Err(Error::Gl(previous));
            }
            (f.disable)(0x0c11); // GL_SCISSOR_TEST
            (f.depth_mask)(1);
            (f.clear_depth)(1.0);
            (f.clear)(0x0100); // GL_DEPTH_BUFFER_BIT
            let error = (f.get_error)();
            if error != 0 {
                return Err(Error::Gl(error));
            }
        }
        Ok(())
    }

    pub fn draw(
        &mut self,
        vertices: &[Vertex],
        batches: &[Batch],
        transform: &[f32; 16],
        pass: Pass,
    ) -> Result<(), Error> {
        self.current()?;
        // SAFETY: the owned context is current; the renderer validates batches.
        unsafe { self.renderer().draw(vertices, batches, transform, pass) }.map_err(Error::Renderer)
    }
    pub fn draw_ordered(
        &mut self,
        vertices: &[Vertex],
        draws: &[Draw],
        transform: &[f32; 16],
        fog: Option<Fog>,
    ) -> Result<(), Error> {
        self.current()?;
        // SAFETY: the owned context is current; the renderer validates batches.
        unsafe {
            match fog {
                Some(fog) => self
                    .renderer()
                    .draw_ordered_with_fog(vertices, draws, transform, fog),
                None => self.renderer().draw_ordered(vertices, draws, transform),
            }
        }
        .map_err(Error::Renderer)
    }
    pub fn present(&mut self) -> Result<(), Error> {
        self.current()?;
        // SAFETY: the owned window and its current context remain alive.
        unsafe { self.context.swap_buffers() }.map_err(Error::Context)
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: this !Send object owns the context and window on this thread.
        unsafe {
            if self.context.make_current().is_ok()
                && let Some(renderer) = self.renderer.take()
            {
                renderer.destroy();
            }
        }
        // The context field is dropped before the retained window field.
    }
}
