//! vitrify — the single multi-call binary. `main` is the only
//! place that converts to a process exit (Conventions).

fn main() -> std::process::ExitCode {
    let code = vitrify_cli::run();
    std::process::ExitCode::from(vitrify_cli::exit::ExitCode::from_code(code) as u8)
}
