mod adapter;
mod cli;
mod config;
mod debug;
mod export;
mod hint;
mod input;
mod ipc;
mod logging;
mod notify;
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
    let config = config::Config::load();
    match args.command {
        cli::Command::Run {
            title_mode,
            title_prefix,
            adapter,
            force,
            no_force,
            command,
        } => {
            // Hand Claude Code the same session name ztx shows in the title, so
            // the session picker and `--resume <name>` speak the same language.
            // A cwd we cannot read is not worth failing over: run unnamed.
            let command = match std::env::current_dir() {
                Ok(cwd) => adapter::with_session_name(adapter, &command, &cwd),
                Err(_) => command,
            };
            let opts = pty::RunOptions {
                title_mode,
                title_prefix,
                adapter,
                prefix: config.prefix.unwrap_or(input::DEFAULT_PREFIX),
                editor: config.editor,
                status_emoji: config.status_emoji,
                force: cli::resolve_force(force, no_force, config.run.force),
            };
            match pty::run(&command, opts) {
                Ok(code) => ExitCode::from(code.min(u8::MAX as u32) as u8),
                Err(err) => {
                    eprintln!("ztx: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        cli::Command::Export { adapter, stdout } => {
            report(run_export(adapter, stdout, config.editor.as_deref()))
        }
        cli::Command::Notify {
            from_hook,
            wake,
            transcript,
            socket,
        } => report(run_notify(
            from_hook,
            wake,
            transcript,
            socket,
            config.notify,
            config.status_emoji,
        )),
        cli::Command::Sessions => report(run_sessions()),
    }
}

fn report(result: std::io::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("ztx: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_notify(
    from_hook: bool,
    wake: bool,
    transcript: Option<std::path::PathBuf>,
    socket: Option<std::path::PathBuf>,
    notify_cfg: config::NotifyConfig,
    status_emoji: config::StatusEmoji,
) -> std::io::Result<()> {
    let mut controls: Vec<ipc::Control> = Vec::new();
    let mut hook_cwd: Option<std::path::PathBuf> = None;
    // Carries the hook event + message so a desktop notification can fire once
    // we confirm a live session below.
    let mut hook_event: Option<(String, Option<String>)> = None;

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
        if let Some(event) = hook.hook_event_name {
            hook_event = Some((event, hook.message));
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
    let Some(target) = ipc::notify_target(hook_cwd.clone(), socket) else {
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

    // A live session exists, so this is a real ztx-wrapped run: surface a
    // desktop notification for attention-worthy events. Best-effort and fully
    // independent of the title refresh above.
    if notify_cfg.desktop
        && let Some((event, message)) = hook_event
    {
        notify::desktop(
            &event,
            hook_cwd.as_deref(),
            message.as_deref(),
            notify_cfg.sound.as_deref(),
            &status_emoji,
        );
    }
    Ok(())
}

/// The subset of the JSON Claude Code delivers to a hook command on stdin that
/// ztx uses. Other fields (`session_id`, …) are ignored.
#[derive(serde::Deserialize, Default)]
struct HookInput {
    cwd: Option<std::path::PathBuf>,
    transcript_path: Option<std::path::PathBuf>,
    /// The hook event (`Notification`, `Stop`, …); drives desktop notifications.
    hook_event_name: Option<String>,
    /// Human-readable message Claude Code supplies for `Notification` events.
    message: Option<String>,
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
        println!("no running ztx sessions ({})", ipc::socket_dir().display());
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
