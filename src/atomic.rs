use std::fs;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use tempfile::Builder;

use crate::{Context, Result, RsomicsError};

/// Writes a named file transactionally without replacing the destination on failure.
pub fn write_atomic<T>(
    path: impl AsRef<Path>,
    operation: impl FnOnce(&mut fs::File) -> Result<T>,
) -> Result<T> {
    let path = path.as_ref();
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let permissions = match fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(RsomicsError::Io(io::Error::new(
                error.kind(),
                format!("reading output metadata {}: {error}", path.display()),
            )));
        }
    };

    let mut builder = Builder::new();
    builder.prefix(".rsomics-");
    #[cfg(unix)]
    if permissions.is_none() {
        builder.permissions(fs::Permissions::from_mode(0o666));
    }
    if let Some(existing) = permissions.as_ref() {
        builder.permissions(existing.clone());
    }
    let mut temporary = builder
        .tempfile_in(parent)
        .rs_with_context(|| format!("creating temporary output beside {}", path.display()))?;
    if let Some(existing) = permissions {
        temporary
            .as_file()
            .set_permissions(existing)
            .rs_with_context(|| format!("preserving output permissions {}", path.display()))?;
    }

    let result = operation(temporary.as_file_mut())?;
    temporary
        .as_file_mut()
        .flush()
        .rs_with_context(|| format!("flushing output {}", path.display()))?;
    temporary
        .as_file_mut()
        .sync_all()
        .rs_with_context(|| format!("syncing output {}", path.display()))?;
    temporary.persist(path).map_err(|error| {
        RsomicsError::Io(io::Error::new(
            error.error.kind(),
            format!("committing output {}: {}", path.display(), error.error),
        ))
    })?;

    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .rs_with_context(|| format!("syncing output directory {}", parent.display()))?;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_replaces_the_destination() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("result.txt");
        fs::write(&path, b"old\n").unwrap();

        let value = write_atomic(&path, |output| {
            output.write_all(b"new\n").map_err(RsomicsError::Io)?;
            Ok(7)
        })
        .unwrap();

        assert_eq!(value, 7);
        assert_eq!(fs::read(path).unwrap(), b"new\n");
    }

    #[test]
    fn operation_failure_keeps_the_destination() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("result.txt");
        fs::write(&path, b"old\n").unwrap();

        let error = write_atomic(&path, |output| {
            output.write_all(b"partial\n").map_err(RsomicsError::Io)?;
            Err::<(), _>(RsomicsError::InvalidInput("rejected".to_owned()))
        })
        .unwrap_err();

        assert!(matches!(error, RsomicsError::InvalidInput(_)));
        assert_eq!(fs::read(&path).unwrap(), b"old\n");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn replacement_preserves_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("result.txt");
        fs::write(&path, b"old\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        write_atomic(&path, |output| {
            output.write_all(b"new\n").map_err(RsomicsError::Io)
        })
        .unwrap();

        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
}
