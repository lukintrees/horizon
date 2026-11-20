//! Windows 7 raw relative mouse. No cursor-edge deltas, warp loop, or newer API.
use super::{
    GetCapture, GetClientRect, Handle, LoadCursorW, Rect, ReleaseCapture, SetCapture, State,
};
use crate::WindowEvent;
use std::ptr;
#[repr(C)]
struct Device {
    page: u16,
    usage: u16,
    flags: u32,
    target: Handle,
}
#[repr(C)]
struct Header {
    kind: u32,
    size: u32,
    device: Handle,
    wparam: usize,
}
#[repr(C)]
struct Mouse {
    flags: u16,
    padding: u16,
    buttons: u32,
    raw_buttons: u32,
    x: i32,
    y: i32,
    extra: u32,
}
#[repr(C)]
struct Input {
    header: Header,
    mouse: Mouse,
}
#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}
#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterRawInputDevices(devices: *const Device, count: u32, size: u32) -> i32;
    fn GetRawInputData(
        input: Handle,
        command: u32,
        data: *mut Input,
        size: *mut u32,
        header_size: u32,
    ) -> u32;
    fn ClipCursor(rect: *const Rect) -> i32;
    fn GetClipCursor(rect: *mut Rect) -> i32;
    fn ClientToScreen(window: Handle, point: *mut Point) -> i32;
    fn GetForegroundWindow() -> Handle;
    fn SetForegroundWindow(window: Handle) -> i32;
    fn SetCursor(cursor: Handle) -> Handle;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLastError() -> u32;
}
const _: () = {
    assert!(size_of::<Header>() == if size_of::<usize>() == 4 { 16 } else { 24 });
    assert!(size_of::<Mouse>() == 24);
    assert!(std::mem::offset_of!(Mouse, x) == 12);
    assert!(size_of::<Input>() == if size_of::<usize>() == 4 { 40 } else { 48 });
    assert!(size_of::<Device>() == if size_of::<usize>() == 4 { 12 } else { 16 });
};
pub(super) fn set(window: Handle, state: &State, relative: bool) -> Result<(), String> {
    if !relative && !state.relative.get() {
        return Ok(());
    }
    if !relative {
        release(window, state);
        return Ok(());
    }
    // SAFETY: owned thread-confined live window and correctly typed storage.
    unsafe {
        if state.size.get().contains(&0) {
            return Err("native window cannot grab a minimized pointer".into());
        }
        // Already ours: nothing to do. Windows ignores a redundant `SetCapture`, but Wine
        // delivers `WM_CAPTURECHANGED` for it, and the window procedure answers that by
        // releasing, so re-issuing the grab would drop a capture we already held.
        if state.relative.get() && GetCapture() == window {
            return Ok(());
        }
        // A raw-mouse grab needs our window to be the foreground window: WM_INPUT
        // only reaches a foreground window, and activating it is also what starts
        // the mouse look for a person who just clicked the window. `wgl.rs` shows
        // the window with SW_SHOWNOACTIVATE, so nothing else ever activates it and
        // a grab requested now would be refused on every run. Only try when the
        // pointer grab is asked for (in-world), never while the menus are up.
        if GetForegroundWindow() != window {
            SetForegroundWindow(window);
            if GetForegroundWindow() != window {
                return Err("native window cannot grab inactive pointer".into());
            }
        }
        let mut rect = Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let mut origin = Point { x: 0, y: 0 };
        if GetClientRect(window, &mut rect) == 0 || ClientToScreen(window, &mut origin) == 0 {
            return Err("cannot locate native pointer clip".into());
        }
        rect.right += origin.x;
        rect.bottom += origin.y;
        rect.left = origin.x;
        rect.top = origin.y;
        let device = Device {
            page: 1,
            usage: 2,
            flags: 0,
            target: window,
        };
        if RegisterRawInputDevices(&device, 1, size_of::<Device>() as u32) == 0 {
            return Err("RegisterRawInputDevices failed".into());
        }
        // A press the game has not grabbed yet already called `SetCapture` for itself
        // (`wgl.rs`'s window procedure). Taking the capture again from the window that holds
        // it is a redundant call: Windows ignores it, Wine answers with `WM_CAPTURECHANGED`,
        // and answering that by releasing is what dropped a first grab mid-session.
        if GetCapture() != window {
            let previous = SetCapture(window);
            if GetCapture() != window {
                release(window, state);
                return Err(format!(
                    "native pointer capture failed (SetCapture returned {previous:?}, error {})",
                    GetLastError()
                ));
            }
        }
        state.relative.set(true);
        if ClipCursor(&rect) == 0 {
            release(window, state);
            return Err(format!(
                "native pointer clip failed (error {})",
                GetLastError()
            ));
        }
        state
            .clip
            .set(Some([rect.left, rect.top, rect.right, rect.bottom]));
        SetCursor(ptr::null_mut());
    }
    Ok(())
}
pub(super) fn release(window: Handle, state: &State) {
    if !state.relative.replace(false) {
        return;
    }
    // Never unclip/deregister another native window's newer capture.
    // SAFETY: calls are synchronous on the owning window's thread.
    unsafe {
        if let Some(owned) = state.clip.take() {
            let mut current = Rect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if GetClipCursor(&mut current) != 0
                && owned == [current.left, current.top, current.right, current.bottom]
            {
                ClipCursor(ptr::null());
            }
        }
        if GetCapture() == window {
            let device = Device {
                page: 1,
                usage: 2,
                flags: 1,
                target: ptr::null_mut(),
            };
            RegisterRawInputDevices(&device, 1, size_of::<Device>() as u32);
            ReleaseCapture();
            SetCursor(LoadCursorW(ptr::null_mut(), 32512usize as *const u16));
        }
    }
}
pub(super) fn message(window: Handle, state: &State, message: u32, lparam: isize) -> Option<isize> {
    if message == 8 {
        release(window, state);
    }
    if message == 0x215 && unsafe { GetCapture() } != window {
        // A capture change that left the pointer to another window ends the grab. Wine
        // also sends this while the capture is still ours (a redundant `SetCapture` on
        // the same window), which is not a loss: releasing there would undo a live grab.
        release(window, state);
    }
    if !state.relative.get() {
        return None;
    }
    if message == 0x20 {
        // SAFETY: null selects the hidden cursor without changing ShowCursor's
        // process-global counter. Ordinary WM_SETCURSOR restores it on release.
        unsafe {
            SetCursor(ptr::null_mut());
        }
        return Some(1);
    }
    if message != 0xff {
        return None;
    }
    let mut input = std::mem::MaybeUninit::<Input>::zeroed();
    let mut size = size_of::<Input>() as u32;
    // SAFETY: fixed-size RAWINPUT mouse storage, correct header size; reject
    // any truncated/oversized/absolute-only packet instead of inventing deltas.
    unsafe {
        let read = GetRawInputData(
            lparam as Handle,
            0x10000003,
            input.as_mut_ptr(),
            &mut size,
            size_of::<Header>() as u32,
        );
        if read == size_of::<Input>() as u32 {
            let input = input.assume_init();
            if input.header.kind == 0 && input.mouse.flags & 1 == 0 {
                state.push(WindowEvent::RelativeMotion {
                    x: f64::from(input.mouse.x),
                    y: f64::from(input.mouse.y),
                });
                return None; // DefWindowProc performs WM_INPUT foreground cleanup.
            }
        }
    }
    release(window, state);
    state.push(WindowEvent::InputUnavailable);
    None
}
