//! Gameplay and startup, above the reusable client library.
// No `#![recursion_limit]` or other inner attribute horizon-mixins doesn't know here: the
// launcher's horizon-mixins then rejects every native mod targeting this crate (#8169).
// The real GL2 backend needs FFI, so unsafe is allowed only in legacy-gl builds.
#![cfg_attr(not(feature = "legacy-gl"), forbid(unsafe_code))]
extern crate self as horizon_client;
pub use horizon_client_lib::*;

#[cfg(feature = "modern-gl")]
pub mod entry;
#[cfg(all(feature = "legacy-gl", not(feature = "modern-gl")))]
#[path = "entry_legacy.rs"]
pub mod entry;

#[cfg(feature = "modern-gl")]
pub mod game;
#[cfg(all(feature = "legacy-gl", not(feature = "modern-gl")))]
#[path = "game_legacy.rs"]
pub mod game;

#[cfg(all(feature = "legacy-gl", not(feature = "modern-gl")))]
pub mod legacy_gl;
#[cfg(feature = "modern-gl")]
pub mod mod_api;
