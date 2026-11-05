//! Visible native startup: one CPU-only worker, one bounded completion, and
//! the same thread-confined window/context from boot screen through Game.
use super::*;
use horizon_client::render_contract::FrameOutcome;
use horizon_client_render::native_loading::{Context, LoadingScreen};
use std::sync::mpsc::{self, TryRecvError};

/// Cancellation never abandons a worker with global/cache writes in flight.
/// Drop is the error/panic safety net; normal close keeps pumping until joined.
struct Worker<T> {
    receiver: Receiver<std::result::Result<T, String>>,
    handle: Option<JoinHandle<()>>,
    progress: Progress,
}
impl<T: Send + 'static> Worker<T> {
    fn spawn(
        progress: Progress,
        work: impl FnOnce(&Progress) -> std::result::Result<T, String> + Send + 'static,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let state = progress.clone();
        let handle = std::thread::Builder::new()
            .name("native-resource-loader".into())
            .spawn(move || {
                let result = state
                    .check_cancelled()
                    .map_err(|e| e.to_string())
                    .and_then(|_| work(&state));
                if state.check_cancelled().is_ok() {
                    let _ = sender.send(result);
                }
            })?;
        Ok(Self {
            receiver,
            handle: Some(handle),
            progress,
        })
    }
    fn join(&mut self) -> Result<()> {
        if let Some(handle) = self.handle.take() {
            handle.join().map_err(|payload| {
                let message = payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("unknown panic");
                format!("native resource loader panicked: {message}")
            })?;
        }
        Ok(())
    }
    fn poll(&mut self) -> Result<Option<T>> {
        match self.receiver.try_recv() {
            Ok(result) => {
                self.join()?;
                Ok(Some(result?))
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                self.join()?;
                Err("native resource loader stopped without a completion".into())
            }
        }
    }
    fn finished(&self) -> bool {
        self.handle.as_ref().is_none_or(|h| h.is_finished())
    }
}
impl<T> Drop for Worker<T> {
    fn drop(&mut self) {
        self.progress.cancel();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn draw(screen: &mut LoadingScreen<'_>, boot: &HudAtlas, size: [u32; 2]) -> Result<bool> {
    if size.contains(&0) {
        return Ok(false);
    }
    boot.font.begin_frame();
    let mut batch = HudBatch::new(size);
    // Reuse the existing asset-free wordmark/console loading producer. No
    // synthetic title or replacement loading UI and no terrain placeholder.
    horizon_client::ui_theme::draw_boot(boot, &mut batch, &horizon_client::logging::recent_lines());
    Ok(screen.draw(&batch)? == FrameOutcome::Presented)
}

pub(super) fn run(
    mut options: Options,
    settings: Settings,
    root: PathBuf,
    started: Instant,
) -> Result<()> {
    options.renderer.validate(&options)?;
    if options.offscreen || options.native_bootstrap_only {
        return Err("visible native loader requires the non-bootstrap window route".into());
    }
    let context = Context::new(
        render_startup::Renderer::context_kind(false)?,
        options.window_size,
    )?;
    let boot = HudAtlas::boot();
    let progress = Progress::new();
    let mut shown = 0u64;
    let mut loaded;
    let mut focused = true;
    {
        let mut screen = LoadingScreen::new(&context, &boot)?;
        // Window and actual boot frame precede the heavy worker dispatch.
        shown += u64::from(draw(&mut screen, &boot, context.size())?);
        startup::mark("native_loading_screen.shown");
        let worker_settings = settings.clone();
        let worker_root = root.clone();
        let mut worker = Worker::spawn(progress.clone(), move |progress| {
            let mut loaded = load_resources(
                worker_root.clone(),
                None,
                &worker_root,
                &worker_settings,
                started,
                false,
                false,
                render_startup::Renderer::NativeGl2,
                progress,
                &mut |_, _| {},
            )
            .map_err(|e| e.to_string())?;
            progress.check_cancelled().map_err(|e| e.to_string())?;
            loaded.game_cpu =
                Some(GameCpuResources::load(&loaded.root).map_err(|e| e.to_string())?);
            progress.check_cancelled().map_err(|e| e.to_string())?;
            Ok(loaded)
        })?;
        let mut closing = false;
        let mut timeout = false;
        loop {
            let events = context.poll_events()?;
            for event in &events.events {
                if let horizon_client::renderer_facade::NativeWindowEvent::Focused(value) = event {
                    focused = *value;
                }
            }
            timeout |= options.frames.is_some() && started.elapsed() > options.timeout;
            closing |= events.close_requested || horizon_threads::stop_requests() > 0 || timeout;
            if closing {
                progress.cancel();
                // Cooperative phase-boundary cancellation, never a detached
                // worker or a blocking join while it is still doing CPU work.
                if worker.finished() {
                    worker.join()?;
                    if let Some(path) = &options.report {
                        let report = serde_json::json!({"scenario": "native-game-loading-cancelled",
                            "game_constructed": false, "loading_presentations": shown,
                            "worker_joined": true, "timed_out": timeout,
                            "playable": false, "world_opened": false});
                        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
                    }
                    return if timeout {
                        Err("native resource loading timed out".into())
                    } else {
                        Ok(())
                    };
                }
            } else if let Some(bundle) = worker.poll()? {
                loaded = bundle;
                break;
            }
            // Never draw after DestroyNotify/zero-size suspension; keep pumping
            // minimize/restore/close while a CPU phase reaches cancellation.
            shown += u64::from(draw(&mut screen, &boot, context.size())?);
            if !events.pending_messages {
                std::thread::sleep(Duration::from_millis(16));
            }
        }
    } // worker joined and boot GPU resources released BEFORE context transfer
    drop(boot);
    if let Some(tail) = loaded.rest_loader.take() {
        progress.cancel();
        let _ = tail.handle.join();
        return Err("native loading unexpectedly spawned a world tail".into());
    }
    if loaded.renderer_cpu.is_some() {
        return Err(
            "native loading unexpectedly prepared modern textures or a detached tail".into(),
        );
    }
    let gui = loaded
        .gui
        .ok_or("native resource loader returned no menu bundle")?;
    options.resources = loaded.root.clone();
    let resources = loaded.resources;
    let audio = Audio::silent(&options.resources);
    let mut game = start_game(
        &resources,
        loaded.pack,
        gui,
        options,
        loaded.pack_error,
        audio,
        root,
        started,
        None,
        Some(settings),
        loaded.game_cpu,
    )?
    .ok_or("native loading did not construct Game")?;
    game.focused = focused;
    let handoff = Instant::now();
    native_startup::initialize_with_context(&mut game, context)?;
    log::info!(
        "native loading: {shown} boot presentations, same-context GPU handoff {} ms",
        handoff.elapsed().as_millis()
    );
    native_startup::run_with_loading(game, shown)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn completion<T: Send + 'static>(worker: &mut Worker<T>) -> Result<T> {
        let start = Instant::now();
        loop {
            if let Some(value) = worker.poll()? {
                return Ok(value);
            }
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::yield_now();
        }
    }
    #[test]
    fn worker_delivers_one_completion_and_joins_without_reuse() {
        let mut w = Worker::spawn(Progress::new(), |_| Ok(42)).unwrap();
        assert_eq!(completion(&mut w).unwrap(), 42);
        assert!(w.handle.is_none());
        assert!(w.poll().is_err());
    }
    #[test]
    fn loader_errors_and_panics_are_not_an_infinite_loading_screen() {
        let mut w = Worker::<()>::spawn(Progress::new(), |_| Err("broken atlas".into())).unwrap();
        assert!(
            completion(&mut w)
                .unwrap_err()
                .to_string()
                .contains("broken atlas")
        );
        let mut w = Worker::<()>::spawn(Progress::new(), |_| panic!("broken worker")).unwrap();
        assert!(
            completion(&mut w)
                .unwrap_err()
                .to_string()
                .contains("broken worker")
        );
    }
    #[test]
    fn cancellation_drops_unpublished_bundle_and_joins_before_restart() {
        use std::sync::atomic::AtomicBool;
        struct Bundle(Arc<AtomicBool>);
        impl Drop for Bundle {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let flag = dropped.clone();
        let (ready, entered) = mpsc::sync_channel(1);
        let progress = Progress::new();
        let worker = Worker::spawn(progress.clone(), move |p| {
            let bundle = Bundle(flag);
            ready.send(()).unwrap();
            while p.check_cancelled().is_ok() {
                std::thread::yield_now();
            }
            Ok(bundle)
        })
        .unwrap();
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(worker); // cancellation + join, including dropping late completion
        assert!(progress.check_cancelled().is_err());
        assert!(dropped.load(Ordering::Acquire));
        assert!(Progress::new().check_cancelled().is_ok());
    }
    #[test]
    fn cancelled_load_never_touches_missing_resource_files() {
        let progress = Progress::new();
        progress.cancel();
        let error = load_resources(
            PathBuf::from("missing-native-cancelled-resources"),
            None,
            std::path::Path::new("missing-native-cancelled-resources"),
            &Settings::default(),
            Instant::now(),
            false,
            false,
            render_startup::Renderer::NativeGl2,
            &progress,
            &mut |_, _| {},
        )
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("cancelled"), "{error}");
    }
}
