//! Events from the native WGL/GLX windows. No windowing crate.
//! Key is raw Win32 metadata; X11Key is raw X11 metadata, neither is a gameplay binding.
//! Text is Unicode (GLX initial adapter has no XIM/preedit/compose support).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    X1,
    X2,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowEvent {
    Resized([u32; 2]),
    Focused(bool),
    CloseRequested,
    Key {
        virtual_key: u32,
        scan_code: u8,
        extended: bool,
        pressed: bool,
        repeat: bool,
        repeat_count: u16,
    },
    /// X11 keycode is NOT a Win32 scancode or a portable physical binding.
    X11Key {
        keysym: u32,
        keycode: u8,
        pressed: bool,
        repeat: bool,
    },
    Text {
        character: char,
        repeat_count: u16,
    },
    PointerMoved {
        x: i32,
        y: i32,
    },
    MouseButton {
        button: MouseButton,
        pressed: bool,
        x: i32,
        y: i32,
    },
    /// Signed Win32 wheel units; 120 is one detent. Horizontal is not vertical.
    Wheel {
        horizontal: bool,
        delta: i16,
    },
    PointerCaptureLost,
}
#[derive(Debug, Default)]
pub struct WindowEvents {
    pub events: Vec<WindowEvent>,
    /// Bounded queue overflow: callers MUST release/reset held input state.
    pub lost_events: u64,
    /// Persistent even if a close event overflowed the queue. Drop the owner to
    /// destroy the window; polling never destroys a live renderer's drawable.
    pub close_requested: bool,
    /// Polling is bounded; another poll can drain the remaining native messages.
    pub pending_messages: bool,
}

#[cfg(any(windows, target_os = "linux", test))]
mod state {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
    };
    const CAPACITY: usize = 256;
    pub(crate) struct State {
        pub size: Cell<[u32; 2]>,
        #[cfg(windows)]
        pub programmatic_resize: Cell<bool>,
        pub buttons: Cell<u8>,
        queue: RefCell<VecDeque<WindowEvent>>,
        lost: Cell<u64>,
        closed: Cell<bool>,
        #[cfg(any(windows, test))]
        surrogate: Cell<Option<(u16, u16)>>,
    }
    impl State {
        pub fn new(size: [u32; 2]) -> Self {
            Self {
                size: Cell::new(size),
                #[cfg(windows)]
                programmatic_resize: Cell::new(false),
                buttons: Cell::new(0),
                queue: RefCell::new(VecDeque::with_capacity(CAPACITY)),
                lost: Cell::new(0),
                closed: Cell::new(false),
                #[cfg(any(windows, test))]
                surrogate: Cell::new(None),
            }
        }
        pub(crate) fn push(&self, event: WindowEvent) {
            if event == WindowEvent::CloseRequested {
                self.closed.set(true);
            }
            let mut queue = self.queue.borrow_mut();
            // Coalesce only adjacent absolute motion/size events; never move an
            // event across key/button boundaries or silently drop key releases.
            if matches!(
                (queue.back(), event),
                (
                    Some(WindowEvent::PointerMoved { .. }),
                    WindowEvent::PointerMoved { .. }
                ) | (Some(WindowEvent::Resized(_)), WindowEvent::Resized(_))
            ) {
                *queue.back_mut().unwrap() = event;
            } else if queue.len() < CAPACITY {
                queue.push_back(event);
            } else {
                self.lost.set(self.lost.get().saturating_add(1));
            }
        }
        pub fn resized(&self, size: [u32; 2]) {
            self.size.set(size);
            self.push(WindowEvent::Resized(size));
        }
        #[cfg(any(windows, test))]
        fn text(&self, unit: u16, repeat_count: u16) {
            if let Some((high, count)) = self.surrogate.take() {
                if (0xdc00..=0xdfff).contains(&unit) {
                    let scalar =
                        0x10000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(unit) - 0xdc00);
                    self.push(WindowEvent::Text {
                        character: char::from_u32(scalar).unwrap(),
                        repeat_count,
                    });
                    return;
                }
                self.push(WindowEvent::Text {
                    character: char::REPLACEMENT_CHARACTER,
                    repeat_count: count,
                });
            }
            if (0xd800..=0xdbff).contains(&unit) {
                self.surrogate.set(Some((unit, repeat_count)));
            } else {
                self.push(WindowEvent::Text {
                    character: char::from_u32(u32::from(unit))
                        .unwrap_or(char::REPLACEMENT_CHARACTER),
                    repeat_count,
                });
            }
        }
        /// None delegates to DefWindowProc (in particular Alt+F4/system keys).
        /// No native calls occur with the queue borrowed, including reentrancy.
        #[cfg(any(windows, test))]
        pub fn message(&self, message: u32, wparam: usize, lparam: isize) -> Option<isize> {
            let (x, y) = (
                i32::from(lparam as u16 as i16),
                i32::from((lparam as usize >> 16) as u16 as i16),
            );
            match message {
                0x0010 => {
                    self.push(WindowEvent::CloseRequested);
                    return Some(0);
                }
                0x0005 => {
                    let size = if wparam == 1 {
                        [0; 2]
                    } else {
                        [
                            u32::from(lparam as u16),
                            ((lparam as usize >> 16) & 0xffff) as u32,
                        ]
                    };
                    self.resized(size);
                }
                0x0007 | 0x0008 => {
                    if message == 8 {
                        self.surrogate.set(None);
                        self.buttons.set(0);
                    }
                    self.push(WindowEvent::Focused(message == 7));
                }
                0x0100 | 0x0101 | 0x0104 | 0x0105 => {
                    let pressed = matches!(message, 0x0100 | 0x0104);
                    self.push(WindowEvent::Key {
                        virtual_key: wparam as u32,
                        scan_code: ((lparam as usize >> 16) & 255) as u8,
                        extended: lparam as usize & (1 << 24) != 0,
                        pressed,
                        repeat: pressed && lparam as usize & (1 << 30) != 0,
                        repeat_count: (lparam as u16).max(1),
                    });
                }
                0x0102 => {
                    self.text(wparam as u16, (lparam as u16).max(1));
                    return Some(0);
                }
                0x0109 => {
                    if wparam == 0xffff {
                        return Some(1);
                    }
                    if let Some(c) = char::from_u32(wparam as u32) {
                        self.surrogate.set(None);
                        self.push(WindowEvent::Text {
                            character: c,
                            repeat_count: (lparam as u16).max(1),
                        });
                    }
                    return Some(0);
                }
                0x0200 => self.push(WindowEvent::PointerMoved { x, y }),
                0x0201 | 0x0202 | 0x0204 | 0x0205 | 0x0207 | 0x0208 | 0x020b | 0x020c => {
                    let (button, bit) = match message {
                        0x0201 | 0x0202 => (MouseButton::Left, 1),
                        0x0204 | 0x0205 => (MouseButton::Right, 2),
                        0x0207 | 0x0208 => (MouseButton::Middle, 4),
                        _ => match (wparam >> 16) & 0xffff {
                            1 => (MouseButton::X1, 8),
                            2 => (MouseButton::X2, 16),
                            _ => return None,
                        },
                    };
                    let pressed = matches!(message, 0x0201 | 0x0204 | 0x0207 | 0x020b);
                    self.buttons.set(if pressed {
                        self.buttons.get() | bit
                    } else {
                        self.buttons.get() & !bit
                    });
                    self.push(WindowEvent::MouseButton {
                        button,
                        pressed,
                        x,
                        y,
                    });
                    if matches!(message, 0x020b | 0x020c) {
                        return Some(1);
                    }
                }
                0x020a | 0x020e => self.push(WindowEvent::Wheel {
                    horizontal: message == 0x020e,
                    delta: (wparam >> 16) as u16 as i16,
                }),
                0x0215 => {
                    self.buttons.set(0);
                    self.push(WindowEvent::PointerCaptureLost);
                }
                _ => (),
            }
            None
        }
        pub fn take(&self, pending_messages: bool) -> WindowEvents {
            WindowEvents {
                events: self.queue.borrow_mut().drain(..).collect(),
                lost_events: self.lost.replace(0),
                close_requested: self.closed.get(),
                pending_messages,
            }
        }
    }
}
#[cfg(any(windows, target_os = "linux"))]
pub(crate) use state::State;

#[cfg(test)]
mod tests {
    use super::{state::State, *};
    #[test]
    fn win32_signed_pointer_wheel_and_physical_key_bits() {
        let state = State::new([64; 2]);
        state.message(
            0x0200,
            0,
            (((-4i16 as u16 as usize) << 16) | (-7i16 as u16 as usize)) as isize,
        );
        state.message(0x020e, (-120i16 as u16 as usize) << 16, 0);
        state.message(0x0104, 0xa3, (0x1d << 16) | (1 << 24) | (1 << 30));
        state.message(0x0105, 0xa3, (0x1d << 16) | (1 << 24) | (1 << 30));
        assert_eq!(
            state.take(false).events,
            [
                WindowEvent::PointerMoved { x: -7, y: -4 },
                WindowEvent::Wheel {
                    horizontal: true,
                    delta: -120
                },
                WindowEvent::Key {
                    virtual_key: 0xa3,
                    scan_code: 0x1d,
                    extended: true,
                    pressed: true,
                    repeat: true,
                    repeat_count: 1
                },
                WindowEvent::Key {
                    virtual_key: 0xa3,
                    scan_code: 0x1d,
                    extended: true,
                    pressed: false,
                    repeat: false,
                    repeat_count: 1
                }
            ]
        );
    }
    #[test]
    fn unicode_surrogates_focus_and_close_are_not_raw_code_units() {
        let state = State::new([64; 2]);
        for unit in [0x0416, 0xd83c, 0xdf0d, 0xd800, 65, 0xdc00] {
            state.message(0x0102, unit, 0);
        }
        state.message(0x0102, 0xd800, 0);
        state.message(8, 0, 0);
        state.message(0x0102, 66, 0);
        assert_eq!(state.message(0x0109, 0xffff, 0), Some(1));
        assert_eq!(state.message(0x0010, 0, 0), Some(0));
        let events = state.take(false);
        assert!(events.close_requested);
        assert_eq!(
            events.events,
            [
                WindowEvent::Text {
                    character: 'Ж',
                    repeat_count: 1
                },
                WindowEvent::Text {
                    character: '🌍',
                    repeat_count: 1
                },
                WindowEvent::Text {
                    character: '\u{fffd}',
                    repeat_count: 1
                },
                WindowEvent::Text {
                    character: 'A',
                    repeat_count: 1
                },
                WindowEvent::Text {
                    character: '\u{fffd}',
                    repeat_count: 1
                },
                WindowEvent::Focused(false),
                WindowEvent::Text {
                    character: 'B',
                    repeat_count: 1
                },
                WindowEvent::CloseRequested
            ]
        );
        assert!(state.take(false).close_requested);
    }
    #[test]
    fn bounded_events_coalesce_only_adjacent_motion_and_never_hide_overflow() {
        let state = State::new([64; 2]);
        state.message(0x0200, 0, 1);
        state.message(0x0200, 0, 2);
        state.message(0x0100, 65, 0);
        state.message(0x0200, 0, 3);
        assert_eq!(state.take(false).events.len(), 3);
        for _ in 0..300 {
            state.message(0x0100, 65, 0);
        }
        state.message(0x0010, 0, 0);
        let events = state.take(true);
        assert_eq!(events.events.len(), 256);
        assert_eq!(events.lost_events, 45);
        assert!(events.close_requested && events.pending_messages);
        assert_eq!(state.take(false).lost_events, 0);
        state.message(5, 1, 0);
        assert_eq!(state.size.get(), [0; 2]);
        state.message(5, 0, ((48 << 16) | 96) as isize);
        assert_eq!(state.size.get(), [96, 48]);
        assert_eq!(state.take(false).events, [WindowEvent::Resized([96, 48])]);
    }
    #[test]
    fn capture_loss_focus_and_coalesced_keyboard_repeats_are_explicit() {
        let state = State::new([64; 2]);
        state.message(0x020b, 2 << 16, 0);
        assert_eq!(state.buttons.get(), 16);
        state.message(0x0215, 0, 0);
        assert_eq!(state.buttons.get(), 0);
        state.message(0x0204, 0, 0);
        state.message(8, 0, 0);
        assert_eq!(state.buttons.get(), 0);
        state.take(false);
        state.message(0x0100, 65, (1 << 30) | 3);
        state.message(0x0102, 65, 7);
        assert_eq!(
            state.take(false).events,
            [
                WindowEvent::Key {
                    virtual_key: 65,
                    scan_code: 0,
                    extended: false,
                    pressed: true,
                    repeat: true,
                    repeat_count: 3
                },
                WindowEvent::Text {
                    character: 'A',
                    repeat_count: 7
                }
            ]
        );
    }
}
