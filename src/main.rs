mod cli;
mod logging;
mod pty;
mod term;
mod term_guard;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let _log_guard = logging::init();
    term_guard::install_panic_hook();

    let args = cli::Cli::parse();
    match args.command {
        cli::Command::Run { command } => match pty::run(&command) {
            Ok(code) => ExitCode::from(code.min(u8::MAX as u32) as u8),
            Err(err) => {
                eprintln!("zediator: {err}");
                ExitCode::FAILURE
            }
        },
    }
}
