use clap::Args;

/// Shared controls for the machine-readable command result.
#[derive(Debug, Default, Clone, Args)]
#[command(next_help_heading = "Global options")]
pub struct OutputArgs {
    /// Emit machine-readable JSON to stdout where applicable.
    #[arg(long = "json", global = true, default_value_t = false)]
    pub json: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser, Debug)]
    struct Cli {
        #[command(flatten)]
        output: OutputArgs,
    }

    #[test]
    fn defaults_when_no_flags() {
        let cli = Cli::parse_from(["test"]);
        assert!(!cli.output.json);
    }

    #[test]
    fn long_forms_parse_json_flag() {
        let cli = Cli::parse_from(["test", "--json"]);
        assert!(cli.output.json);
    }
}
