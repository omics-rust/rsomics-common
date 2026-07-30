# rsomics-common

Layer-A foundation crate shared by every `rsomics-*` tool: canonical
`RsomicsError` + `Result`, process exit codes, `--threads / --json /
--quiet / --verbose / --seed` CLI flags, JSON envelope schema, stderr
log, path-or-standard-stream opening, and the `run()` entry-point wrapper.

`run()` propagates JSON serialization and standard-stream write/flush failures
to a non-zero process exit. Direct emitters should use
`json::try_emit_ok()` / `json::try_emit_error()` and propagate their `Result`.
The legacy `emit_ok()` / `emit_error()` wrappers are deprecated; they preserve
their original signatures but now panic rather than silently discarding an
output failure.

```rust
use clap::Parser;
use rsomics_common::{CommonFlags, Result, ToolMeta, run};

#[derive(Parser)]
struct Cli {
    #[arg(short = 'i', long)]
    input: std::path::PathBuf,
    #[command(flatten)]
    common: CommonFlags,
}

fn pipeline(args: Cli) -> Result<()> { /* … */ Ok(()) }

fn main() -> std::process::ExitCode {
    let args = Cli::parse();
    let common = args.common.clone();
    let meta = ToolMeta {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
    };
    run(&common, meta, || pipeline(args))
}
```

## Scope

Library-only crate. Promoted to Layer A under the workspace's
"2+ consumer" rule — anything in this crate is used by at least two
tool crates in `crates/tools/`. The full architecture rules live in the
workspace `CONVENTIONS.md`.

## Features

| Feature | Default | What it pulls in |
|---|---|---|
| `rayon` | yes | global thread-pool sizing in `CommonFlags`. Disable with `default-features = false` for single-thread FFI-wrapper tools. |
| `test-support` | no | `fixture_path!`, `tool_on_path`, `run_tool` helpers — integration / compat tests only. |
| `tier2` | no | implies `test-support`; adds `toml`/`sha2`/`ureq` for on-demand download + sha256-verify of Tier-2 fixtures. |

## External deps (4-quadrant classification)

- `thiserror`, `clap`, `serde`, `serde_json` — Quadrant ④ (pure-Rust edge utilities).
- `flate2` with `zlib-rs` backend — Quadrant ① (pure Rust + native parallelism via SIMD).
- `rayon` (optional) — Quadrant ① (pure Rust + explicit parallelism).
- `toml`, `sha2`, `ureq` (optional, `tier2`) — Quadrant ④.

License: MIT OR Apache-2.0.
