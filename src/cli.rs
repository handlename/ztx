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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_run_with_trailing_args() {
        let cli = Cli::try_parse_from(["zediator", "run", "--", "claude", "--continue"]).unwrap();
        let Command::Run { command, .. } = cli.command;
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
        } = cli.command;
        assert_eq!(title_mode, Some(crate::title::TitleMode::Managed));
        assert_eq!(adapter, crate::adapter::AdapterKind::Claude);
    }

    #[test]
    fn run_requires_a_command() {
        assert!(Cli::try_parse_from(["zediator", "run"]).is_err());
    }
}
