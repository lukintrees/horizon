//! The native desktop-GL window's input, turned into the game's own input events. A key is a USB
//! HID usage (the producer normalises WGL's scan code and X11's keycode to one), a mouse button is
//! the producer's own enum, and text arrives on its own callback: none of it is a winit identity,
//! so nothing here invents a window or a device id.
// game-module-cfg: feature = "legacy-gl"
use super::input_event::{Event as InputEvent, KeyEvent};
use super::*;
use horizon_client::renderer_facade::{NativeMouseButton as Button, NativeWindowEvent as Event};
use winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    event::{DeviceId, ElementState, Ime, MouseButton, MouseScrollDelta, TouchPhase},
    keyboard::{Key, KeyCode, NativeKey, NativeKeyCode, PhysicalKey},
};

/// USB HID usage (page 07) to the code the key bindings use. A usage the game names nothing for
/// is not a key: the key's own text still arrives on the producer's character callback.
fn key_code(usage: u16) -> Option<KeyCode> {
    use KeyCode::*;
    const LETTERS: [KeyCode; 26] = [
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO,
        KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
    ];
    const DIGITS: [KeyCode; 10] = [
        Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9, Digit0,
    ];
    const FUNCTION: [KeyCode; 12] = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];
    Some(match usage {
        0x04..=0x1d => LETTERS[usize::from(usage - 0x04)],
        0x1e..=0x27 => DIGITS[usize::from(usage - 0x1e)],
        0x3a..=0x45 => FUNCTION[usize::from(usage - 0x3a)],
        0x28 => Enter,
        0x29 => Escape,
        0x2a => Backspace,
        0x2b => Tab,
        0x2c => Space,
        0x2d => Minus,
        0x2e => Equal,
        0x2f => BracketLeft,
        0x30 => BracketRight,
        0x31 => Backslash,
        0x33 => Semicolon,
        0x34 => Quote,
        0x35 => Backquote,
        0x36 => Comma,
        0x37 => Period,
        0x38 => Slash,
        0x39 => CapsLock,
        0x46 => PrintScreen,
        0x47 => ScrollLock,
        0x48 => Pause,
        0x49 => Insert,
        0x4a => Home,
        0x4b => PageUp,
        0x4c => Delete,
        0x4d => End,
        0x4e => PageDown,
        0x4f => ArrowRight,
        0x50 => ArrowLeft,
        0x51 => ArrowDown,
        0x52 => ArrowUp,
        0x53 => NumLock,
        0x54 => NumpadDivide,
        0x55 => NumpadMultiply,
        0x56 => NumpadSubtract,
        0x57 => NumpadAdd,
        0x58 => NumpadEnter,
        0x59 => Numpad1,
        0x5a => Numpad2,
        0x5b => Numpad3,
        0x5c => Numpad4,
        0x5d => Numpad5,
        0x5e => Numpad6,
        0x5f => Numpad7,
        0x60 => Numpad8,
        0x61 => Numpad9,
        0x62 => Numpad0,
        0x63 => NumpadDecimal,
        0x64 => IntlBackslash,
        0x65 => ContextMenu,
        0xe0 => ControlLeft,
        0xe1 => ShiftLeft,
        0xe2 => AltLeft,
        0xe3 => SuperLeft,
        0xe4 => ControlRight,
        0xe5 => ShiftRight,
        0xe6 => AltRight,
        0xe7 => SuperRight,
        _ => return None,
    })
}

fn state(pressed: bool) -> ElementState {
    if pressed {
        ElementState::Pressed
    } else {
        ElementState::Released
    }
}

fn button(button: Button) -> MouseButton {
    match button {
        Button::Left => MouseButton::Left,
        Button::Right => MouseButton::Right,
        Button::Middle => MouseButton::Middle,
        Button::X1 => MouseButton::Back,
        Button::X2 => MouseButton::Forward,
    }
}

/// Characters are a separate native callback. The key that opened a text field (T, the chat
/// slash) must not type its own character into it, unlike a key typed inside one.
#[derive(Default)]
pub(super) struct Router {
    suppress_text: bool,
}

impl Router {
    pub(super) fn clear(&mut self) {
        self.suppress_text = false;
    }

    fn key_finished(&mut self, was_typing: bool, is_typing: bool) {
        self.suppress_text = !was_typing && is_typing;
    }

    /// One producer event into the game's handlers. A press clears the typing gate (it may open
    /// or close a text field), and the character callback that follows it is dropped when the
    /// press opened one.
    pub(super) fn dispatch(
        &mut self,
        game: &mut Game<'_>,
        ctl: &dyn EventLoopCtl,
        event: Event,
    ) -> Result<()> {
        let key = matches!(event, Event::PhysicalKey { pressed: true, .. });
        if key || matches!(event, Event::MouseButton { pressed: true, .. }) {
            self.clear();
        }
        if matches!(event, Event::Text { .. }) && self.suppress_text {
            return Ok(());
        }
        let was_typing = key && game.text_field_focused();
        dispatch_event(game, ctl, event)?;
        if key {
            self.key_finished(was_typing, game.text_field_focused());
        }
        Ok(())
    }
}

fn dispatch_event(game: &mut Game<'_>, ctl: &dyn EventLoopCtl, event: Event) -> Result<()> {
    // The game keeps winit's event shapes so one handler reads both producers. The window this
    // producer drives is not a winit device: the handler never matches on the id, and a dummy one
    // stands in for the identity winit would have supplied.
    let device_id = DeviceId::dummy();
    let id = winit::window::WindowId::dummy();
    let input = match event {
        Event::PhysicalKey {
            usage,
            pressed,
            repeat,
        } => {
            // Every key counts as held while it is down, whether or not a binding names it: a
            // screen's own key handling (arrows, F3's debug keys) reads the set as well.
            if let Some(code) = key_code(usage) {
                game.press_modifier_key(code, pressed);
            }
            InputEvent::KeyboardInput {
                device_id,
                event: KeyEvent {
                    physical_key: key_code(usage).map_or(
                        PhysicalKey::Unidentified(NativeKeyCode::Unidentified),
                        PhysicalKey::Code,
                    ),                    logical_key: Key::Unidentified(NativeKey::Unidentified),
                    text: None,
                    state: state(pressed),
                    repeat,
                },
                is_synthetic: false,
            }
        }
        Event::PointerMoved { x, y } if !game.captured => InputEvent::CursorMoved {
            device_id,
            position: PhysicalPosition::new(f64::from(x), f64::from(y)),
        },
        Event::MouseButton {
            button: b,
            pressed,
            x,
            y,
        } => {
            // A click lands where the pointer is: a screen's widgets get the position first.
            if !game.captured {
                game.handle_window_event(
                    ctl,
                    id,
                    InputEvent::CursorMoved {
                        device_id,
                        position: PhysicalPosition::new(f64::from(x), f64::from(y)),
                    },
                    input_trace::InputOrigin::User,
                );
            }
            InputEvent::MouseInput {
                device_id,
                state: state(pressed),
                button: button(b),
            }
        }
        Event::Wheel { horizontal, delta } => {
            // One detent is 120 units, as Win32's wheel and vanilla's scroll count it.
            let lines = f32::from(delta) / 120.;
            InputEvent::MouseWheel {
                device_id,
                delta: MouseScrollDelta::LineDelta(
                    if horizontal { lines } else { 0. },
                    if horizontal { 0. } else { lines },
                ),
                phase: TouchPhase::Moved,
            }
        }
        Event::Text {
            character,
            repeat_count,
        } => {
            if repeat_count > 32 {
                return Err("native text repeat batch exceeds budget".into());
            }
            if character.is_control() || !game.types_text() {
                return Ok(());
            }
            let text = character.to_string().repeat(usize::from(repeat_count));
            InputEvent::Ime(Ime::Commit(text.into()))
        }
        Event::RelativeMotion { x, y } => {
            game.native_relative_motion(x, y);
            return Ok(());
        }
        Event::InputUnavailable => return Err("native relative mouse device is unsupported".into()),
        Event::Focused(focused) => InputEvent::Focused(focused),
        Event::Resized([width, height]) => InputEvent::Resized(PhysicalSize::new(width, height)),
        Event::CloseRequested => InputEvent::CloseRequested,
        // Raw key metadata (a Win32 scan code, an X11 keysym) is diagnostic only: the physical
        // key above is dispatched once, and the character arrives on its own callback.
        _ => return Ok(()),
    };
    game.handle_window_event(ctl, id, input, input_trace::InputOrigin::User);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_key_characters_do_not_leak_into_chat_and_reset_releases_the_gate() {
        let mut router = Router::default();
        router.key_finished(false, true);
        assert!(router.suppress_text);
        router.key_finished(true, true);
        assert!(!router.suppress_text);
        router.key_finished(false, true);
        router.clear();
        assert!(!router.suppress_text);
        router.key_finished(false, false);
        assert!(!router.suppress_text);
    }

    #[test]
    fn hid_keys_cover_gameplay_rebinding_and_distinct_release_namespaces() {
        for (usage, code) in [
            (0x1a, KeyCode::KeyW),
            (0x08, KeyCode::KeyE),
            (0x2c, KeyCode::Space),
            (0xe1, KeyCode::ShiftLeft),
            (0xe5, KeyCode::ShiftRight),
            (0x28, KeyCode::Enter),
            (0x58, KeyCode::NumpadEnter),
            (0x52, KeyCode::ArrowUp),
            (0x60, KeyCode::Numpad8),
        ] {
            assert_eq!(key_code(usage), Some(code));
        }
        assert_eq!(key_code(0), None);
        assert_eq!(key_code(0xffff), None);
    }

    #[test]
    fn one_wheel_detent_is_one_line() {
        assert_eq!(f32::from(120i16) / 120., 1.);
        assert_eq!(f32::from(-120i16) / 120., -1.);
    }
}
