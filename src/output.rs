use std::fs::File;
use std::io::{Write, stdout};
use std::path::Path;

use crate::{Result, RsomicsError};

/// Opens a file path for writing, or locked standard output when `path` is `"-"`.
pub fn open_path_or_stdout(path: impl AsRef<Path>) -> Result<Box<dyn Write>> {
    let path = path.as_ref();
    if path.as_os_str() == "-" {
        Ok(Box::new(stdout().lock()))
    } else {
        File::create(path)
            .map(|file| Box::new(file) as Box<dyn Write>)
            .map_err(RsomicsError::Io)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn dash_opens_stdout() {
        drop(open_path_or_stdout("-").unwrap());
    }

    #[test]
    fn path_creates_and_truncates_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("output.tsv");
        std::fs::write(&path, b"old data").unwrap();

        let mut output = open_path_or_stdout(&path).unwrap();
        output.write_all(b"new").unwrap();
        drop(output);

        assert_eq!(std::fs::read(path).unwrap(), b"new");
    }

    #[test]
    fn invalid_parent_is_an_io_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("missing").join("output.tsv");

        let error = match open_path_or_stdout(path) {
            Ok(_) => panic!("missing parent must fail"),
            Err(error) => error,
        };
        assert!(matches!(error, RsomicsError::Io(_)));
    }
}
