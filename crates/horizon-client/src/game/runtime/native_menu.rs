//! The native desktop-GL window's frame pump. It drives the same `Game::idle` and redraw path the
//! winit loop does, on a cadence of its own, and feeds the window's events to the same handlers:
//! the game's simulation, screens and clock are not rewritten for it.
// game-module-cfg: feature = "legacy-gl"
use super::*;
use horizon_client::{
    render_contract::{FrameOptions, FrameOutcome, VisibleSections},
    renderer_facade::NativeWindowEvent as Event,
};
use std::cell::Cell;
use winit::keyboard::ModifiersState;

/// What the pump asks the loop to do, as `ActiveEventLoop` answers winit's game: a native window
/// has no loop, so the pump keeps the answers itself.
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
    /// When the next turn begins: vanilla's 20 Hz with a frame in between, slower while the window
    /// is unfocused or minimized (a hidden window has nothing to draw). A pending save asks for a
    /// shorter poll, but even an expired deadline yields: `Poll` is not permission for a
    /// zero-delay loop.
    fn next_step(&self, began: Instant, focused: bool, size: [u32; 2]) -> Instant {
        let frame = began
            + Duration::from_millis(if focused && !size.contains(&0) {
                16
            } else {
                32
            });
        match self.flow.get() {
            ControlFlow::WaitUntil(at) => at.min(frame).max(began + Duration::from_millis(4)),
            ControlFlow::Poll | ControlFlow::Wait => frame,
        }
    }
}

/// What the window's input left held in the game. A native window has no winit to tell the game
/// that keys, buttons and the pointer grab were released, so a focus loss, a resize to nothing or a
/// lost event queue clears them here.
#[derive(Default)]
struct Input {
    focused: bool,
    router: native_input::Router,
}

impl Input {
    fn reset(&mut self, game: &mut Game<'_>) {
        self.router.clear();
        game.capture(false);
        game.keys.clear();
        game.physical_keys.clear();
        game.physical_buttons.clear();
        game.drag_button = None;
        game.left = false;
        game.right = false;
        game.mouse = Default::default();
        game.modifiers = ModifiersState::empty();
        for (slot, enabled) in [
            game.settings.toggle_crouch,
            game.settings.toggle_sprint,
            game.settings.toggle_attack,
            game.settings.toggle_use,
        ]
        .into_iter()
        .enumerate()
        {
            game.toggles.0[slot].set_down(false, enabled);
        }
        game.sync_toggle(0);
        game.sync_toggle(1);
        if let Some(title) = game.title.as_mut() {
            title.release();
            title.release_field();
        }
    }
}

/// The window's frames, until the game exits. The last submission says whether the final frame
/// reached the window; the game's own error, if any, is the result.
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
        drop(facade);
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
                input.router.dispatch(game, &ctl, event)?;
                if !focused {
                    input.reset(game);
                }
                continue;
            }
            if event == Event::PointerCaptureLost {
                input.reset(game);
                game.regrab_on_refocus = false;
                continue;
            }
            if let Event::Resized(size) = event {
                if size.contains(&0) {
                    input.reset(game);
                }
                input.router.dispatch(game, &ctl, event)?;
                continue;
            }
            // A save in progress, a stop request, an unfocused or minimized window and a lost
            // event queue all drop the window's input: whatever it held is already released.
            if events.lost_events > 0
                || game.pending_save.is_some()
                || horizon_threads::stop_requests() > 0
                || !input.focused
                || size.contains(&0)
            {
                continue;
            }
            input.router.dispatch(game, &ctl, event)?;
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
        // Pending OS messages are drained on the next bounded turn too: a flooded native queue
        // must not uncap the frame cadence.
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
    fn native_cadence_bounds_focus_and_minimize_speed() {
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
    fn save_poll_deadlines_shorten_the_turn_but_never_spin() {
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
}
