//! `gl2-probe [--out FILE]`: prints the report as JSON on stdout and a summary on stderr.
//! Exit status: 0 when every required check passes, 1 when one fails, 2 when no context exists.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut out_path = None;
    let mut frame_path = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out_path = args.next(),
            "--frame" => frame_path = args.next(),
            other => {
                eprintln!("unknown argument: {other}\nusage: gl2-probe [--out FILE] [--frame FILE.png]");
                return ExitCode::from(2);
            }
        }
    }

    // A panic is reported like a missing context, so the report file still says why.
    let result = std::panic::catch_unwind(move || horizon_gl2::probe::run(frame_path)).unwrap_or_else(|payload| {
        let text = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        Err(format!("panic: {text}"))
    });
    match result {
        Ok(report) => {
            let json = report.to_json();
            print!("{json}");
            eprint!("{}", report.summary());
            if let Some(path) = out_path {
                let parent = std::path::Path::new(&path).parent().unwrap_or(std::path::Path::new("."));
                let written = std::fs::create_dir_all(parent).and_then(|()| std::fs::write(&path, &json));
                if let Err(e) = written {
                    eprintln!("cannot write {path}: {e}");
                    return ExitCode::from(2);
                }
            }
            if report.passed() { ExitCode::SUCCESS } else { ExitCode::from(1) }
        }
        Err(message) => {
            eprintln!("no OpenGL context: {message}");
            let json = format!("{{\n  \"pass\": false,\n  \"error\": {:?}\n}}\n", message);
            print!("{json}");
            // The report file is what CI keeps, so a failed context leaves its reason there too.
            if let Some(path) = out_path {
                let _ = std::fs::create_dir_all(std::path::Path::new(&path).parent().unwrap_or(std::path::Path::new(".")));
                let _ = std::fs::write(&path, &json);
            }
            ExitCode::from(2)
        }
    }
}
