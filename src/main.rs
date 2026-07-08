mod adapter;
mod cli;
mod config;
mod debug;
mod export;
mod hint;
mod input;
mod ipc;
mod logging;
mod pty;
mod setup;
mod term;
mod term_guard;
mod title;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let _log_guard = logging::init();
    term_guard::install_panic_hook();

    let args = cli::Cli::parse();
    let config = config::Config::load();
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
                prefix: config.prefix.unwrap_or(input::DEFAULT_PREFIX),
                editor: config.editor,
                status_emoji: config.status_emoji,
            };
            match pty::run(&command, opts) {
                Ok(code) => ExitCode::from(code.min(u8::MAX as u32) as u8),
                Err(err) => {
                    eprintln!("zedic: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        cli::Command::Export { adapter, stdout } => {
            report(run_export(adapter, stdout, config.editor.as_deref()))
        }
        cli::Command::Send {
            from_zed_env,
            file,
            line,
            text,
            socket,
            message,
        } => report(run_send(from_zed_env, file, line, text, socket, message)),
        cli::Command::Sessions => report(run_sessions()),
        cli::Command::Setup { target } => match target {
            cli::SetupTarget::Zed { yes } => report(setup::zed(yes)),
        },
    }
}

fn report(result: std::io::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("zedic: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_send(
    from_zed_env: bool,
    mut file: Option<String>,
    mut line: Option<u32>,
    mut text: Option<String>,
    socket: Option<std::path::PathBuf>,
    message: Vec<String>,
) -> std::io::Result<()> {
    // Pull the selection from the environment when asked. Explicit flags win
    // over env values so the flag form stays usable for scripting/testing.
    if from_zed_env {
        let env_nonempty = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
        file = file.or_else(|| env_nonempty("ZED_RELATIVE_FILE"));
        line = line.or_else(|| env_nonempty("ZED_ROW").and_then(|v| v.parse().ok()));
        text = text.or_else(|| env_nonempty("ZED_SELECTED_TEXT"));
    }
    let payload = ipc::compose_message(file.as_deref(), line, text.as_deref(), &message);
    if payload.is_empty() {
        return Err(std::io::Error::other(
            "nothing to send (pass --file/--text, a message, or --from-zed-env)",
        ));
    }
    let target = ipc::resolve_socket(socket)?;
    ipc::send(&target, &payload)
}

fn run_sessions() -> std::io::Result<()> {
    let sessions = ipc::list_sessions();
    if sessions.is_empty() {
        println!(
            "no running zedic sessions ({})",
            ipc::socket_dir().display()
        );
        return Ok(());
    }
    for session in sessions {
        let state = if session.alive { "alive" } else { "stale" };
        let pid = session
            .pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "?".into());
        let cwd = session
            .cwd
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        println!("{pid}\t{state}\t{cwd}");
    }
    Ok(())
}

fn run_export(
    kind: adapter::AdapterKind,
    to_stdout: bool,
    config_editor: Option<&str>,
) -> std::io::Result<()> {
    let mut resolved = adapter::resolve_for_export(kind).ok_or_else(|| {
        std::io::Error::other(
            "export outside a wrapper needs an adapter; \
             inside a running session use ctrl-] e for the capture fallback",
        )
    })?;
    let transcript = resolved
        .transcript_path()
        .ok_or_else(|| std::io::Error::other("no session transcript found for this directory"))?;
    let markdown = export::transcript_to_markdown(&transcript)?;
    if to_stdout {
        print!("{markdown}");
        return Ok(());
    }
    let path = export::write_export(&markdown)?;
    export::open_in_editor(&path, config_editor)?;
    eprintln!("exported to {}", path.display());
    Ok(())
}
