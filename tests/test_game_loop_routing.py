"""Run Game's idle/title policy with isolated platform and durable-write dependencies.

The method bodies are the client's, not a rewritten loop. Fixtures verify routing,
save completion/error ownership and refusal before side effects; they do not load
normal resources, render a world or establish native Game/Win7 acceptance.
"""
from pathlib import Path
import os
import re
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


def method(source, name):
    start = re.search(rf"^(?:    )?(?:pub\(super\) )?fn {name}\(", source, re.M).start()
    opening = source.index("{", start)
    depth, end = 1, opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


FIXTURE = r'''
#![allow(dead_code)]
extern crate self as horizon_client;
extern crate self as horizon_threads;
use std::{cell::Cell, fs, path::PathBuf, time::{Duration, Instant}};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
thread_local! { static STOP: Cell<usize> = const { Cell::new(0) }; }
fn stop_requests() -> usize { STOP.get() }
mod spans { pub fn time<T>(_: &str, f: impl FnOnce() -> T) -> T { f() } }
mod lang { pub fn get(_: &str) -> String { "Unavailable".into() } }
mod clipboard {
    pub trait Clipboard { fn set(&self, text: &str); }
    pub struct Probe(pub std::path::PathBuf);
    impl Clipboard for Probe { fn set(&self, text: &str) { std::fs::write(&self.0, text).unwrap(); } }
}
mod options_ui { pub enum Page { Accessibility, Language, Sound, Credits } }
mod title {
    pub enum Mode { Main, Multiplayer(()) }
    #[derive(Default)] pub struct WorldOptions;
}
mod game {
use super::*;
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ControlFlow { Poll, Wait, WaitUntil(Instant) }
trait EventLoopCtl { fn exit(&self); fn set_control_flow(&self, flow: ControlFlow); }
struct Ctl { exit: Cell<bool>, flow: Cell<ControlFlow> }
impl Default for Ctl { fn default() -> Self { Self { exit: Cell::new(false), flow: Cell::new(ControlFlow::Wait) } } }
impl EventLoopCtl for Ctl {
    fn exit(&self) { self.exit.set(true); }
    fn set_control_flow(&self, flow: ControlFlow) { self.flow.set(flow); }
}
mod event_loop {
    use super::*;
    __REDRAW_FLOW__
}
mod input_trace { pub enum InputOrigin { Game } }
enum WindowEvent { RedrawRequested }
struct Window { redraws: Cell<usize> }
impl Window { fn id(&self) -> u32 { 7 } fn request_redraw(&self) { self.redraws.set(self.redraws.get() + 1); } }
enum Renderer { Modern, Native }
impl Renderer {
    fn is_native(&self) -> bool { matches!(self, Self::Native) }
    fn menu<'a>(&self, hud: &'a HudBatch) -> Menu<'a> { Menu { hud } }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FrameOutcome { Presented, Deferred, Offscreen }
struct HudBatch { size: [u32; 2] }
struct Menu<'a> { hud: &'a HudBatch }
struct Owner { size: Option<[u32; 2]>, outcome: FrameOutcome }
struct Facade<'a>(&'a mut Owner);
impl Owner {
    fn facade(&mut self, _: (), _: ()) -> Option<Facade<'_>> {
        self.size.map(|_| ()).map(|_| Facade(self))
    }
}
impl Facade<'_> {
    fn size(&self) -> [u32; 2] { self.0.size.unwrap() }
    fn render_menu(&mut self, menu: &Menu<'_>) -> Result<FrameOutcome> {
        if menu.hud.size.contains(&0) { return Err("invalid zero-sized HUD".into()); }
        Ok(self.0.outcome)
    }
}
struct Options { renderer: Renderer, split_guest: bool, headless: bool, offscreen: bool,
    frames: Option<u64>, timeout: Duration, resources: PathBuf }
impl Options { fn player_name(&self) -> String { "Test".into() } }
enum Screen { Playing, Options }
struct Title { mode: title::Mode, request: Option<String>, failure: Option<String> }
impl Title { fn take_open_request(&mut self) -> Option<String> { self.request.take() }
    fn failed(&mut self, text: String) { self.failure = Some(text); } }
enum Action {
    None, Quit, Options, Accessibility, Language, Sounds, Credits, OpenUri(String), CopyText(String),
    Play { folder: String }, Create { folder: String, name: String, seed: Option<i64>, creative: bool, options: title::WorldOptions },
    Connect { address: String, lan: bool }, DataPacks { dir: PathBuf, selected: Vec<String> },
}
enum PendingWorldEntry {
    Local { folder: String, name: Option<String>, seed: Option<i64>, creative: bool, create: Option<Box<title::WorldOptions>> },
    Server { address: String, name: String, cookies: Option<()>, lan: bool },
}
mod packs { pub fn open_uri(_: &str) { panic!("an external action escaped the native policy"); } }
mod world_loading { pub fn preload_world_data(_: bool, _: &PathBuf, _: ()) {} use super::*; }
struct Assets { registry: () }
struct Pack { assets: Assets }
struct OptionsUi;
impl OptionsUi { fn open_data_packs(&mut self, _: PathBuf, _: Vec<String>, _: PathBuf) { panic!("native packs opened"); } }
struct Ui { options: OptionsUi }
struct Closing { quit: bool, polls: u8, write: bool }
struct Game {
    options: Options, renderer: Owner, title: Option<Title>, window: Option<Window>, pending_save: Option<Closing>,
    pending_entry: Option<PendingWorldEntry>, pending_pack_reload: bool, rest_loader: Option<()>, pack: Pack,
    error: Option<Box<dyn std::error::Error>>, screen: Screen, options_from_title: bool,
    ui: Ui, clipboard: clipboard::Probe, keys: Vec<()>, frames: u64, frame_drawn: bool, started: Instant,
    root: PathBuf, fail_save: bool, fail_begin: bool, fail_hud: bool, fail_frame: bool,
    exit_frame: bool, control_save: bool, save_huds: u32,
}
impl Game {
    fn fixture(name: &str, native: bool, headless: bool) -> Self {
        STOP.set(0);
        let root = std::env::current_dir().unwrap().join(name);
        fs::create_dir(&root).unwrap();
        for file in ["world", "options", "clipboard"] { fs::write(root.join(file), b"kept").unwrap(); }
        Self {
            options: Options { renderer: if native { Renderer::Native } else { Renderer::Modern }, split_guest: false,
                headless, offscreen: headless, frames: None, timeout: Duration::from_secs(10), resources: root.clone() },
            renderer: Owner { size: Some([640, 480]), outcome: FrameOutcome::Presented },
            title: Some(Title { mode: title::Mode::Main, request: None, failure: None }), window: None, pending_save: None,
            pending_entry: None, pending_pack_reload: false, rest_loader: None, pack: Pack { assets: Assets { registry: () } },
            error: None, screen: Screen::Playing, options_from_title: false, ui: Ui { options: OptionsUi },
            clipboard: clipboard::Probe(root.join("clipboard")), keys: vec![], frames: 0, frame_drawn: false, started: Instant::now(),
            root, fail_save: false, fail_begin: false, fail_hud: false, fail_frame: false,
            exit_frame: false, control_save: false, save_huds: 0,
        }
    }
    fn bytes(&self, name: &str) -> Vec<u8> { fs::read(self.root.join(name)).unwrap() }
    fn exit_on_error(&self, ctl: &dyn EventLoopCtl) { ctl.exit(); }
    fn begin_closing_save(&mut self, quit: bool) -> Result<()> {
        if self.fail_begin { return Err("save setup failed".into()); }
        if let Some(save) = &mut self.pending_save { save.quit |= quit; }
        else { self.pending_save = Some(Closing { quit, polls: 2, write: self.title.is_none() }); }
        Ok(())
    }
    fn poll_closing_save(&mut self) -> Result<bool> {
        if self.fail_save { return Err("durable write failed".into()); }
        let save = self.pending_save.as_mut().unwrap();
        if save.polls > 0 { save.polls -= 1; return Ok(false); }
        let save = self.pending_save.take().unwrap();
        if save.write { fs::write(self.root.join("world"), b"durable")?; }
        Ok(save.quit)
    }
    fn render_closing_save(&mut self) -> Result<()> {
        if self.fail_hud { return Err("save HUD failed".into()); }
        self.save_huds += 1; Ok(())
    }
    // A frame-boundary probe, not a replacement simulation or frame-limit implementation.
    fn frame_tick(&mut self, ctl: &dyn EventLoopCtl) -> Result<()> {
        if self.fail_frame { return Err("frame failed".into()); }
        self.frames += 1; self.frame_drawn = true;
        if self.exit_frame { ctl.exit(); }
        Ok(())
    }
    fn handle_window_event(&mut self, ctl: &dyn EventLoopCtl, id: u32, _: WindowEvent, _: input_trace::InputOrigin) {
        assert!(!self.options.renderer.is_native() && !self.options.headless && id == 7);
        self.frame_tick(ctl).unwrap();
    }
    fn open_options(&mut self, _: bool) { fs::write(self.root.join("options"), b"changed").unwrap(); }
    fn open_options_at(&mut self, _: bool, _: options_ui::Page) { self.open_options(true); }
    fn vanilla_root(&self) -> PathBuf { self.options.resources.clone() }
    fn open_world(&mut self, _: &str, _: Option<&str>, _: Option<i64>, _: bool, _: Option<&title::WorldOptions>) -> Result<()> {
        assert!(!self.options.renderer.is_native(), "a deferred world request bypassed the policy");
        fs::write(self.root.join("world"), b"opened")?; Ok(())
    }
    fn connect_server_with_options(&mut self, _: &str, _: &str, _: Option<()>, _: bool) -> Result<()> {
        panic!("a server request escaped the native policy");
    }
    fn reload_packs(&mut self) { panic!("a reload escaped the native policy"); }
    fn poll_integrated_start(&mut self) -> Result<()> { Ok(()) }
    fn poll_world_rest(&mut self) -> Result<()> { Ok(()) }
    fn poll_world_creation(&mut self) {}
    fn tick_server_packs(&mut self) {}
    fn poll_control(&mut self, _: &dyn EventLoopCtl) {
        if std::mem::take(&mut self.control_save) { self.begin_closing_save(false).unwrap(); }
    }
    fn poll_pads(&mut self, _: &dyn EventLoopCtl) { assert!(!self.options.renderer.is_native()); }
    fn pause_if_inactive(&mut self) {}
    __METHODS__
}
#[test]
fn native_and_headless_dispatch_directly_and_preserve_the_frames_exit_request() {
    for (name, native, headless) in [("native", true, false), ("headless", false, true)] {
        let mut game = Game::fixture(name, native, headless); let ctl = Ctl::default();
        game.idle(&ctl); assert_eq!(game.frames, 1); assert!(!ctl.exit.get());
        game.exit_frame = true;
        game.idle(&ctl); assert_eq!(game.frames, 2); assert!(ctl.exit.get());
        assert!(game.error.is_none()); assert_eq!(game.bytes("world"), b"kept");
    }
}
#[test]
fn modern_visible_redraw_and_real_id_offscreen_dispatch_remain_unchanged() {
    let mut game = Game::fixture("modern", false, false); let ctl = Ctl::default();
    game.window = Some(Window { redraws: Cell::new(0) });
    game.idle(&ctl); assert_eq!(game.frames, 0); assert_eq!(game.window.as_ref().unwrap().redraws.get(), 1);
    game.options.offscreen = true;
    game.idle(&ctl); assert_eq!(game.frames, 1);
}
#[test]
fn close_and_stop_wait_for_durable_completion_without_ticking() {
    for (name, signal) in [("close", false), ("signal", true)] {
        let mut game = Game::fixture(name, true, false); let ctl = Ctl::default(); game.title = None;
        if signal { STOP.set(1); } else { game.begin_closing_save(true).unwrap(); }
        for _ in 0..2 {
            game.idle(&ctl); assert!(!ctl.exit.get()); assert_eq!(game.bytes("world"), b"kept");
            assert!(matches!(ctl.flow.get(), ControlFlow::WaitUntil(_))); assert_eq!(game.frames, 0);
        }
        game.idle(&ctl); assert!(ctl.exit.get()); assert_eq!(game.bytes("world"), b"durable");
        assert_eq!(game.frames, 0); assert_eq!(game.save_huds, 2); STOP.set(0);
    }
}
#[test]
fn save_and_frame_errors_are_retained_and_do_not_become_success() {
    for (name, save, hud, begin) in [("save-error", true, false, false), ("frame-error", false, false, false),
        ("save-hud-error", false, true, false), ("save-setup-error", false, false, true)] {
        let mut game = Game::fixture(name, true, false); let ctl = Ctl::default();
        if save || hud { game.begin_closing_save(true).unwrap(); game.fail_save = save; game.fail_hud = hud; }
        else if begin { game.fail_begin = true; STOP.set(1); }
        else { game.fail_frame = true; }
        game.idle(&ctl); assert!(ctl.exit.get()); assert!(game.error.is_some());
        let error = game.error.as_ref().unwrap().to_string(); game.idle(&ctl);
        assert_eq!(game.error.as_ref().unwrap().to_string(), error);
        assert_eq!(game.bytes("world"), b"kept"); assert_eq!(game.frames, 0);
        if save || hud { assert!(game.pending_save.is_some()); }
        STOP.set(0);
    }
}
#[test]
fn control_save_freezes_direct_dispatch_and_resumes_only_after_completion() {
    let mut game = Game::fixture("control-save", true, false); let ctl = Ctl::default();
    game.title = None; game.control_save = true;
    game.idle(&ctl); assert!(game.pending_save.is_some()); assert_eq!(game.frames, 0);
    for _ in 0..2 {
        game.idle(&ctl); assert!(!ctl.exit.get()); assert_eq!(game.frames, 0);
        assert_eq!(game.bytes("world"), b"kept");
    }
    game.idle(&ctl); assert!(game.pending_save.is_none()); assert!(!ctl.exit.get());
    assert_eq!(game.bytes("world"), b"durable"); assert_eq!(game.frames, 0);
    assert_eq!(game.save_huds, 2);
    game.idle(&ctl); assert_eq!(game.frames, 1); assert!(!ctl.exit.get());
}
#[test]
fn stop_promotes_a_pending_save_without_restarting_its_progress() {
    let mut game = Game::fixture("promoted-save", true, false); let ctl = Ctl::default();
    game.title = None; game.begin_closing_save(false).unwrap();
    game.idle(&ctl); assert_eq!(game.pending_save.as_ref().unwrap().polls, 1);
    STOP.set(1); game.idle(&ctl); assert!(game.pending_save.as_ref().unwrap().quit);
    assert!(!ctl.exit.get()); assert_eq!(game.bytes("world"), b"kept");
    game.idle(&ctl); assert!(ctl.exit.get()); assert_eq!(game.bytes("world"), b"durable"); STOP.set(0);
}
#[test]
fn timed_out_direct_runs_do_not_dispatch_another_frame() {
    for (name, native, headless) in [("native-timeout", true, false), ("headless-timeout", false, true)] {
        let mut game = Game::fixture(name, native, headless); let ctl = Ctl::default();
        game.options.frames = Some(1); game.options.timeout = Duration::from_secs(1);
        game.started = Instant::now() - Duration::from_secs(2);
        game.idle(&ctl); assert!(ctl.exit.get()); assert_eq!(game.frames, 0);
        assert_eq!(game.error.as_ref().unwrap().to_string(), "frame-limited run timed out");
    }
}
#[test]
fn minimized_native_progress_defers_until_a_valid_drawable_returns() {
    let mut game = Game::fixture("minimized-menu", true, false);
    game.renderer.size = Some([0, 480]);
    assert_eq!(game.submit_menu_frame(&HudBatch { size: [0, 480] }).unwrap(), FrameOutcome::Deferred);
    game.renderer.size = Some([640, 480]);
    assert_eq!(game.submit_menu_frame(&HudBatch { size: [640, 480] }).unwrap(), FrameOutcome::Presented);
    game.renderer.size = None;
    assert_eq!(game.submit_menu_frame(&HudBatch { size: [640, 480] }).unwrap(), FrameOutcome::Deferred);
}
#[test]
fn menu_completion_is_not_promoted_or_accepted_on_the_wrong_surface() {
    for (name, native, offscreen) in [("native-menu-outcome", true, false), ("modern-menu-outcome", false, false),
        ("offscreen-menu-outcome", true, true)] {
        let mut game = Game::fixture(name, native, false); game.options.offscreen = offscreen;
        for outcome in [FrameOutcome::Presented, FrameOutcome::Deferred, FrameOutcome::Offscreen] {
            game.renderer.outcome = outcome;
            let result = game.submit_menu_frame(&HudBatch { size: [640, 480] });
            if native && !offscreen && outcome == FrameOutcome::Offscreen { assert!(result.is_err()); }
            else { assert_eq!(result.unwrap(), outcome); }
        }
    }
}
#[test]
fn native_title_and_deferred_callbacks_cannot_mutate_saves_options_or_external_state() {
    let mut game = Game::fixture("policy", true, false); let ctl = Ctl::default();
    for action in [Action::Options, Action::Accessibility, Action::Language, Action::Sounds, Action::Credits,
        Action::Play { folder: "world".into() }, Action::Create { folder: "new".into(), name: "New".into(), seed: None,
            creative: true, options: title::WorldOptions }, Action::Connect { address: "server".into(), lan: false },
        Action::DataPacks { dir: PathBuf::new(), selected: vec![] }, Action::OpenUri("https://example.invalid".into()),
        Action::CopyText("changed".into())] {
        game.title.as_mut().unwrap().failure = None; game.title_action(&ctl, action).unwrap();
        assert!(game.title.as_ref().unwrap().failure.is_some());
        for file in ["world", "options", "clipboard"] { assert_eq!(game.bytes(file), b"kept"); }
    }
    game.title.as_mut().unwrap().request = Some("world".into()); game.idle(&ctl);
    assert!(game.title.as_ref().unwrap().request.is_none()); assert_eq!(game.bytes("world"), b"kept");
    game.pending_entry = Some(PendingWorldEntry::Local { folder: "world".into(), name: None, seed: None, creative: false, create: None });
    game.pending_pack_reload = true; game.idle(&ctl);
    assert!(game.pending_entry.is_none()); assert!(!game.pending_pack_reload); assert!(!ctl.exit.get());
    assert_eq!(game.bytes("world"), b"kept"); assert_eq!(game.bytes("options"), b"kept");
    game.pending_entry = Some(PendingWorldEntry::Server { address: "server".into(), name: "Test".into(), cookies: None, lan: true });
    game.idle(&ctl); assert!(game.pending_entry.is_none()); assert!(!ctl.exit.get());
}
#[test]
fn modern_title_actions_and_native_save_on_quit_have_distinct_policies() {
    let mut game = Game::fixture("modern-actions", false, false); let ctl = Ctl::default();
    game.title_action(&ctl, Action::Options).unwrap(); assert_eq!(game.bytes("options"), b"changed");
    game.title_action(&ctl, Action::Play { folder: "world".into() }).unwrap(); assert_eq!(game.bytes("world"), b"opened");
    game.title_action(&ctl, Action::Quit).unwrap(); assert!(ctl.exit.get()); assert!(game.pending_save.is_none());
    let mut game = Game::fixture("native-quit", true, false); let ctl = Ctl::default();
    game.title_action(&ctl, Action::Quit).unwrap(); assert!(!ctl.exit.get());
    assert!(game.pending_save.as_ref().unwrap().quit);
    for _ in 0..3 { game.idle(&ctl); }
    assert!(ctl.exit.get()); assert_eq!(game.bytes("world"), b"kept"); assert_eq!(game.bytes("options"), b"kept");
}
}
'''


class GameLoopRoutingTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_idle_and_title_side_effect_boundaries(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text(encoding="utf-8")
        methods = "\n".join(method(source, name) for name in
                            ("idle", "title_action", "native_menu_only", "resume_world_entry", "submit_menu_frame"))
        scheduling = (ROOT / "crates/horizon-client/src/game/event_loop.rs").read_text(encoding="utf-8")
        fixture = (FIXTURE.replace("__METHODS__", methods)
                   .replace("__REDRAW_FLOW__", method(scheduling, "redraw_flow")))
        with tempfile.TemporaryDirectory(prefix="horizon-loop-routing-") as directory:
            directory = Path(directory)
            path = directory / "fixture.rs"
            binary = directory / ("fixture.exe" if os.name == "nt" else "fixture")
            path.write_text(fixture, encoding="utf-8")
            built = subprocess.run(["rustc", "--edition=2024", "--test", str(path), "-o", str(binary)],
                                   capture_output=True, text=True, encoding="utf-8")
            self.assertEqual(built.returncode, 0, built.stdout + built.stderr)
            ran = subprocess.run([str(binary), "--test-threads=1"], cwd=directory,
                                 capture_output=True, text=True, encoding="utf-8")
            self.assertEqual(ran.returncode, 0, ran.stdout + ran.stderr)
