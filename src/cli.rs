use clap::{Parser, Subcommand};

/// Version string combining the crate version with the git commit hash
/// captured at build time (see build.rs), e.g. `0.1.0 (a1b2c3d)`.
const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("ZTX_GIT_HASH"), ")");

/// Mediator between Zed terminal sessions and AI agent CLIs.
#[derive(Parser)]
#[command(name = "ztx", version = VERSION, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run an agent CLI wrapped in the ztx PTY proxy
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

        /// Target a session by explicit socket path (default: this project's
        /// session, keyed by ZED_WORKTREE_ROOT or the current directory)
        #[arg(long)]
        socket: Option<std::path::PathBuf>,

        /// Free-form message text
        #[arg(trailing_var_arg = true)]
        message: Vec<String>,
    },

    /// Notify a running session of an activity change (used by the Claude Code
    /// plugin hooks; a silent no-op when no session runs in this project)
    Notify {
        /// Read the hook JSON from stdin: derive the working directory and
        /// transcript path, then wake the session's title. Prefer this in
        /// plugin hooks over the explicit flags below.
        #[arg(long)]
        from_hook: bool,

        /// Force the session's managed title to refresh immediately
        #[arg(long)]
        wake: bool,

        /// Record the authoritative transcript path for `export`
        #[arg(long)]
        transcript: Option<std::path::PathBuf>,

        /// Target a session by explicit socket path (default: this project's
        /// session, keyed by ZED_WORKTREE_ROOT or the current directory)
        #[arg(long)]
        socket: Option<std::path::PathBuf>,
    },

    /// List running ztx sessions
    Sessions,

    /// Generate editor integration (keybinding + task) for ztx
    Setup {
        #[command(subcommand)]
        target: SetupTarget,
    },
}

#[derive(Subcommand)]
pub enum SetupTarget {
    /// Merge a ztx task and keybinding into the Zed configuration
    Zed {
        /// Apply changes without asking for confirmation
        #[arg(long)]
        yes: bool,

        /// Show the changes that would be made without writing any files
        #[arg(long)]
        preview: bool,

        /// Where to write the Zed configuration
        #[arg(long, value_enum, default_value_t)]
        scope: SetupScope,
    },
}

/// Destination for `ztx setup zed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum SetupScope {
    /// User-global config in `~/.config/zed/`.
    #[default]
    Global,
    /// Project-local config in `<worktree>/.zed/`.
    Project,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_run_with_trailing_args() {
        let cli = Cli::try_parse_from(["ztx", "run", "--", "claude", "--continue"]).unwrap();
        let Command::Run { command, .. } = cli.command else {
            panic!("expected run subcommand");
        };
        assert_eq!(command, vec!["claude", "--continue"]);
    }

    #[test]
    fn parses_title_mode_and_adapter_flags() {
        let cli = Cli::try_parse_from([
            "ztx",
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
        let cli = Cli::try_parse_from(["ztx", "send", "--from-zed-env"]).unwrap();
        let Command::Send { from_zed_env, .. } = cli.command else {
            panic!("expected send subcommand");
        };
        assert!(from_zed_env);
    }

    #[test]
    fn run_requires_a_command() {
        assert!(Cli::try_parse_from(["ztx", "run"]).is_err());
    }

    #[test]
    fn parses_notify_from_hook() {
        let cli = Cli::try_parse_from(["ztx", "notify", "--from-hook"]).unwrap();
        let Command::Notify {
            from_hook,
            wake,
            transcript,
            ..
        } = cli.command
        else {
            panic!("expected notify subcommand");
        };
        assert!(from_hook);
        assert!(!wake);
        assert!(transcript.is_none());
    }

    #[test]
    fn parses_notify_wake_and_transcript() {
        let cli =
            Cli::try_parse_from(["ztx", "notify", "--wake", "--transcript", "/p/t.jsonl"]).unwrap();
        let Command::Notify {
            wake, transcript, ..
        } = cli.command
        else {
            panic!("expected notify subcommand");
        };
        assert!(wake);
        assert_eq!(transcript, Some(std::path::PathBuf::from("/p/t.jsonl")));
    }
}
