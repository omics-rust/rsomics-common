# rsomics-common

`rsomics-common` is the narrow Layer-A contract shared by current rsomics
products:

- consistent machine-output and worker-count CLI controls;
- typed error categories and contextual propagation;
- stable process exit codes;
- versioned JSON success and error envelopes;
- static product identity;
- output/input alias rejection across equivalent paths and links;
- transactional single-file and paired-file output;
- runners that map ordinary results and structured validation reports to
  plain or JSON output.

The public CLI surface contains `OutputArgs` for `--json` and `ThreadArgs` for
`--threads`. `OutputArgs` is exercised by `rsomics-seq`,
`rsomics-fastq-preprocess`, and `rsomics-bed`. `ThreadArgs` has concrete
consumers in `rsomics-fastq-preprocess` and `rsomics-fastq-qc`; it standardizes
selection while each product continues to own its parallel runtime and work
partitioning.
`write_atomic` is the named-file commit contract exercised by `rsomics-bed`
and `rsomics-vcf`; it preserves existing permissions and never replaces the
destination when the producing operation fails. `write_output` adds the
shared product boundary: an omitted path or `-` streams to standard output,
while a named path uses that transactional commit contract. `rsomics-bed` and
`rsomics-call` are its concrete consumers.
`write_atomic_pair` stages two named outputs and restores the first if the
second cannot commit. `rsomics-liftover` and `rsomics-count` provide the two
consumer contracts; format-specific naming and serialization remain in those
products.
`reject_output_alias` is the fail-loud preflight shared by `rsomics-seq` and
`rsomics-bed`; it recognizes exact and normalized paths, hard links, and
symbolic links without hiding filesystem errors.
`run_validation` preserves a failed validator's report in the JSON error
envelope while returning the shared invalid-input exit code. Its concrete
consumers are `rsomics-vcf validate` and the planned `rsomics-bam validate`
contract.

Thread pools, work scheduling, RNG policy, logging verbosity, format I/O, and
fixture discovery are deliberately not part of this crate.

```rust
use clap::Parser;
use rsomics_common::{OutputArgs, Result, ThreadArgs, ToolMeta, run};

#[derive(Parser)]
struct Cli {
    #[command(flatten)]
    output: OutputArgs,
    #[command(flatten)]
    threads: ThreadArgs,
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
