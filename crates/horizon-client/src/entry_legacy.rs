//! Startup for the real `legacy-gl` game loop.
//!
//! The wgpu entry point installs native render hooks, crash reporting and the
//! full logging stack; none of those belong to the minimal GL2 build. This
//! entry only loads the bundled language table and runs the cfg-selected game.

use horizon_client_core::lang;

use crate::game;

/// Start the game. The executable supplies the allocator and the mod API marker.
pub fn main() {
    main_with(());
}

/// Kept for callers that used `entry::main_with` in the wgpu build.
pub fn main_with(_: ()) {
    lang::set(lang::Language::load(&[], lang::DEFAULT));
    if let Err(error) = game::run_with() {
        eprintln!("{}", lang::tr("horizon.legacyGl.startupError", &[&error]));
        let usage = error.is::<game::UsageError>();
        std::process::exit(if usage {
            game::exit_code::USAGE
        } else {
            game::exit_code::ERROR
        });
    }
}
