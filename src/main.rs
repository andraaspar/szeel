use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use miette::{NamedSource, Report};

use szeel::format::{FormatError, format};
use szeel::parse::{Diag, parse};
use szeel::render::render;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "fmt") {
        return fmt(&args[2..]);
    }
    let [_, input, output] = args.as_slice() else {
        return usage();
    };
    let (input, output) = (PathBuf::from(input), PathBuf::from(output));

    let text = match std::fs::read_to_string(&input) {
        Ok(t) => Arc::new(t),
        Err(e) => {
            eprintln!("szeel: cannot read {}: {e}", input.display());
            return ExitCode::FAILURE;
        }
    };
    let source = NamedSource::new(input.display().to_string(), text.clone());
    let report = |d: Diag| eprintln!("{:?}", Report::new(d.with_source(source.clone())));

    let parsed = match parse(&text) {
        Ok(p) => p,
        Err(e) => {
            // Report each syntax problem on its own, with the file name.
            for d in e.diagnostics {
                report(Diag {
                    message: d.message.unwrap_or_else(|| "syntax error".into()),
                    span: d.span,
                    label: d.label,
                    secondary: Vec::new(),
                    severity: d.severity,
                    help: d.help,
                    source: None,
                });
            }
            return ExitCode::FAILURE;
        }
    };
    for d in parsed.diagnostics {
        report(d);
    }
    let Some(scene) = parsed.scene else {
        return ExitCode::FAILURE;
    };

    let canvas = render(&scene);

    let result = File::create(&output)
        .map_err(png::EncodingError::from)
        .and_then(|f| canvas.write_png(BufWriter::new(f)));
    if let Err(e) = result {
        eprintln!("szeel: cannot write {}: {e}", output.display());
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn usage() -> ExitCode {
    eprintln!("usage: szeel <input.kdl> <output.png>");
    eprintln!("       szeel fmt [--check] <file.kdl>...");
    ExitCode::from(2)
}

/// Reformats scene files in place, or with `--check` lists the ones that
/// need it.
fn fmt(args: &[String]) -> ExitCode {
    let check = args.iter().any(|a| a == "--check");
    let files: Vec<&String> = args.iter().filter(|a| *a != "--check").collect();
    if files.is_empty() || files.iter().any(|f| f.starts_with('-')) {
        return usage();
    }
    let mut ok = true;
    for file in files {
        let text = match std::fs::read_to_string(file) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("szeel: cannot read {file}: {e}");
                ok = false;
                continue;
            }
        };
        let formatted = match format(&text) {
            Ok(f) => f,
            Err(FormatError::Syntax(e)) => {
                eprintln!("{:?}", Report::new(e).with_source_code(NamedSource::new(file, text)));
                ok = false;
                continue;
            }
            Err(e) => {
                eprintln!("szeel: cannot format {file}: {e}");
                ok = false;
                continue;
            }
        };
        if formatted == text {
            continue;
        }
        if check {
            println!("{file}");
            ok = false;
        } else if let Err(e) = std::fs::write(file, formatted) {
            eprintln!("szeel: cannot write {file}: {e}");
            ok = false;
        }
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
