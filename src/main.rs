mod adapter;
mod cli;
mod export;
mod input;
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
        cli::Command::Export { adapter, stdout } => match run_export(adapter, stdout) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("zediator: {err}");
                ExitCode::FAILURE
            }
        },
    }
}

fn run_export(kind: adapter::AdapterKind, to_stdout: bool) -> std::io::Result<()> {
    let mut resolved = adapter::resolve_for_export(kind).ok_or_else(|| {
        std::io::Error::other(
            "export outside a wrapper needs an adapter; \
             inside a running session use ctrl-] e for the capture fallback",
        )
    })?;
    let transcript = resolved.transcript_path().ok_or_else(|| {
        std::io::Error::other("no session transcript found for this directory")
    })?;
    let markdown = export::transcript_to_markdown(&transcript)?;
    if to_stdout {
        print!("{markdown}");
        return Ok(());
    }
    let path = export::write_export(&markdown)?;
    export::open_in_editor(&path)?;
    eprintln!("exported to {}", path.display());
    Ok(())
}
