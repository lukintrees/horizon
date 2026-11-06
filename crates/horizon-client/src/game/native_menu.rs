//! Native pump drives Game::idle/frame_tick, not a second simulation or clock.
//! Input enters the same Game handlers as modern. World entry remains guarded
//! until the real-worker/world acceptance scenario is verified. No synthetic winit DeviceId/WindowId.
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
}
