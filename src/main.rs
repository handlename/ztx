mod adapter;
mod cli;
mod logging;
mod pty;
mod term;
mod term_guard;
mod title;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let _log_guard = logging::init();
    term_guard::install_panic_hook();

    let args = cli::Cli::parse();
    match args.command {
        cli::Command::Run {
            title_mode,
            title_prefix,
            adapter,
            command,
        } => {
            let opts = pty::RunOptions {
                title_mode,
                title_prefix,
                adapter,
            };
            match pty::run(&command, opts) {
                Ok(code) => ExitCode::from(code.min(u8::MAX as u32) as u8),
                Err(err) => {
                    eprintln!("zediator: {err}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}
