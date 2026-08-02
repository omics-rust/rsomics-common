use std::num::NonZeroUsize;

use clap::Args;

/// Shared worker-count selection for parallel rsomics products.
#[derive(Debug, Default, Clone, Args)]
#[command(next_help_heading = "Global options")]
pub struct ThreadArgs {
    /// Number of worker threads.
    #[arg(short = 't', long, global = true)]
    threads: Option<NonZeroUsize>,
}

impl ThreadArgs {
    #[must_use]
    pub const fn requested(&self) -> Option<NonZeroUsize> {
        self.threads
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{Parser, Subcommand};

    #[derive(Debug, Parser)]
    struct Cli {
        #[command(flatten)]
        threads: ThreadArgs,
        #[command(subcommand)]
        command: Command,
    }

    #[derive(Debug, Subcommand)]
    enum Command {
        Run,
    }

    #[test]
    fn defaults_to_runtime_selection() {
        let cli = Cli::parse_from(["test", "run"]);
        assert_eq!(cli.threads.requested(), None);
    }

    #[test]
    fn parses_global_short_and_long_forms() {
        let before = Cli::parse_from(["test", "-t", "2", "run"]);
        let after = Cli::parse_from(["test", "run", "--threads", "4"]);
        assert_eq!(before.threads.requested().map(NonZeroUsize::get), Some(2));
        assert_eq!(after.threads.requested().map(NonZeroUsize::get), Some(4));
    }

    #[test]
    fn rejects_zero() {
        assert!(Cli::try_parse_from(["test", "--threads", "0", "run"]).is_err());
    }
}
