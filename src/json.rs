use std::io::{self, Write};

use serde::Serialize;

use crate::error::{Result, RsomicsError};
use crate::exit::ExitCode;

pub const SCHEMA_VERSION: &str = "1.0";

#[derive(Debug, Clone, Copy)]
pub struct ToolMeta {
    pub name: &'static str,
    pub version: &'static str,
}

#[derive(Serialize)]
struct OkEnvelope<'a, T: Serialize> {
    schema_version: &'static str,
    tool: &'a str,
    tool_version: &'a str,
    status: &'static str,
    result: &'a T,
}

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    schema_version: &'static str,
    tool: &'a str,
    tool_version: &'a str,
    status: &'static str,
    error: ErrorBody<'a>,
    exit_code: u8,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    kind: &'static str,
    message: &'a str,
}

/// Emits a successful JSON envelope and propagates serialization and I/O errors.
pub fn try_emit_ok<T: Serialize>(meta: &ToolMeta, result: &T) -> Result<()> {
    let stdout = io::stdout();
    write_ok_to(stdout.lock(), meta, result)
}

/// Emits an error JSON envelope and propagates serialization and I/O errors.
pub fn try_emit_error(meta: &ToolMeta, err: &RsomicsError) -> Result<()> {
    let stderr = io::stderr();
    write_error_to(stderr.lock(), meta, err)
}

/// Legacy infallible wrapper.
///
/// Use [`try_emit_ok`] in new code. This wrapper now fails loudly instead of
/// discarding output errors, but cannot return them without changing its
/// established signature.
#[deprecated(since = "0.6.4", note = "use try_emit_ok and propagate its Result")]
pub fn emit_ok<T: Serialize>(meta: &ToolMeta, result: &T) {
    try_emit_ok(meta, result).expect("failed to emit JSON success envelope");
}

/// Legacy infallible wrapper.
///
/// Use [`try_emit_error`] in new code. This wrapper now fails loudly instead of
/// discarding output errors, but cannot return them without changing its
/// established signature.
#[deprecated(since = "0.6.4", note = "use try_emit_error and propagate its Result")]
pub fn emit_error(meta: &ToolMeta, err: &RsomicsError) {
    try_emit_error(meta, err).expect("failed to emit JSON error envelope");
}

fn write_ok_to<W: Write, T: Serialize>(mut writer: W, meta: &ToolMeta, result: &T) -> Result<()> {
    let env = OkEnvelope {
        schema_version: SCHEMA_VERSION,
        tool: meta.name,
        tool_version: meta.version,
        status: "ok",
        result,
    };
    write_json_line(&mut writer, &env)
}

fn write_error_to<W: Write>(mut writer: W, meta: &ToolMeta, err: &RsomicsError) -> Result<()> {
    let kind = match err {
        RsomicsError::Io(_) => "Io",
        RsomicsError::InvalidInput(_) => "InvalidInput",
        RsomicsError::ConfigError(_) => "ConfigError",
        RsomicsError::UpstreamError(_) => "UpstreamError",
    };
    let message = err.to_string();
    let exit_code = ExitCode::from(err) as u8;
    let env = ErrorEnvelope {
        schema_version: SCHEMA_VERSION,
        tool: meta.name,
        tool_version: meta.version,
        status: "error",
        error: ErrorBody {
            kind,
            message: &message,
        },
        exit_code,
    };
    write_json_line(&mut writer, &env)
}

fn write_json_line<W: Write, T: Serialize>(writer: &mut W, value: &T) -> Result<()> {
    serde_json::to_writer(&mut *writer, value).map_err(json_error)?;
    writer.write_all(b"\n").map_err(RsomicsError::Io)?;
    writer.flush().map_err(RsomicsError::Io)
}

fn json_error(error: serde_json::Error) -> RsomicsError {
    let kind = error.io_error_kind().unwrap_or(io::ErrorKind::InvalidData);
    RsomicsError::Io(io::Error::new(kind, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::ser::Error as _;

    #[derive(Serialize)]
    struct Summary {
        total: u64,
        passed: u64,
    }

    struct FailingSerialize;

    impl Serialize for FailingSerialize {
        fn serialize<S>(&self, _serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(S::Error::custom("intentional serialization failure"))
        }
    }

    #[derive(Default)]
    struct ControlledWriter {
        bytes: Vec<u8>,
        fail_json: bool,
        fail_newline: bool,
        fail_flush: bool,
    }

    impl Write for ControlledWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.fail_json || (self.fail_newline && buf == b"\n") {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "intentional"));
            }
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            if self.fail_flush {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "intentional"))
            } else {
                Ok(())
            }
        }
    }

    fn meta() -> ToolMeta {
        ToolMeta {
            name: "rsomics-test",
            version: "0.0.0",
        }
    }

    #[test]
    fn ok_envelope_serializes_with_expected_keys() {
        let meta = ToolMeta {
            name: "rsomics-test",
            version: "0.0.0",
        };
        let s = Summary {
            total: 100,
            passed: 90,
        };
        let env = OkEnvelope {
            schema_version: SCHEMA_VERSION,
            tool: meta.name,
            tool_version: meta.version,
            status: "ok",
            result: &s,
        };
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&env).expect("ser")).expect("de");
        assert_eq!(v["schema_version"], "1.0");
        assert_eq!(v["tool"], "rsomics-test");
        assert_eq!(v["status"], "ok");
        assert_eq!(v["result"]["total"], 100);
        assert_eq!(v["result"]["passed"], 90);
    }

    #[test]
    fn error_envelope_has_kind_message_and_exit_code() {
        let meta = meta();
        let err = RsomicsError::InvalidInput("bad header".into());
        let exit = ExitCode::from(&err) as u8;
        let body = ErrorEnvelope {
            schema_version: SCHEMA_VERSION,
            tool: meta.name,
            tool_version: meta.version,
            status: "error",
            error: ErrorBody {
                kind: "InvalidInput",
                message: "invalid input: bad header",
            },
            exit_code: exit,
        };
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&body).expect("ser")).expect("de");
        assert_eq!(v["status"], "error");
        assert_eq!(v["error"]["kind"], "InvalidInput");
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap()
                .contains("bad header")
        );
        assert_eq!(v["exit_code"], exit);
    }

    #[test]
    fn successful_emission_writes_one_flushed_json_line() {
        let mut writer = ControlledWriter::default();
        write_ok_to(
            &mut writer,
            &meta(),
            &Summary {
                total: 1,
                passed: 1,
            },
        )
        .expect("write");
        assert!(writer.bytes.ends_with(b"\n"));
        let value: serde_json::Value =
            serde_json::from_slice(&writer.bytes).expect("valid JSON line");
        assert_eq!(value["status"], "ok");
    }

    #[test]
    fn serialization_failure_is_propagated() {
        let error = write_ok_to(Vec::new(), &meta(), &FailingSerialize).unwrap_err();
        let RsomicsError::Io(ref io_error) = error else {
            panic!("expected I/O-compatible serialization error");
        };
        assert_eq!(io_error.kind(), io::ErrorKind::InvalidData);
        assert!(
            error
                .to_string()
                .contains("intentional serialization failure")
        );
    }

    #[test]
    fn json_writer_failure_is_propagated() {
        let writer = ControlledWriter {
            fail_json: true,
            ..ControlledWriter::default()
        };
        let error = write_ok_to(
            writer,
            &meta(),
            &Summary {
                total: 1,
                passed: 1,
            },
        )
        .unwrap_err();
        assert!(matches!(error, RsomicsError::Io(_)));
    }

    #[test]
    fn newline_failure_is_propagated() {
        let writer = ControlledWriter {
            fail_newline: true,
            ..ControlledWriter::default()
        };
        let error = write_ok_to(
            writer,
            &meta(),
            &Summary {
                total: 1,
                passed: 1,
            },
        )
        .unwrap_err();
        assert!(matches!(error, RsomicsError::Io(_)));
    }

    #[test]
    fn flush_failure_is_propagated() {
        let writer = ControlledWriter {
            fail_flush: true,
            ..ControlledWriter::default()
        };
        let error = write_ok_to(
            writer,
            &meta(),
            &Summary {
                total: 1,
                passed: 1,
            },
        )
        .unwrap_err();
        assert!(matches!(error, RsomicsError::Io(_)));
    }

    #[test]
    fn error_emission_propagates_writer_failure() {
        let writer = ControlledWriter {
            fail_json: true,
            ..ControlledWriter::default()
        };
        let error =
            write_error_to(writer, &meta(), &RsomicsError::InvalidInput("bad".into())).unwrap_err();
        assert!(matches!(error, RsomicsError::Io(_)));
    }
}
