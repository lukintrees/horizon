//! `horizon-client-gl2 [--frames N] [--out FILE.json]`: the game's OpenGL 2.1 window, drawing gate G1.
//!
//! `--frames N` ends the run after N presented frames; 0 (the default) keeps the window open until
//! it is closed. The JSON report goes to stdout and to `--out`.
//! Exit status: 0 when every required check passed, 1 when a check failed, 2 when there is no window,
//! no OpenGL 2.1 context, or the frame could not run.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut frames = 0u32;
    let mut out = None;
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
            other => {
                eprintln!("unknown argument: {other}\nusage: horizon-client-gl2 [--frames N] [--out FILE.json]");
                return ExitCode::from(2);
            }
        }
    }
    run(frames, out)
}

#[cfg(windows)]
fn run(frames: u32, out: Option<String>) -> ExitCode {
    let (status, json) = horizon_client_gl2::window::run(horizon_client_gl2::window::Options { frames, out });
    print!("{json}");
    ExitCode::from(status as u8)
}

#[cfg(not(windows))]
fn run(_frames: u32, _out: Option<String>) -> ExitCode {
    eprintln!("the game window is a WGL window: it runs on Windows only");
    ExitCode::from(2)
}
