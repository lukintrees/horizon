//! The native desktop-GL window's start: the GL context the game draws into, the first menu
//! frame, and then the frame pump. Everything else about the start is the one every launch makes
//! (resources, packs, the title screen); only the context and the pump are the native route's.
// game-module-cfg: feature = "legacy-gl"
use super::*;
use horizon_client::{
    render_contract::{FrameOptions, FrameOutcome, ResourceGeneration, VisibleSections},
    renderer_facade::NativeContext,
    renderer_owner::RendererOwner,
};

/// The GL context kind this machine's native window takes: a visible one where the platform has
/// window GL (GLX on Linux, WGL on Windows), an offscreen one where it has a software surface.
/// There is no platform where neither exists.
fn context_kind(offscreen: bool) -> Result<&'static str> {
    match (std::env::consts::OS, offscreen) {
        ("linux", true) => Ok("osmesa"),
        ("linux", false) => Ok("glx"),
        ("windows", _) => Ok("wgl"),
        (other, _) => Err(format!("the native GL2 backend has no window on {other}").into()),
    }
}

/// Create the game's GL context and hand it the resources the game already loaded. The owner is
/// empty before this: a backend already there would mean the start took the modern route.
pub(super) fn initialize(game: &mut Game<'_>) -> Result<()> {
    if game.renderer.backend().is_some() {
        return Err("native startup requires an empty Game renderer owner".into());
    }
    let context = NativeContext::new(
        context_kind(game.options.offscreen)?,
        game.options.window_size,
    )?;
    initialize_with_context(game, context)
}

pub(super) fn initialize_with_context(game: &mut Game<'_>, context: NativeContext) -> Result<()> {
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

/// `Game::render` has already prepared the frame and drawn whatever covers it (a progress screen,
/// a pack reload). What is left is a panorama frame: the title screen, or a screen waiting on a
/// server, which the native backend draws as its flat colour under the same HUD. Anything else is
/// the world, which has its own submission.
pub(super) fn draw_menu(game: &mut Game<'_>) -> Result<FrameOutcome> {
    if !game.options.renderer.is_native()
        || !game.shows_panorama()
        || game.integrated_starting.is_some()
        || game.pending_entry.is_some()
        || game.pack_reload.is_some()
    {
        return Err("the native backend draws a menu here; the world has its own submission".into());
    }
    let size = game.renderer.size().ok_or("missing native Game renderer")?;
    if size.contains(&0) {
        return Ok(FrameOutcome::Deferred);
    }
    let hud = game.hud(size);
    game.submit_menu_frame(&hud)
}

/// The window's frames until it closes, then the shutdown every route runs.
pub(super) fn run(mut game: Game<'_>) -> Result<()> {
    let result = super::native_menu::run(&mut game);
    game.shut_down();
    let outcome = result?;
    game.frame_drawn = outcome.was_presented();
    if let Some(path) = &game.options.report {
        let generation = game
            .renderer
            .facade(VisibleSections::default(), FrameOptions::default())
            .ok_or("missing native owner after the run")?
            .resource_generation();
        let report = serde_json::json!({
            "scenario": "native-game",
            "backend": "native-gl2",
            "resource_generation": generation.map(|g| g.0),
            "frames": game.frames,
            "outcome": format!("{outcome:?}"),
            "presented": outcome.was_presented(),
            "world_opened": game.title.is_none(),
        });
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_context_kind_is_the_platforms_own_and_never_falls_back() {
        match std::env::consts::OS {
            "linux" => {
                assert_eq!(context_kind(true).unwrap(), "osmesa");
                assert_eq!(context_kind(false).unwrap(), "glx");
            }
            "windows" => assert_eq!(context_kind(false).unwrap(), "wgl"),
            _ => assert!(context_kind(false).is_err()),
        }
    }
}
