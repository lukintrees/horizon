//! `horizon-client-gl2 [--scene flat|terrain] [--frames N] [--out FILE.json] [--png FILE.png]`: the game's
//! OpenGL 2.1 window. `flat` is gate G1; `terrain` is the GL2 terrain.
//!
//! `--frames N` ends the run after N presented frames; 0 (the default) keeps the window open until it is
//! closed. The JSON report goes to stdout and to `--out`; `--png` writes the last frame.
//! Exit status: 0 when every required check passed, 1 when a check failed, 2 when there is no window,
//! no OpenGL 2.1 context, or the frame could not run.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut frames = 0u32;
    let mut out = None;
    let mut png = None;
    let mut scene = "flat".to_string();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--frames" => match args.next().and_then(|value| value.parse().ok()) {
                Some(count) => frames = count,
                None => {
                    eprintln!("--frames needs a number");
                    return ExitCode::from(2);
                }
            },
            "--out" => out = args.next(),
            "--png" => png = args.next(),
            "--scene" => match args.next() {
                Some(name) if name == "flat" || name == "terrain" => scene = name,
                _ => {
                    eprintln!("--scene needs flat or terrain");
                    return ExitCode::from(2);
                }
            },
            other => {
                eprintln!(
                    "unknown argument: {other}\nusage: horizon-client-gl2 [--scene flat|terrain] [--frames N] [--out FILE.json] [--png FILE.png]"
                );
                return ExitCode::from(2);
            }
        }
    }
    run(frames, out, png, scene)
}

#[cfg(windows)]
fn run(frames: u32, out: Option<String>, png: Option<String>, scene: String) -> ExitCode {
    use horizon_client_gl2::window::{Options, Scene};
    let scene = if scene == "terrain" { Scene::Terrain } else { Scene::Flat };
    let (status, json) = horizon_client_gl2::window::run(Options { frames, out, png, scene });
    print!("{json}");
    ExitCode::from(status as u8)
}

#[cfg(not(windows))]
fn run(_frames: u32, _out: Option<String>, _png: Option<String>, _scene: String) -> ExitCode {
    eprintln!("the game window is a WGL window: it runs on Windows only");
    ExitCode::from(2)
}
