use std::process::ExitCode;

fn main() -> ExitCode {
    match chromaterm::cli::main() {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(e) => {
            eprintln!("ct: {e:#}");
            ExitCode::from(1)
        }
    }
}
