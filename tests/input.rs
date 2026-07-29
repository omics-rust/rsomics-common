use std::io::Read;

use rsomics_common::{RsomicsError, open_path_or_stdin, read_path_or_stdin};

#[test]
fn opens_and_reads_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.tsv");
    std::fs::write(&path, b"a\tb\n1\t2\n").unwrap();

    let mut reader = open_path_or_stdin(&path).unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"a\tb\n1\t2\n");
    assert_eq!(read_path_or_stdin(&path).unwrap(), bytes);
}

#[test]
fn missing_file_error_includes_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.tsv");
    let io_error = std::fs::File::open(&path).unwrap_err();

    let error = read_path_or_stdin(&path).unwrap_err();
    match error {
        RsomicsError::InvalidInput(message) => {
            assert_eq!(message, format!("{}: {io_error}", path.display()));
        }
        other => panic!("unexpected error: {other}"),
    }
}
