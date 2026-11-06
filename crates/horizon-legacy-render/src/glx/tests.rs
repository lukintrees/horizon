use super::*;
use crate::ContextImpl;
static DISPLAY_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
use std::ffi::{c_long, c_uint, c_ulong};
type SendEvent = unsafe extern "C" fn(Display, Xid, c_int, c_long, *mut Event) -> c_int;
type QueryTree =
    unsafe extern "C" fn(Display, Xid, *mut Xid, *mut Xid, *mut *mut Xid, *mut c_uint) -> c_int;
fn send(context: &Context, event: &mut Event, mask: c_long) {
    // SAFETY: exact XSendEvent ABI, owned window/display; initialized typed event.
    unsafe {
        let address = context._x11.symbol(c"XSendEvent").unwrap();
        let call = std::mem::transmute::<*const c_void, SendEvent>(address);
        assert_ne!(call(context.display, context.window, 0, mask, event), 0);
        (context.calls.sync)(context.display, 0);
    }
}
fn any(context: &Context, kind: c_int) -> AnyEvent {
    AnyEvent {
        kind,
        serial: 0,
        sent: 1,
        display: context.display,
        window: context.window,
    }
}
fn motion(context: &Context, x: c_int, y: c_int) -> Event {
    Event {
        motion: MotionEvent {
            any: any(context, 6),
            root: 0,
            subwindow: 0,
            time: 0,
            x,
            y,
            x_root: 0,
            y_root: 0,
            state: 0,
            is_hint: 0,
            same_screen: 1,
        },
    }
}
fn key(context: &Context, kind: c_int, code: u32, time: c_ulong) -> Event {
    Event {
        key: KeyEvent {
            any: any(context, kind),
            root: 0,
            subwindow: 0,
            time,
            x: 0,
            y: 0,
            x_root: 0,
            y_root: 0,
            state: 0,
            code,
            same_screen: 1,
        },
    }
}
fn children(context: &Context) -> u32 {
    // SAFETY: exact XQueryTree ABI; XFree only its returned allocation.
    unsafe {
        let address = context._x11.symbol(c"XQueryTree").unwrap();
        let call = std::mem::transmute::<*const c_void, QueryTree>(address);
        let (mut root, mut parent, mut list, mut count) = (0, 0, ptr::null_mut(), 0);
        let screen = std::mem::transmute::<*const c_void, DefaultScreen>(
            context._x11.symbol(c"XDefaultScreen").unwrap(),
        )(context.display);
        let window = (context.calls.root)(context.display, screen);
        assert_ne!(
            call(
                context.display,
                window,
                &mut root,
                &mut parent,
                &mut list,
                &mut count
            ),
            0
        );
        if !list.is_null() {
            (context.calls.free)(list.cast());
        }
        count
    }
}
#[test]
fn xlib_event_storage_covers_native_records_and_unicode_is_not_a_win32_key() {
    assert!(size_of::<Event>() >= size_of::<MotionEvent>());
    assert!(size_of::<Event>() >= size_of::<KeyEvent>());
    assert_ne!(
        WindowEvent::X11Key {
            keysym: 0xff1b,
            keycode: 9,
            pressed: true,
            repeat: false
        },
        WindowEvent::Key {
            virtual_key: 27,
            scan_code: 1,
            extended: false,
            pressed: true,
            repeat: false,
            repeat_count: 1
        }
    );
}
#[test]
#[ignore = "requires X11/GLX display (xvfb-run), Mesa llvmpipe and GL2 overrides"]
fn visible_glx_window_draws_resizes_pumps_bounded_input_and_reclaims_native_windows() {
    let _guard = DISPLAY_TEST.lock().unwrap();
    let context = Context::new([64; 2]).unwrap();
    assert!(context.is_windowed());
    context.make_current().unwrap();
    context.set_title("Horizon — GL2 окно").unwrap();
    context.poll_events().unwrap();
    // Real double-buffer drawable, not a pbuffer's completion re-labelled as a swap.
    // SAFETY: fixed GL1 functions on this current live desktop context.
    unsafe {
        let clear_color = std::mem::transmute::<
            *const c_void,
            unsafe extern "C" fn(f32, f32, f32, f32),
        >(context.address(c"glClearColor").unwrap());
        let clear = std::mem::transmute::<*const c_void, unsafe extern "C" fn(u32)>(
            context.address(c"glClear").unwrap(),
        );
        let read = std::mem::transmute::<
            *const c_void,
            unsafe extern "C" fn(c_int, c_int, c_int, c_int, u32, u32, *mut c_void),
        >(context.address(c"glReadPixels").unwrap());
        let get = std::mem::transmute::<*const c_void, unsafe extern "C" fn(u32) -> *const u8>(
            context.address(c"glGetString").unwrap(),
        );
        let version = CStr::from_ptr(get(0x1f02).cast()).to_string_lossy();
        let driver = CStr::from_ptr(get(0x1f01).cast()).to_string_lossy();
        crate::Context::qualify(&version, &driver).unwrap();
        assert!(driver.to_ascii_lowercase().contains("llvmpipe"), "{driver}");
        assert_eq!(version.split_whitespace().next(), Some("2.0"));
        clear_color(1., 0., 0., 1.);
        clear(0x4000);
        let mut rgba = [0u8; 4];
        read(32, 32, 1, 1, 0x1908, 0x1401, rgba.as_mut_ptr().cast());
        assert_eq!(rgba, [255, 0, 0, 255]);
    }
    assert!(context.present().unwrap());
    // Read the actual X window's front pixels, not another offscreen GL target.
    // SAFETY: exact Xlib image ABI; XDestroyImage releases only this XGetImage result.
    unsafe {
        type GetImage = unsafe extern "C" fn(
            Display,
            Xid,
            c_int,
            c_int,
            c_uint,
            c_uint,
            c_ulong,
            c_int,
        ) -> *mut c_void;
        type Pixel = unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_ulong;
        type DestroyImage = unsafe extern "C" fn(*mut c_void) -> c_int;
        (context.calls.sync)(context.display, 0);
        let get = std::mem::transmute::<*const c_void, GetImage>(
            context._x11.symbol(c"XGetImage").unwrap(),
        );
        let pixel =
            std::mem::transmute::<*const c_void, Pixel>(context._x11.symbol(c"XGetPixel").unwrap());
        let destroy = std::mem::transmute::<*const c_void, DestroyImage>(
            context._x11.symbol(c"XDestroyImage").unwrap(),
        );
        let image = get(context.display, context.window, 0, 0, 64, 64, !0, 2);
        assert!(!image.is_null());
        let color = pixel(image, 32, 32);
        destroy(image);
        assert_eq!(
            color & 0xffffff,
            0xff0000,
            "X11 visible front buffer was not red"
        );
    }

    context.resize([96, 48]).unwrap();
    assert_eq!(context.drawable_size(), Some([96, 48]));
    let events = context.poll_events().unwrap();
    assert!(events.events.contains(&WindowEvent::Resized([96, 48])));
    // Text uses the xkb Unicode mapping; non-Latin X11 keysyms are not bytes.
    // SAFETY: stable xkbcommon keysym conversion API, no borrowed pointers.
    assert_eq!(
        char::from_u32(unsafe { (context.calls.unicode)(0x06d1) }),
        Some('я')
    );
    send(&context, &mut key(&context, 2, 38, 100), 1); // standard Xvfb keycode for A
    send(&context, &mut key(&context, 3, 38, 101), 2);
    send(&context, &mut key(&context, 2, 38, 101), 1); // core autorepeat pair
    let events = context.poll_events().unwrap();
    let keys: Vec<_> = events
        .events
        .iter()
        .filter(|e| matches!(e, WindowEvent::X11Key { .. }))
        .collect();
    assert_eq!(keys.len(), 2, "{events:?}");
    assert!(matches!(
        keys[0],
        WindowEvent::X11Key {
            pressed: true,
            repeat: false,
            ..
        }
    ));
    assert!(matches!(
        keys[1],
        WindowEvent::X11Key {
            pressed: true,
            repeat: true,
            ..
        }
    ));
    assert!(
        events
            .events
            .iter()
            .any(|e| matches!(e, WindowEvent::Text { character: 'a', .. }))
    );
    let mut focus = Event::default();
    focus.any = any(&context, 10);
    send(&context, &mut focus, 1 << 21);
    let events = context.poll_events().unwrap();
    assert!(events.events.contains(&WindowEvent::Focused(false)));
    assert!(events.events.contains(&WindowEvent::PointerCaptureLost));
    send(&context, &mut key(&context, 2, 38, 102), 1);
    assert!(
        context
            .poll_events()
            .unwrap()
            .events
            .iter()
            .any(|e| matches!(e, WindowEvent::X11Key { repeat: false, .. }))
    );
    // Bound native messages per pump; all deferred messages remain queued.
    for n in 0..600 {
        send(&context, &mut motion(&context, n, -3), 1 << 6);
    }
    let events = context.poll_events().unwrap();
    assert!(events.pending_messages);
    assert_eq!(events.lost_events, 0);
    assert!(events.events.len() <= 256);
    for _ in 0..4 {
        context.poll_events().unwrap();
    }
    assert!(!context.poll_events().unwrap().pending_messages);
    // Hide/map are suspension/restoration, never zero-sized framebuffer allocations.
    // SAFETY: fixed XUnmapWindow ABI and owned live window.
    unsafe {
        let unmap = std::mem::transmute::<*const c_void, WindowCall>(
            context._x11.symbol(c"XUnmapWindow").unwrap(),
        );
        unmap(context.display, context.window);
        (context.calls.sync)(context.display, 0);
    }
    assert!(
        context
            .poll_events()
            .unwrap()
            .events
            .contains(&WindowEvent::Resized([0; 2]))
    );
    assert_eq!(context.drawable_size(), Some([0; 2]));
    assert!(!context.present().unwrap());
    // SAFETY: same owned window, map restores its existing drawable/context.
    unsafe {
        (context.calls.map)(context.display, context.window);
        (context.calls.sync)(context.display, 0);
    }
    context.poll_events().unwrap();
    assert_eq!(context.drawable_size(), Some([96, 48]));
    let baseline = children(&context);
    for _ in 0..8 {
        let other = Context::new([32; 2]).unwrap();
        context.make_current().unwrap();
        drop(other);
        // SAFETY: glXGetCurrentContext has no pointer arguments.
        assert_eq!(unsafe { (context.calls.get_current)() }, context.context);
    }
    assert_eq!(
        children(&context),
        baseline,
        "native window leaked across owner drops"
    );
    let mut close = Event {
        client: ClientEvent {
            any: any(&context, 33),
            message_type: context.protocols,
            format: 32,
            data: [context.delete as c_long, 0, 0, 0, 0],
        },
    };
    send(&context, &mut close, 0);
    assert!(context.poll_events().unwrap().close_requested);
    assert!(context.poll_events().unwrap().close_requested); // persistent until owner drop
    // Window remains alive on close request so Game can finish its lifecycle.
    assert_eq!(context.geometry().unwrap(), [96, 48]);
    // Also exercise an actual server-side destruction notification. Public
    // operations must not call Xlib with a dead XID (default BadWindow is fatal).
    // SAFETY: destroy our own live X window once, then let its notification
    // transfer cleanup responsibility before the owner drops.
    unsafe {
        (context.calls.destroy_window)(context.display, context.window);
        (context.calls.sync)(context.display, 0);
    }
    assert!(context.poll_events().unwrap().close_requested);
    assert!(context.destroyed.get());
    assert_eq!(context.drawable_size(), Some([0; 2]));
    assert!(context.make_current().is_err());
    assert!(context.resize([32; 2]).is_err());
    assert!(context.set_title("already closed").is_err());
    assert!(!context.present().unwrap());
}

#[test]
#[ignore = "requires Xvfb GLX, XI2 and libXtst; real server input, not framebuffer-only"]
fn visible_glx_physical_keys_and_unbounded_raw_motion_release_on_focus_and_drop() {
    let _guard = DISPLAY_TEST.lock().unwrap();
    use horizon_render_api::platform_contract::PointerMode;
    let context = Context::new([64; 2]).unwrap();
    type Focus = unsafe extern "C" fn(Display, Xid, c_int, c_ulong) -> c_int;
    type FakeMotion = unsafe extern "C" fn(Display, c_int, c_int, c_ulong) -> c_int;
    type FakeKey = unsafe extern "C" fn(Display, c_uint, c_int, c_ulong) -> c_int;
    let xtest = Library::open(c"libXtst.so.6").unwrap();
    // SAFETY: documented Xlib/XTest ABI, same owned display/window. XTest
    // injects real server device events; no synthetic winit IDs or raw cookie.
    unsafe {
        let focus = std::mem::transmute::<*const c_void, Focus>(
            context._x11.symbol(c"XSetInputFocus").unwrap(),
        );
        let motion = std::mem::transmute::<*const c_void, FakeMotion>(
            xtest.symbol(c"XTestFakeRelativeMotionEvent").unwrap(),
        );
        let key = std::mem::transmute::<*const c_void, FakeKey>(
            xtest.symbol(c"XTestFakeKeyEvent").unwrap(),
        );
        focus(context.display, context.window, 1, 0);
        (context.calls.sync)(context.display, 0);
        context.poll_events().unwrap();
        let w = context
            .usages
            .borrow()
            .iter()
            .position(|&u| u == 0x1a)
            .unwrap() as c_uint;
        assert_ne!(key(context.display, w, 1, 0), 0);
        assert_ne!(key(context.display, w, 0, 0), 0);
        (context.calls.sync)(context.display, 0);
        let events = context.poll_events().unwrap();
        for pressed in [true, false] {
            assert!(
                events.events.contains(&WindowEvent::PhysicalKey {
                    usage: 0x1a,
                    pressed,
                    repeat: false
                }),
                "{events:?}"
            );
        }
        context.set_pointer_mode(PointerMode::Relative).unwrap();
        for _ in 0..3 {
            assert_ne!(motion(context.display, 1024, -1024, 0), 0);
        }
        (context.calls.sync)(context.display, 0);
        let events = context.poll_events().unwrap();
        let (mut x, mut y) = (0., 0.);
        for event in &events.events {
            if let WindowEvent::RelativeMotion { x: dx, y: dy } = event {
                x += dx;
                y += dy;
            }
        }
        assert!(
            x >= 3072. && y <= -3072.,
            "raw motion must not clamp at the 64px cursor edge: {events:?}"
        );
        context.set_pointer_mode(PointerMode::Free).unwrap();
        assert_ne!(motion(context.display, 8, 8, 0), 0);
        (context.calls.sync)(context.display, 0);
        assert!(
            !context
                .poll_events()
                .unwrap()
                .events
                .iter()
                .any(|e| matches!(e, WindowEvent::RelativeMotion { .. }))
        );
        context.set_pointer_mode(PointerMode::Relative).unwrap();
        focus(context.display, context.root, 1, 0);
        (context.calls.sync)(context.display, 0);
        let events = context.poll_events().unwrap();
        assert!(events.events.contains(&WindowEvent::PointerCaptureLost));
        assert!(
            context.set_pointer_mode(PointerMode::Relative).is_err(),
            "an inactive window must not steal capture"
        );
        focus(context.display, context.window, 1, 0);
        (context.calls.sync)(context.display, 0);
        context.poll_events().unwrap();
        context.set_pointer_mode(PointerMode::Relative).unwrap();
    }
    drop(context); // must release an active grab before closing its Display
    let next = Context::new([64; 2]).unwrap();
    // SAFETY: real second window; successful grab proves previous teardown did
    // not leave a process-global pointer lock behind.
    unsafe {
        let focus = std::mem::transmute::<*const c_void, Focus>(
            next._x11.symbol(c"XSetInputFocus").unwrap(),
        );
        focus(next.display, next.window, 1, 0);
        (next.calls.sync)(next.display, 0);
    }
    next.poll_events().unwrap();
    next.set_pointer_mode(PointerMode::Relative).unwrap();
    next.set_pointer_mode(PointerMode::Free).unwrap();
}
