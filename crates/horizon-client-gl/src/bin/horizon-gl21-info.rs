//! `horizon-gl21-info`: what this machine can do on Horizon's OpenGL 2.1 path, and why.
//!
//! ```sh
//! horizon-gl21-info                 # the capability table, from the environment
//! horizon-gl21-info --probe         # can a GL library be opened at all, and what lacks
//! horizon-gl21-info --check-shaders # measure the bundled shaders, no driver needed
//! horizon-gl21-info --strict        # the same as HORIZON_GL21_STRICT=1
//! horizon-gl21-info --json          # machine-readable, for a bug report or CI
//! ```
//!
//! Without a context it still answers the questions a bug report needs: which library was
//! found, which entry points it lacks, which shaders break the target's budget. That is the
//! mode this binary runs in on a build machine with no GPU, and the mode CI runs.

use std::process::ExitCode;

use horizon_client_gl::budget::{self, Stage};
use horizon_client_gl::caps::{Caps, Feature, Limits, Version};
use horizon_client_gl::loader::GlLibrary;

/// The shaders that ship with the legacy path, embedded so the binary can check them without a
/// prepared game install: a name, its vertex source, its fragment source.
///
/// Every pair goes through [`budget`] here and through `tools/gl21_glsl.py` in CI. Two
/// independent measurements of the same files is one more than the job needs, and it is the
/// arrangement that caught 19 components of dead uniforms in the first drafts.
const SHADERS: &[(&str, &str, &str)] = &[
    (
        "terrain",
        include_str!("../shaders/terrain/terrain.vert"),
        include_str!("../shaders/terrain/terrain.frag"),
    ),
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let strict = args.iter().any(|a| a == "--strict")
        || std::env::var("HORIZON_GL21_STRICT").map(|v| !v.is_empty() && v != "0").unwrap_or(false);

    let want_probe = args.iter().any(|a| a == "--probe");
    let want_shaders = args.iter().any(|a| a == "--check-shaders");
    let want_context = args.iter().any(|a| a == "--context");
    let want_help = args.iter().any(|a| a == "--help" || a == "-h");
    let unknown: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| {
            !matches!(
                *a,
                "--json"
                    | "--strict"
                    | "--probe"
                    | "--check-shaders"
                    | "--context"
                    | "--help"
                    | "-h"
            )
        })
        .collect();
    if !unknown.is_empty() {
        eprintln!("horizon-gl21-info: unknown argument(s): {}", unknown.join(" "));
        eprintln!("try --help");
        return ExitCode::from(2);
    }

    if want_help {
        println!(
            "horizon-gl21-info: Horizon's OpenGL 2.1 path\n\n\
             --probe           open the system GL library and list missing entry points\n\
             --check-shaders   measure the bundled GLSL against the X1650's budgets\n\
             --context         create a real context if this machine has a driver\n\
             --strict          use the target's capability table (HORIZON_GL21_STRICT=1)\n\
             --json            machine-readable output\n"
        );
        return ExitCode::SUCCESS;
    }

    let mut failed = false;

    // 1. What the environment offers, before any context exists.
    let probe = horizon_client_gl::gl::probe();
    if !json {
        println!("Horizon GL 2.1 path, crate {}", horizon_client_gl::VERSION);
        match &probe {
            Ok(text) => println!("library: {text}"),
            Err(text) => println!("library: not usable ({text})"),
        }
    }

    if want_probe {
        match GlLibrary::open() {
            Ok(lib) => {
                let missing = lib.missing_essentials();
                if !json {
                    println!("open: {}", lib.name());
                    println!(
                        "entry points: {} of {} required present",
                        horizon_client_gl::loader::ESSENTIAL.len() - missing.len(),
                        horizon_client_gl::loader::ESSENTIAL.len()
                    );
                    if !missing.is_empty() {
                        println!("missing: {}", missing.join(", "));
                    }
                }
                if !missing.is_empty() {
                    failed = true;
                }
            }
            Err(e) => {
                if json {
                    println!(
                        "{{\"library\": null, \"error\": {:?}, \"tried\": {:?}}}",
                        e.message, e.tried
                    );
                } else {
                    println!("open: failed");
                    println!("  reason: {}", e.message);
                    for t in &e.tried {
                        println!("  tried: {t}");
                    }
                }
                failed = true;
            }
        }
    }

    // 2. The capability table. From a real context when asked and possible, else from the
    //    target's table, so the check is still meaningful on a machine that cannot draw.
    let caps = if want_context {
        match real_caps(strict) {
            Ok(caps) => caps,
            Err(message) => {
                if json {
                    println!("{{\"context\": null, \"error\": {message:?}}}");
                } else {
                    println!("context: not created ({message})");
                }
                failed = true;
                target_caps(strict)
            }
        }
    } else {
        target_caps(strict)
    };

    if want_shaders {
        let measurements = check_shaders(&caps);
        let over = measurements.iter().filter(|m| m.errors > 0).count();
        if !json {
            println!("\nshaders checked: {}", measurements.len());
            for m in &measurements {
                let status = if m.errors == 0 { "ok  " } else { "OVER" };
                println!(
                    "  {status} {:<34} uniform {:>4}/{:<4} varying {:>3}/{:<3} samplers {}/{}",
                    m.path,
                    m.uniform_components,
                    m.uniform_limit,
                    m.varying_floats,
                    m.varying_limit,
                    m.samplers,
                    m.sampler_limit
                );
                for e in &m.details {
                    println!("         {e}");
                }
            }
            if over > 0 {
                println!("{over} shader(s) over the GL 2.1 budget");
            }
        } else {
            println!("{}", measurements_json(&measurements));
        }
        if over > 0 {
            failed = true;
        }
    }

    if !json && !want_shaders && !want_probe {
        println!();
        println!("{}", caps.summary());
        println!();
        println!("{}", feature_table(&caps));
    }

    ExitCode::from(if failed { 1 } else { 0 })
}

fn target_caps(strict: bool) -> Caps {
    let mut caps = Caps::detect(
        Version::parse(&format!(
            "2.1.8545 Release (assumed: HORIZON_GL21_STRICT{})",
            if strict { "=1" } else { " is off; no context was created" }
        )),
        "1.20",
        "ATI Technologies Inc. (assumed)",
        "Radeon X1600/X1650 Series (assumed target)",
        std::iter::empty(),
        Limits::X1650,
    );
    caps = caps.strict_over();
    caps
}

fn real_caps(strict: bool) -> Result<Caps, String> {
    let library = GlLibrary::open().map_err(|e| e.message)?;
    let mut options = horizon_client_gl::gl::Options::from_env();
    options.strict = strict;
    // SAFETY: this binary owns the thread and no other context; the library outlives the
    // context because the context keeps it.
    let context = unsafe { horizon_client_gl::gl::Context::new(library, options) }
        .map_err(|e| e.to_string())?;
    Ok(context.caps().clone())
}

struct Measurement {
    path: String,
    uniform_components: u32,
    uniform_limit: u32,
    varying_floats: u32,
    varying_limit: u32,
    samplers: u32,
    sampler_limit: u32,
    errors: usize,
    details: Vec<String>,
}

fn check_shaders(caps: &Caps) -> Vec<Measurement> {
    let mut out = Vec::new();
    for (path, vertex, fragment) in SHADERS {
        for (stage, source) in [(Stage::Vertex, *vertex), (Stage::Fragment, *fragment)] {
            let measured = budget::measure(source, stage);
            let mut error = budget::check(&measured, stage, &caps.limits).err();
            if stage == Stage::Vertex {
                // The pair is checked once, when both sources have been read: a varying
                // declared differently in the two stages is the target linker's silent
                // failure, and the reason this loop keeps the vertex measurement.
                let fragment = budget::measure(fragment, Stage::Fragment);
                if error.is_none() {
                    error = budget::check_linked(&measured, &fragment).err();
                }
            }
            out.push(Measurement {
                path: format!("{path} ({})", stage.name()),
                uniform_components: measured.uniform_components,
                uniform_limit: match stage {
                    Stage::Vertex => caps.limits.max_vertex_uniform_components,
                    Stage::Fragment => caps.limits.max_fragment_uniform_components,
                },
                varying_floats: measured.varying_floats,
                varying_limit: caps.limits.max_varying_floats,
                samplers: measured.samplers,
                sampler_limit: caps.limits.max_texture_units,
                errors: usize::from(error.is_some()),
                details: error.into_iter().map(|e| e.to_string()).collect(),
            });
        }
    }
    out
}

fn feature_table(caps: &Caps) -> String {
    let mut s = String::from("feature                       here    provided by\n");
    for f in Feature::ALL {
        s.push_str(&format!(
            "{:<30}{:<8}{}\n",
            format!("{f:?}"),
            if caps.has(*f) { "yes" } else { "no" },
            f.source()
        ));
    }
    s
}

/// A JSON writer small enough not to need serde in a crate that has no other use for it.
fn measurements_json(items: &[Measurement]) -> String {
    let mut s = String::from("[");
    for (i, m) in items.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"path\": {:?}, \"uniform_components\": {}, \"uniform_limit\": {}, \
             \"varying_floats\": {}, \"varying_limit\": {}, \"samplers\": {}, \
             \"sampler_limit\": {}, \"errors\": {}, \"details\": {:?}}}",
            m.path,
            m.uniform_components,
            m.uniform_limit,
            m.varying_floats,
            m.varying_limit,
            m.samplers,
            m.sampler_limit,
            m.errors,
            m.details
        ));
    }
    s.push(']');
    s
}
