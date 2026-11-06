//! One client frame for winit, headless and native loops. Platform adapters
//! drive this existing Game tick; none owns a second simulation or clock.
use super::*;

impl Game<'_> {
    pub(super) fn frame_tick(&mut self, event_loop: &dyn EventLoopCtl) -> Result<()> {
        // A pad/control action may have started a save during idle. Direct
        // headless/native frames freeze just as winit's redraw handler does.
        if self.pending_save.is_some() {
            return self.render_closing_save();
        }
        horizon_client::spans::time("recover_device", || self.recover_device())?;
        self.observe_fullscreen();
        self.sync_ime();
        let now = Instant::now();
        let frame_cpu = self
            .options
            .spikes
            .and_then(|_| horizon_client::spans::thread_cpu());
        let elapsed = now.duration_since(self.previous);
        let between = now.saturating_duration_since(self.frame_end);
        self.frame_ticks = elapsed.as_secs_f32() * 20.0;
        self.previous = now;
        // Arrow-key camera controls belong to gameplay; chat and
        // other screens use the same keys for their own navigation.
        if self.screen == Screen::Playing
            && self.title.is_none()
            && self.sign_edit.is_none()
            && self.book.is_none()
            && !self.dialog_open()
        {
            let turn = |k| f32::from(u8::from(self.keys.contains(&k)));
            let secs = elapsed.as_secs_f32().min(0.1);
            let yaw = (turn(KeyCode::ArrowRight) - turn(KeyCode::ArrowLeft)) * 90.0;
            let pitch = (turn(KeyCode::ArrowDown) - turn(KeyCode::ArrowUp)) * 90.0;
            if yaw != 0.0 || pitch != 0.0 {
                // 90 degrees a second, as `Entity.turn` degrees.
                self.turn_player(f64::from(yaw * secs / 0.15), f64::from(pitch * secs / 0.15));
            }
        }
        if self.title.is_none() {
            horizon_client::spans::time("update_world", || self.update_world());
            if self.options.renderer.is_native()
                && let Some(error) = self.error.take()
            {
                return Err(error);
            }
            horizon_client::spans::time("spawn_player", || self.spawn_player())?;
        }
        let updated = Instant::now();
        self.track_toggle_screens();
        self.heal_hold_latches();
        // For the "I'm playing" ping: an atomic, sent on when it changes.
        horizon_client::heartbeat::set_lan(self.is_published());
        self.update_discord_presence();
        let steps = self.clock.advance(elapsed);
        for _ in 0..steps {
            let started = Instant::now();
            if self.title.is_none() && !self.pause {
                horizon_client::spans::time("tick", || self.tick())?;
                self.tick_ambience();
                self.tick_fov();
            }
            self.tick_win_screen()?;
            self.native_configs_tick_client();
            // Gui.tick runs ChatListener.tick paused or not.
            self.tick_chat_queue();
            self.tick_music();
            self.tick_sound_engine();
            self.clock.record(started.elapsed());
            if let Some(report) = self.clock.take_sprint_report() {
                self.sprint_report(&report);
            }
        }
        self.extras_frame(elapsed);
        self.extras_retry()?;
        self.apply_mouse_look();
        self.pad_look(elapsed.as_secs_f32());
        self.advance_flight();
        self.tick_world_icon();
        let ticked = Instant::now();
        let rendered = match self.render() {
            // The callback can fire during this frame; rebuild on
            // the next redraw rather than exiting on its error.
            Err(_)
                if self
                    .renderer
                    .modern_ref()
                    .is_some_and(Renderer::device_lost) =>
            {
                false
            }
            result => result?,
        };
        self.frame_drawn = rendered;
        self.apply_cursor_icon();
        // The frame's spans, taken every frame they are recorded so they never
        // pile up: the slow-frame line's, and the session's (`perf.frame`).
        let frame_spans = if horizon_client::spans::recording_here() {
            horizon_client::spans::take_raw()
        } else {
            Vec::new()
        };
        if let Some(threshold) = self.options.spikes
            && let Some(r) = self.renderer.modern_ref()
        {
            let cpu = now.elapsed().saturating_sub(r.last_wait).as_secs_f32() * 1000.0;
            // Spans timed between frames (`idle`) come first.
            let spans = horizon_client::spans::merge(&frame_spans);
            let between = between.as_secs_f32() * 1000.0;
            if cpu > threshold || between > threshold {
                // The render thread's own CPU time in the frame:
                // far below the wall time, the frame waited.
                let worked = horizon_client::spans::thread_cpu()
                    .zip(frame_cpu)
                    .map_or_else(String::new, |(end, start)| {
                        format!(", cpu {:.1} ms", (end - start).as_secs_f32() * 1000.0)
                    });
                let gap = if between > threshold {
                    format!(", {between:.1} ms before it")
                } else {
                    String::new()
                };
                log::info!(
                    "Horizon: slow frame {cpu:.1} ms{worked}{gap} ({steps} ticks): {}",
                    horizon_client::spans::describe(&spans)
                );
            }
        }
        if let Some(r) = self.renderer.modern_mut() {
            let cpu = now.elapsed().saturating_sub(r.last_wait);
            if rendered {
                let ms = |d: Duration| d.as_secs_f32() * 1000.0;
                self.frame_graph
                    .record(ms(elapsed), ms(cpu), r.gpu_times.last().copied());
                if let Some(&gpu) = r.gpu_times.last() {
                    self.gpu_utilization = f64::from(gpu) * 100.0 / f64::from(ms(cpu).max(0.001));
                }
                self.frame_stats.record_cpu(ms(cpu));
                let render = r.last_encode;
                self.frame_stats.record_phases([
                    ms(updated - now),
                    ms(ticked - updated),
                    ms(ticked.elapsed().saturating_sub(r.last_wait + render)),
                    ms(render),
                ]);
                self.frame_stats
                    .record_block_entities(r.static_block_entity_frame_stats());
            }
            self.frame_stats.record_gpu(&r.gpu_times);
            r.gpu_times.clear();
        }
        horizon_client::ram_limit::frame();
        self.limit_frame();
        self.pause = self.pauses_game();
        if let Some(server) = &self.integrated {
            server.set_paused(self.pause);
            server.set_distances(
                self.render_distance_in_use(),
                self.settings.simulation_distance,
                self.settings.entity_distance,
            );
        }
        self.clock.end_frame(self.pause);
        let mut consumed = false;
        let finished = self
            .renderer
            .facade(Default::default(), Default::default())
            .and_then(|mut renderer| renderer.take_screenshot_result());
        if let Some(result) = &finished
            && self.ui.screenshot_message.is_some()
        {
            self.finish_screenshot(result);
            consumed = true;
        }
        if let Some(result) = &finished
            && let Some(reply) = self.pending_screenshot.take()
        {
            let _ = reply.send(
                match result {
                    Ok([w, h]) => {
                        serde_json::json!({ "ok": true, "width": w, "height": h })
                    }
                    Err(e) => serde_json::json!({ "error": e }),
                }
                .to_string(),
            );
            consumed = true;
        }
        if let Some(result) = self
            .renderer
            .modern_mut()
            .and_then(|r| r.capture_result.take())
            && let Some(reply) = self.pending_render_capture.take()
        {
            let value = result.unwrap_or_else(|e| serde_json::json!({ "error": e }));
            let _ = reply.send(value.to_string());
        }
        // A result nobody asked for was the world icon's
        // (`tick_world_icon`): a failure tries again a second
        // later, as `tryTakeScreenshotIfNeeded` does.
        if !consumed && let Some(Err(error)) = finished {
            log::warn!("Horizon: couldn't save the world icon: {error}");
        }
        if rendered {
            self.frame_stats.record(elapsed.as_secs_f32() * 1000.0);
            self.debug_charts.frame_times.log(elapsed.as_nanos() as i64);
            if self.frames == 0 {
                // The launcher times Play to this line (launcher/src/perf.rs).
                log::info!(
                    "Horizon: first frame after {} ms",
                    self.started.elapsed().as_millis()
                );
            }
            if self.title.is_some() {
                self.perf.title_shown();
            } else if self.screen == Screen::Playing && self.player.is_some() {
                self.perf
                    .frame(elapsed.as_secs_f32() * 1000.0, &frame_spans);
                if startup::filling() {
                    let mut fill: Vec<(&'static str, usize)> = self.streaming.stages().to_vec();
                    fill.push(("resident", self.world.chunks.len()));
                    fill.push(("uploads", self.pending_uploads.len()));
                    fill.push(("backlog", self.remote_backlog()));
                    startup::counters("world.fill", &fill);
                }
                if startup::on() {
                    startup::view_filled(
                        self.world.chunks.len(),
                        self.streaming.pending()
                            + self.pending_uploads.len()
                            + self.remote_backlog(),
                        self.streaming.radius.max(0) as u32,
                    );
                }
                self.world_frames += 1;
            }
            self.frames += 1;
            self.fps_frames += 1;
            horizon_client::crash::debug_crash_tick();
            if self.fps_at.elapsed() >= Duration::from_secs(1) {
                self.fps = self.fps_frames as f32 / self.fps_at.elapsed().as_secs_f32();
                self.fps_frames = 0;
                self.fps_at = Instant::now();
            }
            self.frame_end = Instant::now();
            if self.options.frames.is_some_and(|f| self.frames >= f)
                || self
                    .options
                    .world_frames
                    .is_some_and(|f| self.world_frames >= f)
            {
                event_loop.exit();
            }
        }
        Ok(())
    }
}
