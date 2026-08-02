//! Shared CLI, error, exit-code, and machine-output contracts for rsomics products.

#![forbid(unsafe_code)]

mod atomic;
mod error;
mod exit;
mod flags;
mod json;
mod path;
mod runner;
mod threads;

pub use atomic::{write_atomic, write_atomic_pair, write_output};
pub use error::{Context, Result, RsomicsError};
pub use exit::ExitCode;
pub use flags::OutputArgs;
pub use json::ToolMeta;
pub use path::reject_output_alias;
pub use runner::{Validation, run, run_validation};
pub use threads::ThreadArgs;
