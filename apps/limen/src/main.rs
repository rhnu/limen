use std::process::ExitCode;

/// Marks the executable as a scaffold until the configuration and CLI milestone lands.
fn main() -> ExitCode {
    eprintln!("Limen is not implemented yet. See docs/roadmap.md for the implementation plan.");
    ExitCode::FAILURE
}
