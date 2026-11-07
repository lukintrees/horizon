//! Private Xlib/GLX ABI. All XIDs are native unsigned longs, never pointers.
use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
pub(super) type Display = *mut c_void;
pub(super) type Xid = c_ulong;
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct VisualInfo {
    pub visual: *mut c_void,
    pub id: Xid,
    pub screen: c_int,
    pub depth: c_int,
    pub class: c_int,
    pub red: c_ulong,
    pub green: c_ulong,
    pub blue: c_ulong,
    pub colormap_size: c_int,
    pub bits_per_rgb: c_int,
}
#[repr(C)]
#[derive(Default)]
pub(super) struct Attributes {
    pub background_pixmap: Xid,
    pub background_pixel: c_ulong,
    pub border_pixmap: Xid,
    pub border_pixel: c_ulong,
    pub bit_gravity: c_int,
    pub win_gravity: c_int,
    pub backing_store: c_int,
    pub backing_planes: c_ulong,
    pub backing_pixel: c_ulong,
    pub save_under: c_int,
    pub event_mask: c_long,
    pub do_not_propagate_mask: c_long,
    pub override_redirect: c_int,
    pub colormap: Xid,
    pub cursor: Xid,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct AnyEvent {
    pub kind: c_int,
    pub serial: c_ulong,
    pub sent: c_int,
    pub display: Display,
    pub window: Xid,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct KeyEvent {
    pub any: AnyEvent,
    pub root: Xid,
    pub subwindow: Xid,
    pub time: c_ulong,
    pub x: c_int,
    pub y: c_int,
    pub x_root: c_int,
    pub y_root: c_int,
    pub state: c_uint,
    pub code: c_uint,
    pub same_screen: c_int,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct MotionEvent {
    pub any: AnyEvent,
    pub root: Xid,
    pub subwindow: Xid,
    pub time: c_ulong,
    pub x: c_int,
    pub y: c_int,
    pub x_root: c_int,
    pub y_root: c_int,
    pub state: c_uint,
    pub is_hint: c_char,
    pub same_screen: c_int,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct ConfigureEvent {
    pub any: AnyEvent,
    pub window: Xid,
    pub x: c_int,
    pub y: c_int,
    pub width: c_int,
    pub height: c_int,
    pub border: c_int,
    pub above: Xid,
    pub override_redirect: c_int,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct ClientEvent {
    pub any: AnyEvent,
    pub message_type: Xid,
    pub format: c_int,
    pub data: [c_long; 5],
}
#[repr(C)]
pub(super) union Event {
    pub kind: c_int,
    pub any: AnyEvent,
    pub key: KeyEvent,
    pub motion: MotionEvent,
    pub configure: ConfigureEvent,
    pub client: ClientEvent,
    pub storage: [c_long; 24],
}
impl Default for Event {
    fn default() -> Self {
        Self { storage: [0; 24] }
    }
}
pub(super) type OpenDisplay = unsafe extern "C" fn(*const c_char) -> Display;
pub(super) type CloseDisplay = unsafe extern "C" fn(Display) -> c_int;
pub(super) type DefaultScreen = unsafe extern "C" fn(Display) -> c_int;
pub(super) type RootWindow = unsafe extern "C" fn(Display, c_int) -> Xid;
pub(super) type CreateColormap = unsafe extern "C" fn(Display, Xid, *mut c_void, c_int) -> Xid;
pub(super) type FreeColormap = unsafe extern "C" fn(Display, Xid) -> c_int;
pub(super) type CreateWindow = unsafe extern "C" fn(
    Display,
    Xid,
    c_int,
    c_int,
    c_uint,
    c_uint,
    c_uint,
    c_int,
    c_uint,
    *mut c_void,
    c_ulong,
    *mut Attributes,
) -> Xid;
pub(super) type WindowCall = unsafe extern "C" fn(Display, Xid) -> c_int;
pub(super) type ResizeWindow = unsafe extern "C" fn(Display, Xid, c_uint, c_uint) -> c_int;
pub(super) type Sync = unsafe extern "C" fn(Display, c_int) -> c_int;
pub(super) type Pending = unsafe extern "C" fn(Display) -> c_int;
pub(super) type NextEvent = unsafe extern "C" fn(Display, *mut Event) -> c_int;
pub(super) type InternAtom = unsafe extern "C" fn(Display, *const c_char, c_int) -> Xid;
pub(super) type SetProtocols = unsafe extern "C" fn(Display, Xid, *mut Xid, c_int) -> c_int;
pub(super) type StoreName = unsafe extern "C" fn(Display, Xid, *const c_char) -> c_int;
pub(super) type ChangeProperty =
    unsafe extern "C" fn(Display, Xid, Xid, Xid, c_int, c_int, *const u8, c_int) -> c_int;
pub(super) type Free = unsafe extern "C" fn(*mut c_void) -> c_int;
pub(super) type GetGeometry = unsafe extern "C" fn(
    Display,
    Xid,
    *mut Xid,
    *mut c_int,
    *mut c_int,
    *mut c_uint,
    *mut c_uint,
    *mut c_uint,
    *mut c_uint,
) -> c_int;
pub(super) type LookupString =
    unsafe extern "C" fn(*mut KeyEvent, *mut c_char, c_int, *mut Xid, *mut c_void) -> c_int;
pub(super) type LookupKeysym = unsafe extern "C" fn(*mut KeyEvent, c_int) -> Xid;
pub(super) type KeysymToUnicode = unsafe extern "C" fn(u32) -> u32;
pub(super) type QueryVersion = unsafe extern "C" fn(Display, *mut c_int, *mut c_int) -> c_int;
pub(super) type ChooseVisual = unsafe extern "C" fn(Display, c_int, *mut c_int) -> *mut VisualInfo;
pub(super) type CreateContext =
    unsafe extern "C" fn(Display, *mut VisualInfo, *mut c_void, c_int) -> *mut c_void;
pub(super) type DestroyContext = unsafe extern "C" fn(Display, *mut c_void);
pub(super) type MakeCurrent = unsafe extern "C" fn(Display, Xid, *mut c_void) -> c_int;
pub(super) type GetCurrent = unsafe extern "C" fn() -> *mut c_void;
pub(super) type SwapBuffers = unsafe extern "C" fn(Display, Xid);
pub(super) type GetProc = unsafe extern "C" fn(*const u8) -> *const c_void;
const _: () = {
    assert!(size_of::<Event>() == 24 * size_of::<c_long>());
    assert!(size_of::<VisualInfo>() == if size_of::<usize>() == 8 { 64 } else { 40 });
    assert!(size_of::<Attributes>() == if size_of::<usize>() == 8 { 112 } else { 60 });
    assert!(size_of::<KeyEvent>() == if size_of::<usize>() == 8 { 96 } else { 60 });
    assert!(std::mem::offset_of!(KeyEvent, code) == if size_of::<usize>() == 8 { 84 } else { 52 });
    assert!(size_of::<ConfigureEvent>() == if size_of::<usize>() == 8 { 88 } else { 52 });
    assert!(size_of::<ClientEvent>() == if size_of::<usize>() == 8 { 96 } else { 48 });
};

pub(super) type RefreshKeyboard = unsafe extern "C" fn(*mut Event) -> c_int;
