use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::{Result, RsomicsError};

/// Rejects a named output that resolves to any named input.
pub fn reject_output_alias<'a>(
    output: &Path,
    inputs: impl IntoIterator<Item = &'a Path>,
) -> Result<()> {
    if output == Path::new("-") {
        return Ok(());
    }
    for input in inputs.into_iter().filter(|input| *input != Path::new("-")) {
        if paths_alias(input, output)? {
            return Err(RsomicsError::ConfigError(format!(
                "output {} is also an input path",
                output.display()
            )));
        }
    }
    Ok(())
}

fn paths_alias(left: &Path, right: &Path) -> Result<bool> {
    if left == right {
        return Ok(true);
    }
    match same_file::is_same_file(left, right) {
        Ok(true) => return Ok(true),
        Ok(false) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(RsomicsError::Io(io::Error::new(
                error.kind(),
                format!(
                    "comparing input {} with output {}: {error}",
                    left.display(),
                    right.display()
                ),
            )));
        }
    }

    let left = canonicalize_if_exists(left, "input")?;
    let right = canonicalize_if_exists(right, "output")?;
    Ok(matches!((left, right), (Some(left), Some(right)) if left == right))
}

fn canonicalize_if_exists(path: &Path, role: &str) -> Result<Option<PathBuf>> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(Some(path)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(RsomicsError::Io(io::Error::new(
            error.kind(),
            format!("canonicalizing {role} {}: {error}", path.display()),
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_output_never_aliases_a_file() {
        reject_output_alias(Path::new("-"), [Path::new("-")]).unwrap();
        reject_output_alias(Path::new("-"), [Path::new("input.fa")]).unwrap();
    }

    #[test]
    fn exact_path_is_rejected() {
        let error =
            reject_output_alias(Path::new("input.fa"), [Path::new("input.fa")]).unwrap_err();
        assert!(error.to_string().contains("also an input path"), "{error}");
    }

    #[test]
    fn absent_output_is_distinct() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("input.fa");
        fs::write(&input, b">record\nACGT\n").unwrap();
        let output = directory.path().join("output.fa");

        reject_output_alias(&output, [input.as_path()]).unwrap();
    }

    #[test]
    fn normalized_path_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("input.fa");
        fs::write(&input, b">record\nACGT\n").unwrap();
        let output = directory.path().join(".").join("input.fa");

        let error = reject_output_alias(&output, [input.as_path()]).unwrap_err();
        assert!(error.to_string().contains("also an input path"), "{error}");
    }

    #[test]
    fn hard_link_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("input.fa");
        let output = directory.path().join("output.fa");
        fs::write(&input, b">record\nACGT\n").unwrap();
        fs::hard_link(&input, &output).unwrap();

        let error = reject_output_alias(&output, [input.as_path()]).unwrap_err();
        assert!(error.to_string().contains("also an input path"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_link_is_rejected() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("input.fa");
        let output = directory.path().join("output.fa");
        fs::write(&input, b">record\nACGT\n").unwrap();
        symlink(&input, &output).unwrap();

        let error = reject_output_alias(&output, [input.as_path()]).unwrap_err();
        assert!(error.to_string().contains("also an input path"), "{error}");
    }
}
