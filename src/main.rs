use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use miette::{NamedSource, Report};

use szeel::parse::{Diag, parse};
use szeel::render::{Skip, render};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let [_, input, output] = args.as_slice() else {
        eprintln!("usage: szeel <input.kdl> <output.png>");
        return ExitCode::from(2);
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

    let canvas = render(&scene, |i, why| {
        // Paths without gradients were already reported by the parser.
        if why == Skip::Degenerate {
            report(Diag::warning(scene.paths[i].span, "all points of this path are the same; it paints nothing"));
        }
    });

    let result = File::create(&output)
        .map_err(png::EncodingError::from)
        .and_then(|f| canvas.write_png(BufWriter::new(f)));
    if let Err(e) = result {
        eprintln!("szeel: cannot write {}: {e}", output.display());
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
