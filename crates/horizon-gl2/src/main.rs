//! `gl2-probe [--out FILE]`: prints the report as JSON on stdout and a summary on stderr.
//! Exit status: 0 when every required check passes, 1 when one fails, 2 when no context exists.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut out_path = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out_path = args.next(),
            other => {
                eprintln!("unknown argument: {other}\nusage: gl2-probe [--out FILE]");
                return ExitCode::from(2);
            }
        }
    }

    match horizon_gl2::probe::run() {
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
            println!("{{\n  \"pass\": false,\n  \"error\": {:?}\n}}", message);
            ExitCode::from(2)
        }
    }
}
