use std::fs::File;
use std::io::{Read, stdin};
use std::path::Path;

use crate::{Result, RsomicsError};

/// Opens a file path for reading, or locked standard input when `path` is `"-"`.
pub fn open_path_or_stdin(path: impl AsRef<Path>) -> Result<Box<dyn Read>> {
    let path = path.as_ref();
    if path.as_os_str() == "-" {
        Ok(Box::new(stdin().lock()))
    } else {
        File::open(path)
            .map(|file| Box::new(file) as Box<dyn Read>)
            .map_err(|error| RsomicsError::InvalidInput(format!("{}: {error}", path.display())))
    }
}

/// Reads a file path in full, or locked standard input when `path` is `"-"`.
pub fn read_path_or_stdin(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let mut reader = open_path_or_stdin(path)?;
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(RsomicsError::Io)?;
    Ok(bytes)
}
