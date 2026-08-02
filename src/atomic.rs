use std::fs;
use std::io::{self, BufWriter, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tempfile::{Builder, NamedTempFile};

use crate::{Context, Result, RsomicsError};

const OUTPUT_BUFFER_SIZE: usize = 1024 * 1024;

/// Writes a named file transactionally without replacing the destination on failure.
pub fn write_atomic<T>(
    path: impl AsRef<Path>,
    operation: impl FnOnce(&mut fs::File) -> Result<T>,
) -> Result<T> {
    let path = path.as_ref();
    let mut staged = Staged::new(path)?;
    let result = operation(staged.file())?;
    staged.prepare()?;
    staged.commit()?;
    Ok(result)
}

/// Writes two named files as one recoverable transaction.
pub fn write_atomic_pair<T>(
    first: impl AsRef<Path>,
    second: impl AsRef<Path>,
    operation: impl FnOnce(&mut fs::File, &mut fs::File) -> Result<T>,
) -> Result<T> {
    let first = first.as_ref();
    let second = second.as_ref();
    reject_pair_alias(first, second)?;
    let mut first_staged = Staged::new(first)?;
    let mut second_staged = Staged::new(second)?;
    let first_backup = Backup::new(first)?;
    let second_backup = Backup::new(second)?;
    let result = operation(first_staged.file(), second_staged.file())?;
    first_staged.prepare()?;
    second_staged.prepare()?;
    if let Err(error) = first_staged.commit() {
        return Err(first_backup.restore(first, error));
    }
    if let Err(error) = second_staged.commit() {
        let error = first_backup.restore(first, error);
        return Err(second_backup.restore(second, error));
    }
    Ok(result)
}

/// Writes to standard output or commits a named output file transactionally.
pub fn write_output<T>(
    path: Option<&Path>,
    operation: impl FnOnce(&mut dyn Write) -> Result<T>,
) -> Result<T> {
    let mut stdout = io::stdout().lock();
    write_output_to(path, &mut stdout, operation)
}

fn write_output_to<T>(
    path: Option<&Path>,
    stdout: &mut dyn Write,
    operation: impl FnOnce(&mut dyn Write) -> Result<T>,
) -> Result<T> {
    match path {
        None => write_buffered(stdout, operation),
        Some(path) if path == Path::new("-") => write_buffered(stdout, operation),
        Some(path) => write_atomic(path, |output| write_buffered(output, operation)),
    }
}

fn write_buffered<T>(
    output: &mut dyn Write,
    operation: impl FnOnce(&mut dyn Write) -> Result<T>,
) -> Result<T> {
    let mut output = BufWriter::with_capacity(OUTPUT_BUFFER_SIZE, output);
    let result = operation(&mut output)?;
    output.flush().map_err(RsomicsError::Io)?;
    Ok(result)
}

struct Staged {
    path: PathBuf,
    parent: PathBuf,
    temporary: NamedTempFile,
}

impl Staged {
    fn new(path: &Path) -> Result<Self> {
        let parent = parent(path);
        let permissions = match fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => Some(metadata.permissions()),
            Ok(_) => {
                return Err(RsomicsError::InvalidInput(format!(
                    "output {} is not a regular file",
                    path.display()
                )));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(RsomicsError::Io(error)),
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
        let temporary = builder
            .tempfile_in(parent)
            .rs_with_context(|| format!("creating temporary output beside {}", path.display()))?;
        Ok(Self {
            path: path.to_owned(),
            parent: parent.to_owned(),
            temporary,
        })
    }

    fn file(&mut self) -> &mut fs::File {
        self.temporary.as_file_mut()
    }

    fn prepare(&mut self) -> Result<()> {
        self.file()
            .flush()
            .rs_with_context(|| format!("flushing output {}", self.path.display()))?;
        self.file()
            .sync_all()
            .rs_with_context(|| format!("syncing output {}", self.path.display()))
    }

    fn commit(self) -> Result<()> {
        self.temporary.persist(&self.path).map_err(|error| {
            RsomicsError::Io(io::Error::new(
                error.error.kind(),
                format!("committing output {}: {}", self.path.display(), error.error),
            ))
        })?;
        #[cfg(unix)]
        fs::File::open(&self.parent)
            .and_then(|directory| directory.sync_all())
            .rs_with_context(|| format!("syncing output directory {}", self.parent.display()))?;
        Ok(())
    }
}

enum Backup {
    Absent,
    Existing(NamedTempFile),
}

impl Backup {
    fn new(path: &Path) -> Result<Self> {
        match fs::metadata(path) {
            Ok(metadata) if !metadata.is_file() => Err(RsomicsError::InvalidInput(format!(
                "output {} is not a regular file",
                path.display()
            ))),
            Ok(_) => Builder::new()
                .prefix(".rsomics-backup-")
                .make_in(parent(path), |backup| {
                    fs::hard_link(path, backup)?;
                    fs::File::open(backup)
                })
                .map(Self::Existing)
                .rs_with_context(|| format!("backing up output {}", path.display())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::Absent),
            Err(error) => Err(RsomicsError::Io(error)),
        }
    }

    fn restore(self, path: &Path, cause: RsomicsError) -> RsomicsError {
        let restored = match self {
            Self::Absent => match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            },
            Self::Existing(backup) => backup
                .persist(path)
                .map(|_| ())
                .map_err(|error| error.error),
        };
        match restored {
            Ok(()) => cause,
            Err(error) => RsomicsError::Io(io::Error::new(
                error.kind(),
                format!(
                    "{cause}; also failed to restore output {}: {error}",
                    path.display()
                ),
            )),
        }
    }
}

fn reject_pair_alias(first: &Path, second: &Path) -> Result<()> {
    if first == Path::new("-") || second == Path::new("-") {
        return Err(RsomicsError::ConfigError(
            "paired transaction outputs must be named files".to_owned(),
        ));
    }
    let alias = if first == second {
        true
    } else {
        match same_file::is_same_file(first, second) {
            Ok(alias) => alias,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                prospective_path(first)? == prospective_path(second)?
            }
            Err(error) => return Err(RsomicsError::Io(error)),
        }
    };
    if alias {
        return Err(RsomicsError::ConfigError(format!(
            "paired outputs resolve to the same path: {}",
            first.display()
        )));
    }
    Ok(())
}

fn prospective_path(path: &Path) -> Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| {
        RsomicsError::ConfigError(format!("output path has no file name: {}", path.display()))
    })?;
    fs::canonicalize(parent(path))
        .map(|parent| parent.join(name))
        .rs_with_context(|| format!("resolving output parent for {}", path.display()))
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct CountingWriter {
        bytes: Vec<u8>,
        fail_flush: bool,
        writes: usize,
    }

    impl Write for CountingWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            if self.fail_flush {
                Err(io::Error::other("flush failed"))
            } else {
                Ok(())
            }
        }
    }

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

    #[test]
    fn output_target_selects_stream_or_transactional_file() {
        for path in [None, Some(Path::new("-"))] {
            let mut stdout = Vec::new();
            let value = write_output_to(path, &mut stdout, |output| {
                output.write_all(b"stream\n").map_err(RsomicsError::Io)?;
                Ok(7)
            })
            .unwrap();
            assert_eq!(value, 7);
            assert_eq!(stdout, b"stream\n");
        }

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("result.txt");
        let mut stdout = Vec::new();
        write_output_to(Some(&path), &mut stdout, |output| {
            output.write_all(b"file\n").map_err(RsomicsError::Io)
        })
        .unwrap();
        assert!(stdout.is_empty());
        assert_eq!(fs::read(path).unwrap(), b"file\n");
    }

    #[test]
    fn output_target_batches_small_writes() {
        let mut output = CountingWriter::default();
        write_output_to(None, &mut output, |writer| {
            for _ in 0..10_000 {
                writer.write_all(b"row\n").map_err(RsomicsError::Io)?;
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(output.writes, 1);
        assert_eq!(output.bytes.len(), 40_000);
    }

    #[test]
    fn output_target_propagates_buffer_flush_failure() {
        let mut output = CountingWriter {
            fail_flush: true,
            ..Default::default()
        };
        let error = write_output_to(None, &mut output, |writer| {
            writer.write_all(b"row\n").map_err(RsomicsError::Io)
        })
        .unwrap_err();

        assert!(matches!(error, RsomicsError::Io(_)));
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

    #[test]
    fn pair_failure_keeps_both_destinations() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("counts.tsv");
        let second = directory.path().join("counts.tsv.summary");
        fs::write(&first, b"old counts\n").unwrap();
        fs::write(&second, b"old summary\n").unwrap();

        let error = write_atomic_pair(&first, &second, |counts, summary| {
            counts
                .write_all(b"new counts\n")
                .map_err(RsomicsError::Io)?;
            summary
                .write_all(b"new summary\n")
                .map_err(RsomicsError::Io)?;
            Err::<(), _>(RsomicsError::InvalidInput("rejected".to_owned()))
        })
        .unwrap_err();

        assert!(matches!(error, RsomicsError::InvalidInput(_)));
        assert_eq!(fs::read(&first).unwrap(), b"old counts\n");
        assert_eq!(fs::read(&second).unwrap(), b"old summary\n");
    }

    #[test]
    fn second_pair_commit_failure_restores_the_first() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("counts.tsv");
        let second = directory.path().join("counts.tsv.summary");
        fs::write(&first, b"old counts\n").unwrap();
        fs::write(&second, b"old summary\n").unwrap();

        let error = write_atomic_pair(&first, &second, |counts, summary| {
            counts
                .write_all(b"new counts\n")
                .map_err(RsomicsError::Io)?;
            summary
                .write_all(b"new summary\n")
                .map_err(RsomicsError::Io)?;
            fs::remove_file(&second).map_err(RsomicsError::Io)?;
            fs::create_dir(&second).map_err(RsomicsError::Io)?;
            Ok(())
        })
        .unwrap_err();

        assert!(error.to_string().contains("committing output"));
        assert_eq!(fs::read(&first).unwrap(), b"old counts\n");
        assert!(second.is_dir());
    }

    #[test]
    fn pair_rejects_equivalent_destinations() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("counts.tsv");
        let second = directory.path().join(".").join("counts.tsv");
        let error = write_atomic_pair(first, second, |_, _| Ok(())).unwrap_err();
        assert!(error.to_string().contains("same path"));
    }
}
