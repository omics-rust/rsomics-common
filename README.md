# rsomics-common

`rsomics-common` is the narrow Layer-A contract shared by current rsomics
products:

- typed error categories and contextual propagation;
- stable process exit codes;
- versioned JSON success and error envelopes;
- static product identity;
- a runner that maps a product result to plain or JSON output.

The public CLI surface contains only `--json`, the one output control exercised
by `rsomics-seq`, `rsomics-fastq-preprocess`, and `rsomics-bed`.

Thread pools, RNG policy, logging verbosity, format I/O, transactional product
output, and fixture discovery are deliberately not part of this crate.
`rsomics-fastq-preprocess` currently owns its thread control because it is the
only verified product that uses it. A capability moves here only after a second
product establishes the same contract.

```rust
use clap::Parser;
use rsomics_common::{OutputArgs, Result, ToolMeta, run};

#[derive(Parser)]
struct Cli {
    #[command(flatten)]
    output: OutputArgs,
}

fn execute() -> Result<()> {
    Ok(())
}

fn main() -> std::process::ExitCode {
    let cli = rsomics_help::parse::<Cli>();
    run(
        &cli.output,
        ToolMeta {
            name: env!("CARGO_PKG_NAME"),
            version: env!("CARGO_PKG_VERSION"),
        },
        execute,
    )
}
```

License: MIT OR Apache-2.0.
