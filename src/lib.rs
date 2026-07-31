//! Shared error, exit-code, and machine-output contracts for rsomics products.

#![forbid(unsafe_code)]

mod atomic;
mod error;
mod exit;
mod flags;
mod json;
mod runner;

pub use atomic::write_atomic;
pub use error::{Context, Result, RsomicsError};
pub use exit::ExitCode;
pub use flags::OutputArgs;
pub use json::ToolMeta;
pub use runner::run;
