//! Native producers enter Game's existing input pipeline with ordinary CPU
//! data, never synthetic winit window/device IDs or private KeyEvent state.
use super::input_event::{Event as InputEvent, KeyEvent};
use super::*;
use horizon_client::renderer_facade::{NativeMouseButton as Button, NativeWindowEvent as Event};
use winit::keyboard::{Key, NativeKey};

pub(super) fn key_code(usage: u16) -> Option<KeyCode> {
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
        0x04..=0x1d => LETTERS[usize::from(usage - 4)],
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
/// Characters are a separate native callback. Do not type the key that
/// just opened a text field (T/slash/chat), unlike an ordinary typing key.
#[derive(Default)]
pub(super) struct Router {
    suppress_text: bool,
}
impl Router {
    pub fn clear(&mut self) {
        self.suppress_text = false;
    }
    fn key_finished(&mut self, was_typing: bool, is_typing: bool) {
        self.suppress_text = !was_typing && is_typing;
    }
    pub fn dispatch(
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
    let input = match event {
        Event::PhysicalKey {
            usage,
            pressed,
            repeat,
        } => {
            let Some(code) = key_code(usage) else {
                return Ok(());
            };
            game.press_modifier_key(code, pressed);
            InputEvent::KeyboardInput {
                event: KeyEvent {
                    physical_key: PhysicalKey::Code(code),
                    logical_key: Key::Unidentified(NativeKey::Unidentified),
                    text: None,
                    state: state(pressed),
                    repeat,
                },
                is_synthetic: false,
            }
        }
        Event::PointerMoved { x, y } if !game.captured => InputEvent::CursorMoved {
            position: winit::dpi::PhysicalPosition::new(f64::from(x), f64::from(y)),
        },
        Event::MouseButton {
            button: b,
            pressed,
            x,
            y,
        } => {
            if !game.captured {
                game.handle_input_event(
                    ctl,
                    InputEvent::CursorMoved {
                        position: winit::dpi::PhysicalPosition::new(f64::from(x), f64::from(y)),
                    },
                    input_trace::InputOrigin::User,
                );
            }
            InputEvent::MouseInput {
                state: state(pressed),
                button: button(b),
            }
        }
        Event::Wheel { horizontal, delta } => {
            let lines = f32::from(delta) / 120.;
            InputEvent::MouseWheel {
                delta: MouseScrollDelta::LineDelta(
                    if horizontal { lines } else { 0. },
                    if horizontal { 0. } else { lines },
                ),
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
            InputEvent::Ime(winit::event::Ime::Commit(text))
        }
        Event::RelativeMotion { x, y } => {
            game.relative_motion(x, y);
            return Ok(());
        }
        Event::InputUnavailable => return Err("native relative mouse device is unsupported".into()),
        Event::Focused(focused) => InputEvent::Focused(focused),
        Event::Resized([width, height]) => {
            InputEvent::Resized(winit::dpi::PhysicalSize::new(width, height))
        }
        Event::CloseRequested => InputEvent::CloseRequested,
        // Raw native key metadata is diagnostic only; normalized PhysicalKey
        // is dispatched once, and Unicode arrives separately from the producer.
        _ => return Ok(()),
    };
    game.handle_input_event(ctl, input, input_trace::InputOrigin::User);
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
}
