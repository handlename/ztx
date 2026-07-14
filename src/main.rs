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
        cli::Command::Notify {
            from_hook,
            wake,
            transcript,
            socket,
        } => report(run_notify(from_hook, wake, transcript, socket)),
        cli::Command::Sessions => report(run_sessions()),
        cli::Command::Setup { target } => match target {
            cli::SetupTarget::Zed {
                yes,
                preview,
                scope,
            } => report(setup::zed(yes, preview, scope)),
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

fn run_notify(
    from_hook: bool,
    wake: bool,
    transcript: Option<std::path::PathBuf>,
    socket: Option<std::path::PathBuf>,
) -> std::io::Result<()> {
    let mut controls: Vec<ipc::Control> = Vec::new();
    let mut hook_cwd: Option<std::path::PathBuf> = None;

    if from_hook {
        let hook = read_hook_input()?;
        hook_cwd = hook.cwd;
        // Always refresh the title. The transcript path is additive and may be
        // absent or (rarely) non-UTF-8 and thus unencodable, which must never
        // cost us the wake.
        controls.push(ipc::Control::Wake);
        if let Some(path) = hook.transcript_path {
            controls.push(ipc::Control::Transcript { path });
        }
    }
    if wake {
        controls.push(ipc::Control::Wake);
    }
    if let Some(path) = transcript {
        controls.push(ipc::Control::Transcript { path });
    }
    if controls.is_empty() {
        return Err(std::io::Error::other(
            "nothing to notify (pass --wake, --transcript, or --from-hook)",
        ));
    }

    // Best-effort: with no live session this is a silent no-op, so a plugin
    // hook never fails the agent.
    let Some(target) = ipc::notify_target(hook_cwd, socket) else {
        tracing::debug!("no live session for this project; notify skipped");
        return Ok(());
    };
    for control in &controls {
        let frame = match ipc::encode_control(control) {
            Ok(frame) => frame,
            Err(err) => {
                tracing::warn!(error = %err, "skipping uncodable control frame");
                continue;
            }
        };
        if let Err(err) = ipc::send(&target, &frame) {
            tracing::warn!(error = %err, "failed to send notify control frame");
        }
    }
    Ok(())
}

/// The subset of the JSON Claude Code delivers to a hook command on stdin that
/// zedic uses. Unknown fields (`session_id`, `hook_event_name`, …) are ignored.
#[derive(serde::Deserialize, Default)]
struct HookInput {
    cwd: Option<std::path::PathBuf>,
    transcript_path: Option<std::path::PathBuf>,
}

/// Reads and parses the hook JSON from stdin. Best-effort: an empty or
/// malformed payload yields an all-`None` input (so the hook still triggers a
/// plain wake) rather than an error that would surface in the agent session.
fn read_hook_input() -> std::io::Result<HookInput> {
    use std::io::Read;
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
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
        let sock = session.socket.display();
        println!("{pid}\t{state}\t{sock}\t{cwd}");
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
