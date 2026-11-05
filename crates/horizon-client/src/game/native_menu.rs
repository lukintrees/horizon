//! Native pump drives Game::idle/frame_tick, not a second simulation or clock.
//! World entry and options remain explicit error screens until their native
//! producers/lifecycle are connected. No synthetic winit DeviceId/WindowId.
use super::*;
use horizon_client::{
    render_contract::{FrameOptions, FrameOutcome, VisibleSections},
    renderer_facade::NativeWindowEvent as Event,
};
use std::cell::Cell;
use winit::keyboard::ModifiersState;

struct LoopCtl {
    exit: Cell<bool>,
    flow: Cell<ControlFlow>,
}
impl Default for LoopCtl {
    fn default() -> Self {
        Self {
            exit: Cell::new(false),
            flow: Cell::new(ControlFlow::Wait),
        }
    }
}
impl EventLoopCtl for LoopCtl {
    fn exit(&self) {
        self.exit.set(true);
    }
    fn set_control_flow(&self, flow: ControlFlow) {
        self.flow.set(flow);
    }
}
impl LoopCtl {
    fn next_step(&self, began: Instant, focused: bool, size: [u32; 2]) -> Instant {
        let frame = began
            + Duration::from_millis(if focused && !size.contains(&0) {
                16
            } else {
                32
            });
        match self.flow.get() {
            // A closing save asks for a shorter poll. Even an expired deadline
            // must yield; Poll is not permission for a zero-delay native loop.
            ControlFlow::WaitUntil(at) => at.min(frame).max(began + Duration::from_millis(4)),
            ControlFlow::Poll | ControlFlow::Wait => frame,
        }
    }
}
#[derive(Default)]
struct Input {
    left: bool,
    focused: bool,
    pointer: Option<[i32; 2]>,
}
impl Input {
    fn reset(&mut self, game: &mut Game<'_>) {
        self.left = false;
        game.keys.clear();
        game.left = false;
        game.right = false;
        game.physical_keys.clear();
        game.modifiers = ModifiersState::empty();
        if let Some(title) = game.title.as_mut() {
            title.release();
            title.release_field();
        }
    }
    fn pointer(&mut self, game: &mut Game<'_>, size: [u32; 2], x: i32, y: i32) {
        self.pointer = Some([x, y]);
        game.last_input = Instant::now();
        let batch = HudBatch::new(size);
        game.cursor = (x as f32 / batch.scale, y as f32 / batch.scale);
        if self.left
            && let Some(title) = game.title.as_mut()
        {
            title.drag(game.cursor, (batch.width, batch.height));
            title.drag_field(&game.gui.atlas, game.cursor.0);
        }
    }
}
fn key(
    game: &mut Game<'_>,
    ctl: &LoopCtl,
    size: [u32; 2],
    code: KeyCode,
    pressed: bool,
) -> Result<()> {
    game.last_input = Instant::now();
    game.press_modifier_key(code, pressed);
    if !pressed {
        return Ok(());
    }
    let b = HudBatch::new(size);
    let viewport = (b.width, b.height);
    let (ctrl, shift) = (game.has_control_down(), game.has_shift_down());
    let Some(title) = game.title.as_mut() else {
        return Err("missing native title".into());
    };
    if title.tab_bar_key(code, ctrl, shift).is_some() {
        return Ok(());
    }
    match code {
        KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space
            if code != KeyCode::Space || title.list_focused() || title.focused_button.is_some() =>
        {
            let (next, _) = title.confirm_key_size(viewport);
            game.title_action(ctl, next)?;
        }
        code @ (KeyCode::Backspace
        | KeyCode::Delete
        | KeyCode::ArrowLeft
        | KeyCode::ArrowRight
        | KeyCode::Home
        | KeyCode::End)
            if title.has_caret_field() =>
        {
            title.field_key(code, ctrl, shift);
        }
        code @ (KeyCode::ArrowLeft
        | KeyCode::ArrowRight
        | KeyCode::ArrowUp
        | KeyCode::ArrowDown) => {
            title.arrow_key(code, viewport, shift);
        }
        KeyCode::Backspace => game.title_edit(None, true),
        KeyCode::Tab => title.tab_dir(shift, viewport),
        KeyCode::Escape => title.escape(),
        _ => {}
    }
    Ok(())
}
/// UI keys only. Raw native metadata never pretends to be a portable gameplay
/// binding; native gameplay/relative look will use a separate binding adapter.
fn ui_key(event: &Event) -> Option<(KeyCode, bool)> {
    use KeyCode::*;
    let (value, pressed, x11) = match *event {
        Event::X11Key {
            keysym, pressed, ..
        } => (keysym, pressed, true),
        Event::Key {
            virtual_key,
            scan_code,
            extended,
            pressed,
            ..
        } => {
            let value = match virtual_key {
                16 if scan_code == 0x36 => 0xa1,
                17 if extended => 0xa3,
                18 if extended => 0xa5,
                _ => virtual_key,
            };
            (value, pressed, false)
        }
        _ => return None,
    };
    let code = if x11 {
        match value {
            0xff08 => Backspace,
            0xffff => Delete,
            0xff09 => Tab,
            0xff0d => Enter,
            0xff8d => NumpadEnter,
            0xff1b => Escape,
            0x20 => Space,
            0xff50 => Home,
            0xff57 => End,
            0xff51 => ArrowLeft,
            0xff52 => ArrowUp,
            0xff53 => ArrowRight,
            0xff54 => ArrowDown,
            0xffe1 => ShiftLeft,
            0xffe2 => ShiftRight,
            0xffe3 => ControlLeft,
            0xffe4 => ControlRight,
            0xffe9 => AltLeft,
            0xffea => AltRight,
            _ => return None,
        }
    } else {
        match value {
            8 => Backspace,
            9 => Tab,
            13 => Enter,
            27 => Escape,
            32 => Space,
            35 => End,
            36 => Home,
            37 => ArrowLeft,
            38 => ArrowUp,
            39 => ArrowRight,
            40 => ArrowDown,
            46 => Delete,
            16 | 0xa0 => ShiftLeft,
            0xa1 => ShiftRight,
            17 | 0xa2 => ControlLeft,
            0xa3 => ControlRight,
            18 | 0xa4 => AltLeft,
            0xa5 => AltRight,
            _ => return None,
        }
    };
    Some((code, pressed))
}

pub(super) fn run(game: &mut Game<'_>) -> Result<FrameOutcome> {
    let ctl = LoopCtl::default();
    let mut input = Input {
        focused: game.focused,
        ..Default::default()
    };
    game.previous = Instant::now();
    let mut last = FrameOutcome::Deferred;
    while !ctl.exit.get() {
        let began = Instant::now();
        ctl.flow.set(ControlFlow::Wait);
        let mut facade = game
            .renderer
            .facade(VisibleSections::default(), FrameOptions::default())
            .ok_or("missing native menu owner")?;
        let events = facade.poll_native_window_events()?;
        let size = facade.size();
        if events.close_requested {
            game.begin_closing_save(true)?;
            input.reset(game);
        }
        if events.lost_events > 0 {
            input.reset(game);
        }
        for event in events.events {
            if let Event::Focused(focused) = event {
                input.focused = focused;
                game.focused = focused;
                game.focus_lost = (!focused).then(Instant::now);
                if !focused {
                    input.reset(game);
                }
                continue;
            }
            if event == Event::PointerCaptureLost {
                input.reset(game);
                continue;
            }
            if matches!(event, Event::Resized(_)) {
                if size.contains(&0) {
                    input.reset(game);
                } else if let Some([x, y]) = input.pointer {
                    input.pointer(game, size, x, y);
                }
                continue;
            }
            if events.lost_events > 0
                || game.pending_save.is_some()
                || horizon_threads::stop_requests() > 0
                || !input.focused
                || size.contains(&0)
            {
                continue;
            }
            if let Some((code, pressed)) = ui_key(&event) {
                key(game, &ctl, size, code, pressed)?;
                continue;
            }
            match event {
                Event::PointerMoved { x, y } => input.pointer(game, size, x, y),
                Event::MouseButton {
                    button,
                    pressed,
                    x,
                    y,
                } => {
                    input.pointer(game, size, x, y);
                    let b = HudBatch::new(size);
                    if button == horizon_client::renderer_facade::NativeMouseButton::Left {
                        input.left = pressed;
                        if !pressed {
                            if let Some(t) = game.title.as_mut() {
                                t.release();
                                t.release_field();
                            }
                        } else {
                            let shift = game.has_shift_down();
                            if let Some(t) = game.title.as_mut() {
                                let (next, _) =
                                    t.click(game.cursor.0, game.cursor.1, (b.width, b.height));
                                t.press_field(
                                    &game.gui.atlas,
                                    game.cursor,
                                    (b.width, b.height),
                                    shift,
                                );
                                t.press_rules(&game.gui.atlas, game.cursor, (b.width, b.height));
                                game.title_action(&ctl, next)?;
                            }
                        }
                    }
                }
                Event::Wheel {
                    horizontal: false,
                    delta,
                } => {
                    let b = HudBatch::new(size);
                    if let Some(t) = game.title.as_mut() {
                        t.scroll_worlds(delta as f32 / 120., game.cursor, (b.width, b.height));
                    }
                }
                Event::Text {
                    character,
                    repeat_count,
                } if !game.modifiers.control_key() && !game.modifiers.alt_key() => {
                    game.last_input = Instant::now();
                    if repeat_count > 32 {
                        return Err("native menu text repeat batch exceeds budget".into());
                    }
                    let mut buffer = [0; 4];
                    let text = character.encode_utf8(&mut buffer);
                    for _ in 0..repeat_count {
                        game.title_edit(Some(text), false);
                    }
                }
                _ => {}
            }
        }
        if ctl.exit.get() {
            break;
        }
        let frames = game.frames;
        let saving = game.pending_save.is_some() || horizon_threads::stop_requests() > 0;
        game.idle(&ctl);
        if !saving && !size.contains(&0) {
            last = if game.frames > frames {
                FrameOutcome::Presented
            } else {
                FrameOutcome::Deferred
            };
        }
        if ctl.exit.get() {
            break;
        }
        // Pending OS messages are drained on the next bounded turn too: a
        // flooded native queue must not uncap Game's frame/idle cadence.
        if let Some(left) = ctl
            .next_step(began, input.focused, size)
            .checked_duration_since(Instant::now())
        {
            std::thread::sleep(left);
        }
    }
    input.reset(game);
    game.error.take().map_or(Ok(last), Err)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_poll_and_wait_still_bound_focus_and_minimize_cadence() {
        let ctl = LoopCtl::default();
        let now = Instant::now();
        for flow in [ControlFlow::Poll, ControlFlow::Wait] {
            ctl.set_control_flow(flow);
            assert_eq!(
                ctl.next_step(now, true, [640, 480]),
                now + Duration::from_millis(16)
            );
            for (focused, size) in [(false, [640, 480]), (true, [0, 480]), (true, [640, 0])] {
                assert_eq!(
                    ctl.next_step(now, focused, size),
                    now + Duration::from_millis(32)
                );
            }
        }
    }
    #[test]
    fn save_poll_deadlines_can_shorten_cadence_but_never_spin() {
        let ctl = LoopCtl::default();
        let now = Instant::now();
        for deadline in [
            now - Duration::from_secs(1),
            now,
            now + Duration::from_millis(4),
        ] {
            ctl.set_control_flow(ControlFlow::WaitUntil(deadline));
            assert_eq!(
                ctl.next_step(now, false, [0; 2]),
                now + Duration::from_millis(4)
            );
        }
        ctl.set_control_flow(ControlFlow::WaitUntil(now + Duration::from_secs(1)));
        assert_eq!(
            ctl.next_step(now, true, [640, 480]),
            now + Duration::from_millis(16)
        );
    }
    #[test]
    fn ui_mapping_preserves_the_native_key_namespaces_and_release_edges() {
        assert_eq!(
            ui_key(&Event::X11Key {
                keysym: 0xff1b,
                keycode: 9,
                pressed: true,
                repeat: false
            }),
            Some((KeyCode::Escape, true))
        );
        assert_eq!(
            ui_key(&Event::Key {
                virtual_key: 27,
                scan_code: 1,
                extended: false,
                pressed: false,
                repeat: false,
                repeat_count: 1
            }),
            Some((KeyCode::Escape, false))
        );
        assert_eq!(
            ui_key(&Event::X11Key {
                keysym: 27,
                keycode: 9,
                pressed: true,
                repeat: false
            }),
            None
        );
        assert_eq!(
            ui_key(&Event::X11Key {
                keysym: 0xffe2,
                keycode: 62,
                pressed: true,
                repeat: false
            }),
            Some((KeyCode::ShiftRight, true))
        );
    }
}
