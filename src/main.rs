use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let [_, input, output] = args.as_slice() else {
        eprintln!("usage: szeel <input.kdl> <output.png>");
        return ExitCode::from(2);
    };
    let (input, output) = (PathBuf::from(input), PathBuf::from(output));

    eprintln!(
        "szeel: rendering is not implemented yet ({} -> {})",
        input.display(),
        output.display()
    );
    ExitCode::FAILURE
}
