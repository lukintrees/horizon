//! Native menu adapter into existing Game/Title producers, not a simulation.
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
#[derive(Default)]
struct Input {
    left: bool,
    focused: bool,
    pointer: Option<[i32; 2]>,
}
impl Input {
    fn reset(&mut self, game: &mut Game<'_>) {
        self.left = false;
        game.physical_keys.clear();
        game.modifiers = ModifiersState::empty();
        if let Some(title) = game.title.as_mut() {
            title.release();
            title.release_field();
        }
    }
    fn pointer(&mut self, game: &mut Game<'_>, size: [u32; 2], x: i32, y: i32) {
        self.pointer = Some([x, y]);
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
fn action(game: &mut Game<'_>, ctl: &LoopCtl, action: Action) -> Result<()> {
    if matches!(action, Action::None | Action::Quit) {
        return game.title_action(ctl, action);
    }
    // Do not open an existing save, create a world, connect a server or mutate
    // modern-only settings and then discover native world rendering is absent.
    if let Some(title) = game.title.as_mut() {
        title.failed("Native GL2 menu-only: world entry, options, packs and external actions are not connected yet. No world was opened.");
    }
    Ok(())
}
fn key(
    game: &mut Game<'_>,
    ctl: &LoopCtl,
    size: [u32; 2],
    code: KeyCode,
    pressed: bool,
) -> Result<()> {
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
            action(game, ctl, next)?;
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
    let started = Instant::now();
    let mut last = FrameOutcome::Deferred;
    while !ctl.exit.get() && horizon_threads::stop_requests() == 0 {
        let began = Instant::now();
        let mut facade = game
            .renderer
            .facade(VisibleSections::default(), FrameOptions::default())
            .ok_or("missing native menu owner")?;
        let events = facade.poll_native_window_events()?;
        let size = facade.size();
        if events.close_requested {
            break;
        }
        if events.lost_events > 0 {
            input.reset(game);
        } else {
            for event in events.events {
                if let Event::Focused(focused) = event {
                    input.focused = focused;
                    game.focused = focused;
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
                    if !size.contains(&0)
                        && let Some([x, y]) = input.pointer
                    {
                        input.pointer(game, size, x, y);
                    }
                    continue;
                }
                if !input.focused || size.contains(&0) {
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
                                    t.press_rules(
                                        &game.gui.atlas,
                                        game.cursor,
                                        (b.width, b.height),
                                    );
                                    action(game, &ctl, next)?;
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
        }
        if ctl.exit.get() {
            break;
        }
        if !size.contains(&0) {
            horizon_client::widgets::end_frame(game.frames);
            last = super::native_startup::render_menu(game)?;
            if last == FrameOutcome::Offscreen {
                return Err("native window loop received offscreen completion".into());
            }
            if last.was_presented() {
                game.frames += 1;
            }
        }
        if game.options.frames.is_some_and(|n| game.frames >= n) {
            break;
        }
        if game.options.frames.is_some() && started.elapsed() > game.options.timeout {
            return Err("native menu frame limit timed out".into());
        }
        // Bounded pumping and a capped menu cadence, including focus/minimize.
        // This is not Game::idle or a replacement for the simulation scheduler.
        let cadence = Duration::from_millis(if !input.focused || size.contains(&0) {
            32
        } else {
            16
        });
        let period = match ctl.flow.get() {
            ControlFlow::Poll => Duration::ZERO,
            ControlFlow::Wait => cadence,
            ControlFlow::WaitUntil(at) => cadence.min(at.saturating_duration_since(began)),
        };
        if !events.pending_messages
            && let Some(left) = period.checked_sub(began.elapsed())
        {
            std::thread::sleep(left);
        }
    }
    input.reset(game);
    Ok(last)
}
#[cfg(test)]
mod tests {
    use super::*;
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
