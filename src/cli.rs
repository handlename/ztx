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
        let Command::Run { command } = cli.command;
        assert_eq!(command, vec!["claude", "--continue"]);
    }

    #[test]
    fn run_requires_a_command() {
        assert!(Cli::try_parse_from(["zediator", "run"]).is_err());
    }
}
