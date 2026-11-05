//! Actual Game native menu ownership/submission. The visible menu pump is a
//! platform adapter only, not Game simulation. World/reload remain explicit
//! unsupported actions; offscreen startup stays bounded bootstrap validation.
use super::*;
use horizon_client::render_contract::{
    Backend, FrameOptions, FrameOutcome, ResourceGeneration, VisibleSections,
};
use horizon_client::renderer_owner::RendererOwner;

pub(super) fn initialize(game: &mut Game<'_>) -> Result<()> {
    game.options.renderer.validate(&game.options)?;
    if game.renderer.backend().is_some() {
        return Err("native bootstrap requires an empty Game renderer owner".into());
    }
    let context = horizon_client_render::native_loading::Context::new(
        render_startup::Renderer::context_kind(game.options.offscreen)?,
        game.options.window_size,
    )?;
    initialize_with_context(game, context)
}

pub(super) fn initialize_with_context(
    game: &mut Game<'_>,
    context: horizon_client_render::native_loading::Context,
) -> Result<()> {
    game.options.renderer.validate(&game.options)?;
    if game.renderer.backend().is_some() {
        return Err("native startup requires an empty Game renderer owner".into());
    }
    game.renderer = RendererOwner::legacy_menu_with_context(
        context,
        &game.pack.render,
        &game.gui.atlas,
        ResourceGeneration(1),
    )?;
    // The boot producer's cached UI-theme UVs belong to its retired atlas.
    horizon_client::ui_theme::reset_atlas();
    Ok(())
}

pub(super) fn render_menu(game: &mut Game<'_>) -> Result<FrameOutcome> {
    if game.renderer.backend() != Some(Backend::LegacyGl)
        || !game.options.renderer.is_native()
        || game.title.is_none()
        || game.player.is_some()
        || game.remote.is_some()
        || game.integrated_starting.is_some()
        || game.pending_entry.is_some()
        || game.pack_reload.is_some()
    {
        return Err("native Game currently supports static menus only; world/loading/reload submission is not connected".into());
    }
    game.gui.atlas.font.begin_frame();
    game.glint_frame = game.capture_glint_frame();
    game.observe_screen();
    let size = game
        .renderer
        .facade(VisibleSections::default(), FrameOptions::default())
        .ok_or("missing native Game renderer")?
        .size();
    // The actual Game producer: includes the real Title, onboarding, widgets,
    // font, toasts and any supported GUI geometry. No synthetic preview UI.
    let hud = game.hud(size);
    let menu = game.options.renderer.menu(&hud);
    game.renderer
        .facade(VisibleSections::default(), FrameOptions::default())
        .ok_or("missing native Game renderer")?
        .render_menu(&menu)
}

pub(super) fn run(game: Game<'_>) -> Result<()> {
    run_inner(game, None)
}
pub(super) fn run_with_loading(game: Game<'_>, loading_presentations: u64) -> Result<()> {
    run_inner(game, Some(loading_presentations))
}
fn run_inner(mut game: Game<'_>, loading_presentations: Option<u64>) -> Result<()> {
    game.options.renderer.validate(&game.options)?;
    let bootstrap = game.options.native_bootstrap_only;
    let outcome = if bootstrap {
        render_menu(&mut game)?
    } else {
        super::native_menu::run(&mut game)?
    };
    if bootstrap {
        match (game.options.offscreen, outcome) {
            (true, FrameOutcome::Offscreen) | (false, FrameOutcome::Presented) => {}
            _ => {
                return Err(
                    "native bootstrap did not complete the requested offscreen/window presentation"
                        .into(),
                );
            }
        }
        game.frames = 1;
    }
    game.frame_drawn = outcome.was_presented();
    log::info!(
        "native Game menu: bootstrap_only={bootstrap}, last submission {outcome:?}; no world/playable qualification"
    );
    if let Some(path) = &game.options.report {
        let generation = game
            .renderer
            .facade(VisibleSections::default(), FrameOptions::default())
            .ok_or("missing native owner after bootstrap")?
            .resource_generation();
        let report = serde_json::json!({
            "scenario": if bootstrap {"native-game-bootstrap-only"} else {"native-game-menu-only"},
            "game_constructed": true,
            "loading_presentations": loading_presentations.unwrap_or(0),
            "loading_context_transferred": loading_presentations.is_some(),
            "loading_worker_joined": loading_presentations.is_some(),
            "backend": "native-gl2",
            "resource_generation": generation.map(|g|g.0),
            "menu_submissions": game.frames,
            "outcome": format!("{outcome:?}"),
            "presented": outcome.was_presented(),
            "playable": false,
            "world_opened": false,
        });
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    // No simulation/save/shut_down path was entered. Drop releases the native
    // owner; do NOT persist a temporary renderer profile into player settings.
    Ok(())
}
