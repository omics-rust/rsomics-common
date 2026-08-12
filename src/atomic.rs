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
    let mut staged = AtomicFile::new(path)?;
    let result = operation(staged.file_mut())?;
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
    let mut first_staged = AtomicFile::new(first)?;
    let mut second_staged = AtomicFile::new(second)?;
    let result = operation(first_staged.file_mut(), second_staged.file_mut())?;
    AtomicFile::commit_all(vec![first_staged, second_staged])?;
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

/// A named output staged beside its destination until it is committed.
pub struct AtomicFile {
    path: PathBuf,
    parent: PathBuf,
    temporary: NamedTempFile,
    backup: Backup,
}

impl AtomicFile {
    /// Creates a staged output beside the destination.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
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
        let temporary = builder
            .tempfile_in(parent)
            .rs_with_context(|| format!("creating temporary output beside {}", path.display()))?;
        if let Some(existing) = permissions {
            temporary
                .as_file()
                .set_permissions(existing)
                .rs_with_context(|| {
                    format!("preserving permissions for output {}", path.display())
                })?;
        }
        let backup = Backup::new(path)?;
        Ok(Self {
            path: path.to_owned(),
            parent: parent.to_owned(),
            temporary,
            backup,
        })
    }

    /// Opens an owned handle to the staged output.
    pub fn reopen(&self) -> Result<fs::File> {
        self.temporary
            .reopen()
            .rs_with_context(|| format!("opening temporary output beside {}", self.path.display()))
    }

    /// Returns the staged output handle.
    pub fn file_mut(&mut self) -> &mut fs::File {
        self.temporary.as_file_mut()
    }

    /// Returns the temporary path used by the staged output.
    pub fn temporary_path(&self) -> &Path {
        self.temporary.path()
    }

    /// Flushes and atomically replaces the destination.
    pub fn commit(mut self) -> Result<()> {
        self.prepare()?;
        let path = self.path.clone();
        let backup = std::mem::replace(&mut self.backup, Backup::Absent);
        self.persist().map_err(|error| backup.restore(&path, error))
    }

    /// Commits multiple staged outputs as one recoverable transaction.
    pub fn commit_all(mut outputs: Vec<Self>) -> Result<()> {
        for (index, output) in outputs.iter().enumerate() {
            for previous in &outputs[..index] {
                reject_pair_alias(&previous.path, &output.path)?;
            }
        }
        for output in &mut outputs {
            output.prepare()?;
        }

        let paths = outputs
            .iter()
            .map(|output| output.path.clone())
            .collect::<Vec<_>>();
        let backups = outputs
            .iter_mut()
            .map(|output| std::mem::replace(&mut output.backup, Backup::Absent))
            .collect::<Vec<_>>();
        for (index, output) in outputs.into_iter().enumerate() {
            if let Err(error) = output.persist() {
                let mut restore = backups
                    .into_iter()
                    .zip(paths)
                    .take(index + 1)
                    .collect::<Vec<_>>();
                let mut error = error;
                while let Some((backup, path)) = restore.pop() {
                    error = backup.restore(&path, error);
                }
                return Err(error);
            }
        }
        Ok(())
    }

    fn prepare(&mut self) -> Result<()> {
        self.file_mut()
            .flush()
            .rs_with_context(|| format!("flushing output {}", self.path.display()))?;
        self.file_mut()
            .sync_all()
            .rs_with_context(|| format!("syncing output {}", self.path.display()))
    }

    fn persist(self) -> Result<()> {
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
    fn owned_atomic_file_is_invisible_until_commit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("result.txt");
        fs::write(&path, b"old\n").unwrap();

        let transaction = AtomicFile::new(&path).unwrap();
        let temporary = transaction.temporary_path().to_owned();
        let mut output = transaction.reopen().unwrap();
        output.write_all(b"new\n").unwrap();
        output.flush().unwrap();
        drop(output);

        assert_eq!(fs::read(&path).unwrap(), b"old\n");
        transaction.commit().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new\n");
        assert!(!temporary.exists());
    }

    #[test]
    fn owned_atomic_files_restore_prior_outputs_as_a_group() {
        let directory = tempfile::tempdir().unwrap();
        let paths =
            ["first.txt", "second.txt", "third.txt"].map(|name| directory.path().join(name));
        for path in &paths {
            fs::write(
                path,
                format!("old {}\n", path.file_name().unwrap().to_string_lossy()),
            )
            .unwrap();
        }

        let mut transactions = paths
            .iter()
            .map(|path| AtomicFile::new(path).unwrap())
            .collect::<Vec<_>>();
        for (index, transaction) in transactions.iter_mut().enumerate() {
            writeln!(transaction.file_mut(), "new {index}").unwrap();
        }
        fs::remove_file(&paths[1]).unwrap();
        fs::create_dir(&paths[1]).unwrap();

        let error = AtomicFile::commit_all(transactions).unwrap_err();
        assert!(error.to_string().contains("committing output"));
        assert_eq!(fs::read(&paths[0]).unwrap(), b"old first.txt\n");
        assert!(paths[1].is_dir());
        assert_eq!(fs::read(&paths[2]).unwrap(), b"old third.txt\n");
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
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();

        write_atomic(&path, |output| {
            output.write_all(b"new\n").map_err(RsomicsError::Io)
        })
        .unwrap();

        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o666
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
