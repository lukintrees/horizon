//! Stage-0 desktop GL probe. Offscreen Linux contexts are fixtures, not the game backend.
#[cfg(any(target_os = "linux", windows))]
mod gl;
#[cfg(any(target_os = "linux", windows))]
mod render_smoke;
#[cfg(any(target_os = "linux", windows))]
use horizon_legacy_render::Context;

fn main() {
    if let Err(error) = run() {
        eprintln!("legacy GL probe: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
fn run() -> Result<(), String> {
    Err("this probe implements Linux EGL/OSMesa and Windows WGL only".into())
}

#[cfg(any(target_os = "linux", windows))]
fn run() -> Result<(), String> {
    let mut require_llvmpipe = false;
    let mut shader_version = "110";
    let mut context_kind = if cfg!(windows) { "wgl" } else { "egl" };
    let mut expected_gl = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--require-llvmpipe" => require_llvmpipe = true,
            "--shader-version" => {
                shader_version = match args.next().as_deref() {
                    Some("110") => "110",
                    Some("120") => "120",
                    _ => return Err("--shader-version requires 110 or 120".into()),
                };
            }
            "--expect-gl" => {
                expected_gl = Some(args.next().ok_or("--expect-gl requires a version")?);
            }
            "--context" => {
                context_kind = match args.next().as_deref() {
                    Some("egl") => "egl",
                    Some("osmesa") => "osmesa",
                    Some("wgl") => "wgl",
                    _ => return Err("--context requires egl, osmesa or wgl".into()),
                };
            }
            "--help" => {
                println!(
                    "horizon-legacy-gl-probe [--context egl|osmesa|wgl] [--require-llvmpipe] [--shader-version 110|120] [--expect-gl 2.0|2.1]"
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }
    let context = Context::new(context_kind, [64, 64])?;
    let mut report = gl::probe(&context, shader_version)?;
    if require_llvmpipe && !report.renderer.to_ascii_lowercase().contains("llvmpipe") {
        return Err(format!("expected llvmpipe, got {}", report.renderer));
    }
    if let Some(version) = expected_gl {
        if report.version.split_whitespace().next() != Some(version.as_str()) {
            return Err(format!("expected GL {version}, got {}", report.version));
        }
    }
    report.backend_checks = render_smoke::run(&context)?;
    report.buffer_swap = context.present()?;
    println!("{}", report.json(shader_version));
    Ok(())
}
