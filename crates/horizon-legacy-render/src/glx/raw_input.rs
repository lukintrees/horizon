//! XI2 raw motion, never absolute cursor-edge deltas or a recentering loop.
use super::{Display, Library, Xid};
use crate::window::WindowEvent;
use std::{
    cell::Cell,
    ffi::{c_char, c_int, c_uint, c_ulong, c_void},
};
type Grab =
    unsafe extern "C" fn(Display, Xid, c_int, c_uint, c_int, c_int, Xid, Xid, c_ulong) -> c_int;
type Ungrab = unsafe extern "C" fn(Display, c_ulong) -> c_int;
type Select = unsafe extern "C" fn(Display, Xid, *mut Mask, c_int) -> c_int;
type CookieCall = unsafe extern "C" fn(Display, *mut Cookie) -> c_int;
type FreeCookie = unsafe extern "C" fn(Display, *mut Cookie);
type Focus = unsafe extern "C" fn(Display, *mut Xid, *mut c_int) -> c_int;
type FreeCursor = unsafe extern "C" fn(Display, Xid) -> c_int;
#[repr(C)]
struct Mask {
    device: c_int,
    len: c_int,
    mask: *mut u8,
}
#[repr(C)]
struct Cookie {
    kind: c_int,
    serial: c_ulong,
    sent: c_int,
    display: Display,
    extension: c_int,
    event_type: c_int,
    cookie: c_uint,
    data: *mut c_void,
}
#[repr(C)]
struct Valuators {
    len: c_int,
    mask: *const u8,
    values: *const f64,
}
#[repr(C)]
struct Raw {
    kind: c_int,
    serial: c_ulong,
    sent: c_int,
    display: Display,
    extension: c_int,
    event_type: c_int,
    time: c_ulong,
    device: c_int,
    source: c_int,
    detail: c_int,
    flags: c_int,
    valuators: Valuators,
    raw_values: *const f64,
}
#[repr(C)]
struct Color {
    pixel: c_ulong,
    red: u16,
    green: u16,
    blue: u16,
    flags: u8,
    pad: u8,
}
pub(super) struct Input {
    display: Display,
    root: Xid,
    window: Xid,
    cursor: Xid,
    opcode: c_int,
    select: Select,
    grab: Grab,
    ungrab: Ungrab,
    get: CookieCall,
    free: FreeCookie,
    free_cursor: FreeCursor,
    focus: Focus,
    active: Cell<bool>,
}
impl Input {
    pub fn new(
        x11: &Library,
        xi: &Library,
        display: Display,
        root: Xid,
        window: Xid,
    ) -> Result<Self, String> {
        macro_rules! load {
            ($lib:ident,$name:literal,$ty:ty) => {{
                // SAFETY: fixed public Xlib/XInput function name and C signature.
                unsafe { std::mem::transmute::<*const c_void, $ty>($lib.symbol($name)?) }
            }};
        }
        let extension = load!(
            x11,
            c"XQueryExtension",
            unsafe extern "C" fn(
                Display,
                *const c_char,
                *mut c_int,
                *mut c_int,
                *mut c_int,
            ) -> c_int
        );
        let version = load!(
            xi,
            c"XIQueryVersion",
            unsafe extern "C" fn(Display, *mut c_int, *mut c_int) -> c_int
        );
        let bitmap = load!(
            x11,
            c"XCreateBitmapFromData",
            unsafe extern "C" fn(Display, Xid, *const c_char, c_uint, c_uint) -> Xid
        );
        let cursor = load!(
            x11,
            c"XCreatePixmapCursor",
            unsafe extern "C" fn(
                Display,
                Xid,
                Xid,
                *const Color,
                *const Color,
                c_uint,
                c_uint,
            ) -> Xid
        );
        let free_pixmap = load!(
            x11,
            c"XFreePixmap",
            unsafe extern "C" fn(Display, Xid) -> c_int
        );
        let select = load!(xi, c"XISelectEvents", Select);
        let mut this = Self {
            display,
            root,
            window,
            cursor: 0,
            opcode: 0,
            select,
            grab: load!(x11, c"XGrabPointer", Grab),
            ungrab: load!(x11, c"XUngrabPointer", Ungrab),
            get: load!(x11, c"XGetEventData", CookieCall),
            free: load!(x11, c"XFreeEventData", FreeCookie),
            free_cursor: load!(x11, c"XFreeCursor", FreeCursor),
            focus: load!(x11, c"XGetInputFocus", Focus),
            active: Cell::new(false),
        };
        // Announce XI2.2: XI2.0 compatibility suppresses raw events during
        // grabs on some servers. The reply can negotiate an older XI2 server;
        // we still select only the original RawMotion event and fields.
        let (mut event, mut error, mut major, mut minor) = (0, 0, 2, 2);
        // SAFETY: owned live display, valid C output storage. Cursor pixmap is
        // zero/transparent and is freed on both success and cursor failure.
        unsafe {
            if extension(
                display,
                c"XInputExtension".as_ptr(),
                &mut this.opcode,
                &mut event,
                &mut error,
            ) == 0
                || version(display, &mut major, &mut minor) != 0
            {
                return Err("native relative mouse requires XI2".into());
            }
            let pixmap = bitmap(display, window, [0 as c_char].as_ptr(), 1, 1);
            if pixmap == 0 {
                return Err("cannot allocate native invisible cursor".into());
            }
            let black = Color {
                pixel: 0,
                red: 0,
                green: 0,
                blue: 0,
                flags: 0,
                pad: 0,
            };
            this.cursor = cursor(display, pixmap, pixmap, &black, &black, 0, 0);
            free_pixmap(display, pixmap);
            if this.cursor == 0 {
                return Err("cannot create native invisible cursor".into());
            }
        }
        Ok(this)
    }
    pub fn set(&self, active: bool) -> Result<(), String> {
        if self.active.get() == active {
            return Ok(());
        }
        if active {
            let (mut focused, mut revert) = (0, 0);
            // SAFETY: owned display and valid output storage.
            unsafe {
                (self.focus)(self.display, &mut focused, &mut revert);
            }
            if focused != self.window {
                return Err("native relative pointer requires window focus".into());
            }
        }
        let mut bits = [0, 0, if active { 2 } else { 0 }]; // XI_RawMotion=17, all master devices=1
        let mut mask = Mask {
            device: 1,
            len: 3,
            mask: bits.as_mut_ptr(),
        };
        // SAFETY: owned display/window, mask borrowed for synchronous request.
        unsafe {
            if (self.select)(self.display, self.root, &mut mask, 1) != 0 {
                return Err("XISelectEvents failed".into());
            }
            if active
                && (self.grab)(
                    self.display,
                    self.window,
                    0,
                    4 | 8 | 64,
                    1,
                    1,
                    self.window,
                    self.cursor,
                    0,
                ) != 0
            {
                bits.fill(0);
                (self.select)(self.display, self.root, &mut mask, 1);
                return Err("XGrabPointer refused native relative mouse".into());
            }
            if !active {
                (self.ungrab)(self.display, 0);
            }
        }
        self.active.set(active);
        Ok(())
    }
    /// `event` points to a live XEvent; caller checks type GenericEvent first.
    pub unsafe fn event(&self, event: *mut c_void) -> Result<Option<WindowEvent>, String> {
        // SAFETY: XEvent is big/aligned enough for its GenericEventCookie member.
        let cookie = unsafe { &mut *event.cast::<Cookie>() };
        if cookie.extension != self.opcode || cookie.event_type != 17 {
            return Ok(None);
        }
        // SAFETY: same display and cookie returned by XNextEvent. Data is freed
        // after copying even when malformed/non-finite motion is rejected.
        unsafe {
            if (self.get)(self.display, cookie) == 0 {
                return Err("XGetEventData failed".into());
            }
            let result = (|| {
                if !self.active.get() {
                    return Ok(None);
                }
                let raw = cookie
                    .data
                    .cast::<Raw>()
                    .as_ref()
                    .ok_or("missing XI2 raw event")?;
                if !(0..=32).contains(&raw.valuators.len) {
                    return Err("invalid XI2 valuator mask".into());
                }
                if raw.valuators.len == 0 {
                    return Ok(None);
                }
                if raw.valuators.mask.is_null() || raw.raw_values.is_null() {
                    return Err("missing XI2 raw valuators".into());
                }
                let mask = *raw.valuators.mask;
                let mut at = 0;
                let mut delta = [0.; 2];
                for (axis, value) in delta.iter_mut().enumerate() {
                    if mask & (1 << axis) != 0 {
                        *value = *raw.raw_values.add(at);
                        at += 1;
                    }
                }
                if !delta.iter().all(|v| v.is_finite()) {
                    return Err("non-finite native relative motion".into());
                }
                Ok(Some(WindowEvent::RelativeMotion {
                    x: delta[0],
                    y: delta[1],
                }))
            })();
            (self.free)(self.display, cookie);
            result
        }
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        let _ = self.set(false);
        if self.cursor != 0 {
            // SAFETY: Context drops Input before closing its display.
            unsafe {
                (self.free_cursor)(self.display, self.cursor);
            }
        }
    }
}
