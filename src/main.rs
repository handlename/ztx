mod cli;
mod pty;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
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
