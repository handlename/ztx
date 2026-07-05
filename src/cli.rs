use clap::{Parser, Subcommand};

/// Mediator between Zed terminal sessions and AI agent CLIs.
#[derive(Parser)]
#[command(name = "zediator", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run an agent CLI wrapped in the zediator PTY proxy
    Run {
        /// How to handle the child's OSC title sequences.
        /// Defaults to `managed` when an adapter matches, `passthrough` otherwise.
        #[arg(long, value_enum)]
        title_mode: Option<crate::title::TitleMode>,

        /// Prefix used by `--title-mode prefix` (defaults to "<command>: ")
        #[arg(long)]
        title_prefix: Option<String>,

        /// CLI-specific adapter selection
        #[arg(long, value_enum, default_value_t)]
        adapter: crate::adapter::AdapterKind,

        /// Agent CLI command and its arguments (pass after `--`)
        #[arg(required = true, trailing_var_arg = true)]
        command: Vec<String>,
    },

    /// Export the latest session transcript for this directory as Markdown.
    /// (Inside a running wrapper, `ctrl-] e` exports the live session,
    /// including the PTY-capture fallback.)
    Export {
        /// CLI-specific adapter selection
        #[arg(long, value_enum, default_value_t)]
        adapter: crate::adapter::AdapterKind,

        /// Write the Markdown to stdout instead of opening an editor
        #[arg(long)]
        stdout: bool,
    },

    /// Send a file reference / selected text into a running session
    /// (designed to be called from a Zed task; see `--from-zed-env`)
    Send {
        /// Read file/line/text from the ZED_RELATIVE_FILE, ZED_ROW and
        /// ZED_SELECTED_TEXT environment variables instead of flags.
        ///
        /// Prefer this in Zed tasks: Zed runs tasks through `zsh -c "..."`
        /// and interpolates $ZED_* into the command line, so passing the
        /// selection as an argument lets the shell re-execute it. Reading
        /// the values from the environment avoids that injection entirely.
        #[arg(long)]
        from_zed_env: bool,

        /// File to reference (e.g. a path string)
        #[arg(long)]
        file: Option<String>,

        /// Line number to reference
        #[arg(long)]
        line: Option<u32>,

        /// Selected text to attach as a fenced block
        #[arg(long)]
        text: Option<String>,

        /// Target session by wrapper pid (see `zediator sessions`)
        #[arg(long)]
        pid: Option<u32>,

        /// Target session by explicit socket path
        #[arg(long)]
        socket: Option<std::path::PathBuf>,

        /// Free-form message text
        #[arg(trailing_var_arg = true)]
        message: Vec<String>,
    },

    /// List running zediator sessions
    Sessions,

    /// Generate editor integration (keybinding + task) for zediator
    Setup {
        #[command(subcommand)]
        target: SetupTarget,
    },
}

#[derive(Subcommand)]
pub enum SetupTarget {
    /// Merge a zediator task and keybinding into the Zed configuration
    Zed {
        /// Apply changes without asking for confirmation
        #[arg(long)]
        yes: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_run_with_trailing_args() {
        let cli = Cli::try_parse_from(["zediator", "run", "--", "claude", "--continue"]).unwrap();
        let Command::Run { command, .. } = cli.command else {
            panic!("expected run subcommand");
        };
        assert_eq!(command, vec!["claude", "--continue"]);
    }

    #[test]
    fn parses_title_mode_and_adapter_flags() {
        let cli = Cli::try_parse_from([
            "zediator",
            "run",
            "--title-mode",
            "managed",
            "--adapter",
            "claude",
            "--",
            "claude",
        ])
        .unwrap();
        let Command::Run {
            title_mode,
            adapter,
            ..
        } = cli.command
        else {
            panic!("expected run subcommand");
        };
        assert_eq!(title_mode, Some(crate::title::TitleMode::Managed));
        assert_eq!(adapter, crate::adapter::AdapterKind::Claude);
    }

    #[test]
    fn parses_send_from_zed_env() {
        let cli = Cli::try_parse_from(["zediator", "send", "--from-zed-env"]).unwrap();
        let Command::Send { from_zed_env, .. } = cli.command else {
            panic!("expected send subcommand");
        };
        assert!(from_zed_env);
    }

    #[test]
    fn run_requires_a_command() {
        assert!(Cli::try_parse_from(["zediator", "run"]).is_err());
    }
}
