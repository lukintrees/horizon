//! One visible Linux path: a private Xlib window + legacy GLX desktop context.
//! No winit, EGL pbuffer, foreign window IDs, newer GL requirement or fallback.
mod abi;
mod keyboard;
mod raw_input;
use crate::{
    egl::Library,
    window::{MouseButton, State, WindowEvent, WindowEvents},
};
use abi::*;
use std::{
    cell::{Cell, RefCell},
    ffi::{CStr, CString, c_char, c_int, c_void},
    marker::PhantomData,
    ptr,
    rc::Rc,
};

struct Calls {
    close: CloseDisplay,
    root: RootWindow,
    create_colormap: CreateColormap,
    free_colormap: FreeColormap,
    create_window: CreateWindow,
    destroy_window: WindowCall,
    map: WindowCall,
    resize: ResizeWindow,
    sync: Sync,
    pending: Pending,
    next: NextEvent,
    peek: NextEvent,
    intern: InternAtom,
    protocols: SetProtocols,
    store_name: StoreName,
    property: ChangeProperty,
    free: Free,
    geometry: GetGeometry,
    lookup: LookupString,
    base_keysym: LookupKeysym,
    refresh_keyboard: RefreshKeyboard,
    unicode: KeysymToUnicode,
    create_context: CreateContext,
    destroy_context: DestroyContext,
    current: MakeCurrent,
    get_current: GetCurrent,
    swap: SwapBuffers,
    proc: GetProc,
}
pub struct Context {
    calls: Calls,
    // Libraries outlive every pointer and native object's destruction.
    _x11: Library,
    gl: Library,
    _keyboard: Library,
    display: Display,
    window: Xid,
    root: Xid,
    colormap: Xid,
    context: *mut c_void,
    protocols: Xid,
    delete: Xid,
    destroyed: Cell<bool>,
    mapped: Cell<bool>,
    state: State,
    keys: RefCell<[bool; 256]>,
    usages: RefCell<[u16; 256]>,
    relative: RefCell<Option<raw_input::Input>>,
    keyboard_error: RefCell<Option<String>>,
    description: String,
    _thread: PhantomData<Rc<()>>,
}
impl Context {
    pub fn new(size: [u32; 2]) -> Result<Self, String> {
        let x11 = Library::open(c"libX11.so.6")?;
        let gl = Library::open(c"libGL.so.1")?;
        // Unicode keysym decoding is locale-independent and does not mutate
        // process-global setlocale while Game CPU workers are running. XIM/
        // preedit/compose are not claimed by this initial core-event adapter.
        let keyboard = Library::open(c"libxkbcommon.so.0")?;
        macro_rules! load {
            ($lib:ident,$name:literal,$ty:ty) => {{
                let address = $lib.symbol($name)?;
                // SAFETY: fixed Xlib/GLX/xkbcommon names and matching C signatures.
                unsafe { std::mem::transmute::<*const c_void, $ty>(address) }
            }};
        }
        let open = load!(x11, c"XOpenDisplay", OpenDisplay);
        let screen = load!(x11, c"XDefaultScreen", DefaultScreen);
        let choose = load!(gl, c"glXChooseVisual", ChooseVisual);
        let version = load!(gl, c"glXQueryVersion", QueryVersion);
        let mut this = Self {
            calls: Calls {
                close: load!(x11, c"XCloseDisplay", CloseDisplay),
                root: load!(x11, c"XRootWindow", RootWindow),
                create_colormap: load!(x11, c"XCreateColormap", CreateColormap),
                free_colormap: load!(x11, c"XFreeColormap", FreeColormap),
                create_window: load!(x11, c"XCreateWindow", CreateWindow),
                destroy_window: load!(x11, c"XDestroyWindow", WindowCall),
                map: load!(x11, c"XMapWindow", WindowCall),
                resize: load!(x11, c"XResizeWindow", ResizeWindow),
                sync: load!(x11, c"XSync", Sync),
                pending: load!(x11, c"XPending", Pending),
                next: load!(x11, c"XNextEvent", NextEvent),
                peek: load!(x11, c"XPeekEvent", NextEvent),
                intern: load!(x11, c"XInternAtom", InternAtom),
                protocols: load!(x11, c"XSetWMProtocols", SetProtocols),
                store_name: load!(x11, c"XStoreName", StoreName),
                property: load!(x11, c"XChangeProperty", ChangeProperty),
                free: load!(x11, c"XFree", Free),
                geometry: load!(x11, c"XGetGeometry", GetGeometry),
                lookup: load!(x11, c"XLookupString", LookupString),
                base_keysym: load!(x11, c"XLookupKeysym", LookupKeysym),
                refresh_keyboard: load!(x11, c"XRefreshKeyboardMapping", RefreshKeyboard),
                unicode: load!(keyboard, c"xkb_keysym_to_utf32", KeysymToUnicode),
                create_context: load!(gl, c"glXCreateContext", CreateContext),
                destroy_context: load!(gl, c"glXDestroyContext", DestroyContext),
                current: load!(gl, c"glXMakeCurrent", MakeCurrent),
                get_current: load!(gl, c"glXGetCurrentContext", GetCurrent),
                swap: load!(gl, c"glXSwapBuffers", SwapBuffers),
                proc: load!(gl, c"glXGetProcAddressARB", GetProc),
            },
            _x11: x11,
            gl,
            _keyboard: keyboard,
            display: ptr::null_mut(),
            window: 0,
            root: 0,
            colormap: 0,
            context: ptr::null_mut(),
            protocols: 0,
            delete: 0,
            destroyed: Cell::new(false),
            mapped: Cell::new(true),
            state: State::new(size),
            keys: RefCell::new([false; 256]),
            usages: RefCell::new([0; 256]),
            relative: RefCell::new(None),
            keyboard_error: RefCell::new(None),
            description: String::new(),
            _thread: PhantomData,
        };
        // SAFETY: all handles originate from these live libraries. Initialized
        // Self owns partial allocations, so every failure tears them down.
        unsafe {
            this.display = open(ptr::null());
            if this.display.is_null() {
                return Err("XOpenDisplay failed: native GLX requires a running X11 display (no offscreen fallback)".into());
            }
            let (mut major, mut minor) = (0, 0);
            if version(this.display, &mut major, &mut minor) == 0 {
                return Err("X11 display has no GLX extension".into());
            }
            let screen = screen(this.display);
            this.root = (this.calls.root)(this.display, screen);
            let mut attributes = [4, 5, 8, 8, 9, 8, 10, 8, 11, 8, 12, 24, 0];
            let visual = choose(this.display, screen, attributes.as_mut_ptr());
            if visual.is_null() {
                return Err("no RGBA double-buffered GLX depth24 visual".into());
            }
            // Copy the selected record, free Xlib storage even on later errors.
            let info = *visual;
            (this.calls.free)(visual.cast());
            let root = (this.calls.root)(this.display, screen);
            this.colormap = (this.calls.create_colormap)(this.display, root, info.visual, 0);
            if this.colormap == 0 {
                return Err("XCreateColormap failed".into());
            }
            let mut attrs = Attributes {
                colormap: this.colormap,
                event_mask: (1 << 0)
                    | (1 << 1)
                    | (1 << 2)
                    | (1 << 3)
                    | (1 << 6)
                    | (1 << 15)
                    | (1 << 17)
                    | (1 << 21),
                ..Default::default()
            };
            this.window = (this.calls.create_window)(
                this.display,
                root,
                0,
                0,
                size[0],
                size[1],
                0,
                info.depth,
                1,
                info.visual,
                (1 << 13) | (1 << 11),
                &mut attrs,
            );
            if this.window == 0 {
                return Err("XCreateWindow failed".into());
            }
            this.context = (this.calls.create_context)(
                this.display,
                &info as *const _ as *mut _,
                ptr::null_mut(),
                1,
            );
            if this.context.is_null() {
                return Err("glXCreateContext failed (no modern fallback)".into());
            }
            this.protocols = (this.calls.intern)(this.display, c"WM_PROTOCOLS".as_ptr(), 0);
            this.delete = (this.calls.intern)(this.display, c"WM_DELETE_WINDOW".as_ptr(), 0);
            if this.protocols == 0
                || this.delete == 0
                || (this.calls.protocols)(this.display, this.window, &mut this.delete, 1) == 0
            {
                return Err("XSetWMProtocols failed".into());
            }
            this.description =
                format!("GLX {major}.{minor} X11 visible window, legacy desktop context");
            crate::ContextImpl::set_title(&this, "Horizon — Native GL2")?;
            (this.calls.map)(this.display, this.window);
            (this.calls.sync)(this.display, 0);
        }
        *this.usages.borrow_mut() = keyboard::read(&this._x11, this.display)?;
        crate::ContextImpl::make_current(&this)?;
        Ok(this)
    }
    fn geometry(&self) -> Result<[u32; 2], String> {
        let (mut root, mut x, mut y, mut w, mut h, mut border, mut depth) = (0, 0, 0, 0, 0, 0, 0);
        // SAFETY: live display/window and correctly typed output storage.
        if self.destroyed.get()
            || unsafe {
                (self.calls.geometry)(
                    self.display,
                    self.window,
                    &mut root,
                    &mut x,
                    &mut y,
                    &mut w,
                    &mut h,
                    &mut border,
                    &mut depth,
                )
            } == 0
        {
            return Err("XGetGeometry failed for native drawable".into());
        }
        Ok([w, h])
    }
    fn focus_lost(&self) {
        if let Some(input) = self.relative.borrow().as_ref() {
            let _ = input.set(false);
        }
        self.keys.borrow_mut().fill(false);
        self.state.buttons.set(0);
        self.state.push(WindowEvent::Focused(false));
        self.state.push(WindowEvent::PointerCaptureLost);
    }
    fn event(&self, event: &mut Event) {
        // SAFETY: Event is filled by XNextEvent; type determines the ABI member.
        unsafe {
            if event.kind == 35 {
                if let Some(input) = self.relative.borrow().as_ref() {
                    match input.event((event as *mut Event).cast()) {
                        Ok(Some(motion)) => self.state.push(motion),
                        Ok(None) => {}
                        Err(error) => *self.keyboard_error.borrow_mut() = Some(error),
                    }
                }
                return;
            }
            if event.kind == 34 {
                (self.calls.refresh_keyboard)(event);
                self.keys.borrow_mut().fill(false);
                self.state.push(WindowEvent::PointerCaptureLost);
                match keyboard::read(&self._x11, self.display) {
                    Ok(usages) => *self.usages.borrow_mut() = usages,
                    Err(error) => *self.keyboard_error.borrow_mut() = Some(error),
                }
                return;
            }
            if event.any.window != self.window {
                return;
            }
            match event.kind {
                2 | 3 => {
                    let key = &mut event.key;
                    let pressed = key.any.kind == 2;
                    if key.code >= 256 {
                        return;
                    }
                    // Core X11 autorepeat is release+press with identical time.
                    // Never emit the synthetic release, including across polls.
                    if !pressed && (self.calls.pending)(self.display) > 0 {
                        let mut next = Event::default();
                        (self.calls.peek)(self.display, &mut next);
                        if next.kind == 2
                            && next.key.any.window == self.window
                            && next.key.code == key.code
                            && next.key.time == key.time
                        {
                            return;
                        }
                    }
                    let repeat =
                        std::mem::replace(&mut self.keys.borrow_mut()[key.code as usize], pressed)
                            && pressed;
                    let base = (self.calls.base_keysym)(key, 0) as u32;
                    self.state.push(WindowEvent::X11Key {
                        keysym: base,
                        keycode: key.code as u8,
                        pressed,
                        repeat,
                    });
                    let usage = match base {
                        0xff1b => 0x29,
                        0xffe5 => 0x39, // follow explicit escape/caps keymap swaps
                        _ => self.usages.borrow()[key.code as usize],
                    };
                    if usage != 0 {
                        self.state.push(WindowEvent::PhysicalKey {
                            usage,
                            pressed,
                            repeat,
                        });
                    }
                    if pressed {
                        let mut symbol = 0;
                        let mut bytes = [0 as c_char; 64];
                        (self.calls.lookup)(
                            key,
                            bytes.as_mut_ptr(),
                            64,
                            &mut symbol,
                            ptr::null_mut(),
                        );
                        if let Some(character) = char::from_u32((self.calls.unicode)(symbol as u32))
                            && !character.is_control()
                        {
                            self.state.push(WindowEvent::Text {
                                character,
                                repeat_count: 1,
                            });
                        }
                    }
                }
                4 | 5 => {
                    let mouse = event.key;
                    let pressed = mouse.any.kind == 4;
                    let (button, bit) = match mouse.code {
                        1 => (MouseButton::Left, 1),
                        2 => (MouseButton::Middle, 2),
                        3 => (MouseButton::Right, 4),
                        8 => (MouseButton::X1, 8),
                        9 => (MouseButton::X2, 16),
                        4..=7 => {
                            if pressed {
                                self.state.push(WindowEvent::Wheel {
                                    horizontal: mouse.code >= 6,
                                    delta: if matches!(mouse.code, 4 | 7) {
                                        120
                                    } else {
                                        -120
                                    },
                                });
                            }
                            return;
                        }
                        _ => return,
                    };
                    self.state.buttons.set(if pressed {
                        self.state.buttons.get() | bit
                    } else {
                        self.state.buttons.get() & !bit
                    });
                    self.state.push(WindowEvent::MouseButton {
                        button,
                        pressed,
                        x: mouse.x,
                        y: mouse.y,
                    });
                }
                6 => {
                    let mouse = event.motion;
                    self.state.push(WindowEvent::PointerMoved {
                        x: mouse.x,
                        y: mouse.y,
                    });
                }
                9 => self.state.push(WindowEvent::Focused(true)),
                10 => self.focus_lost(),
                17 => {
                    self.destroyed.set(true);
                    self.state.resized([0; 2]);
                    self.state.push(WindowEvent::CloseRequested);
                }
                18 => {
                    self.mapped.set(false);
                    self.state.resized([0; 2]);
                    self.focus_lost();
                }
                19 => {
                    self.mapped.set(true);
                    if let Ok(size) = self.geometry() {
                        self.state.resized(size);
                    }
                }
                22 => {
                    let e = event.configure;
                    if self.mapped.get() {
                        self.state
                            .resized([e.width.max(0) as u32, e.height.max(0) as u32]);
                    }
                }
                33 => {
                    let e = event.client;
                    if e.message_type == self.protocols
                        && e.format == 32
                        && e.data[0] as Xid == self.delete
                    {
                        self.state.push(WindowEvent::CloseRequested);
                    }
                }
                _ => {}
            }
        }
    }
}
impl crate::ContextImpl for Context {
    fn address(&self, name: &CStr) -> Result<*const c_void, String> {
        // SAFETY: GLX library is live; entry point names are NUL-terminated.
        let address = unsafe { (self.calls.proc)(name.as_ptr().cast()) };
        if !address.is_null() {
            Ok(address)
        } else {
            self.gl.symbol(name)
        }
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn make_current(&self) -> Result<(), String> {
        if self.destroyed.get() {
            return Err("native GLX drawable was destroyed".into());
        }
        // SAFETY: thread-confined, owned and live GLX context/window/display.
        if unsafe { (self.calls.current)(self.display, self.window, self.context) } == 0 {
            Err("glXMakeCurrent failed".into())
        } else {
            Ok(())
        }
    }
    fn set_title(&self, title: &str) -> Result<(), String> {
        if self.destroyed.get() {
            return Err("cannot set title on destroyed GLX window".into());
        }
        let text = CString::new(title).map_err(|_| "native window title contains NUL")?;
        let length = c_int::try_from(title.len()).map_err(|_| "native window title too long")?;
        // SAFETY: native display/window live, strings and bytes remain borrowed
        // only during calls. _NET_WM_NAME is explicitly UTF8_STRING, not Latin1.
        unsafe {
            (self.calls.store_name)(self.display, self.window, text.as_ptr());
            let property = (self.calls.intern)(self.display, c"_NET_WM_NAME".as_ptr(), 0);
            let utf8 = (self.calls.intern)(self.display, c"UTF8_STRING".as_ptr(), 0);
            (self.calls.property)(
                self.display,
                self.window,
                property,
                utf8,
                8,
                0,
                title.as_ptr(),
                length,
            );
        }
        Ok(())
    }
    fn resize(&self, size: [u32; 2]) -> Result<(), String> {
        if self.destroyed.get() {
            return Err("cannot resize destroyed GLX window".into());
        }
        // SAFETY: dimensions validated by outer Context, owned live X window.
        unsafe {
            (self.calls.resize)(self.display, self.window, size[0], size[1]);
            (self.calls.sync)(self.display, 0);
        }
        let actual = self.geometry()?;
        self.state.resized(actual);
        if actual != size {
            return Err(format!(
                "X11 window manager refused native resize: {actual:?}"
            ));
        }
        Ok(())
    }
    fn is_windowed(&self) -> bool {
        true
    }
    fn drawable_size(&self) -> Option<[u32; 2]> {
        Some(self.state.size.get())
    }
    fn set_pointer_mode(
        &self,
        mode: horizon_render_api::platform_contract::PointerMode,
    ) -> Result<(), String> {
        let relative = mode == horizon_render_api::platform_contract::PointerMode::Relative;
        let mut input = self.relative.borrow_mut();
        if input.is_none() && relative {
            if self.destroyed.get() || self.state.size.get().contains(&0) {
                return Err("native drawable cannot capture pointer".into());
            }
            let root = self.root;
            *input = Some(raw_input::Input::new(
                &self._x11,
                self.display,
                root,
                self.window,
            )?);
        }
        if let Some(input) = input.as_ref() {
            input.set(relative)?;
        }
        Ok(())
    }
    fn poll_events(&self) -> Result<WindowEvents, String> {
        for _ in 0..256 {
            // SAFETY: XPending is nonblocking; XNextEvent called only with a
            // queued event, into a correctly sized/aligned XEvent union.
            unsafe {
                if (self.calls.pending)(self.display) == 0 {
                    break;
                }
                let mut event = Event::default();
                (self.calls.next)(self.display, &mut event);
                self.event(&mut event);
            }
        }
        if let Some(error) = self.keyboard_error.borrow_mut().take() {
            return Err(error);
        }
        // SAFETY: live owned display. Remaining messages are never discarded.
        Ok(self
            .state
            .take(unsafe { (self.calls.pending)(self.display) > 0 }))
    }
    fn present(&self) -> Result<bool, String> {
        if self.destroyed.get() || self.state.size.get().contains(&0) {
            return Ok(false);
        }
        // SAFETY: outer Context rebound this owned live double-buffer drawable.
        unsafe {
            (self.calls.swap)(self.display, self.window);
        }
        Ok(true)
    }
}
impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: uniquely owned allocations, reverse teardown order. Do not
        // unbind a different context belonging to another owner on this thread.
        unsafe {
            if !self.display.is_null() {
                drop(self.relative.get_mut().take());
                if !self.context.is_null() {
                    if (self.calls.get_current)() == self.context {
                        (self.calls.current)(self.display, 0, ptr::null_mut());
                    }
                    (self.calls.destroy_context)(self.display, self.context);
                }
                if self.window != 0 && !self.destroyed.get() {
                    (self.calls.destroy_window)(self.display, self.window);
                }
                if self.colormap != 0 {
                    (self.calls.free_colormap)(self.display, self.colormap);
                }
                (self.calls.sync)(self.display, 0);
                (self.calls.close)(self.display);
            }
        }
    }
}

#[cfg(test)]
mod tests;
