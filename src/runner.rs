use std::io::{self, Write};
use std::process;

use serde::Serialize;

use crate::error::{Context, Result, RsomicsError};
use crate::exit::ExitCode;
use crate::flags::OutputArgs;
use crate::json::{ToolMeta, try_emit_error, try_emit_ok};

/// Maps a product result to plain or JSON output and a process exit code.
pub fn run<T, F>(output: &OutputArgs, meta: ToolMeta, body: F) -> process::ExitCode
where
    F: FnOnce() -> Result<T>,
    T: Serialize,
{
    run_with_emitters(
        output,
        meta,
        body,
        try_emit_ok,
        try_emit_error,
        try_emit_plain_error,
    )
}

fn run_with_emitters<T, F, O, E, P>(
    output: &OutputArgs,
    meta: ToolMeta,
    body: F,
    mut emit_ok: O,
    mut emit_error: E,
    mut emit_plain_error: P,
) -> process::ExitCode
where
    F: FnOnce() -> Result<T>,
    T: Serialize,
    O: FnMut(&ToolMeta, &T) -> Result<()>,
    E: FnMut(&ToolMeta, &crate::error::RsomicsError) -> Result<()>,
    P: FnMut(&crate::error::RsomicsError) -> Result<()>,
{
    match body() {
        Ok(result) => {
            if output.json
                && let Err(output_error) = emit_ok(&meta, &result)
            {
                return report_emission_failure(
                    &mut emit_plain_error,
                    output_error,
                    "emitting JSON success envelope",
                );
            }
            ExitCode::Ok.into()
        }
        Err(error) => {
            if output.json {
                if let Err(output_error) = emit_error(&meta, &error) {
                    return report_emission_failure(
                        &mut emit_plain_error,
                        output_error,
                        format!("emitting JSON error envelope for {error}"),
                    );
                }
                ExitCode::from(&error).into()
            } else {
                emit_plain_error(&error).map_or_else(
                    |output_error| ExitCode::from(&output_error).into(),
                    |()| ExitCode::from(&error).into(),
                )
            }
        }
    }
}

fn report_emission_failure<P>(
    emit_plain_error: &mut P,
    output_error: RsomicsError,
    context: impl Into<String>,
) -> process::ExitCode
where
    P: FnMut(&RsomicsError) -> Result<()>,
{
    let error = Err::<(), _>(output_error)
        .rs_context(context)
        .expect_err("constructed error remains an error");
    emit_plain_error(&error).map_or_else(
        |report_error| ExitCode::from(&report_error).into(),
        |()| ExitCode::from(&error).into(),
    )
}

fn try_emit_plain_error(error: &RsomicsError) -> Result<()> {
    write_plain_error(io::stderr().lock(), error)
}

fn write_plain_error(mut writer: impl Write, error: &RsomicsError) -> Result<()> {
    writeln!(writer, "error: {error}").map_err(RsomicsError::Io)?;
    writer.flush().map_err(RsomicsError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::RsomicsError;

    const META: ToolMeta = ToolMeta {
        name: "rsomics-runner-test",
        version: "0.0.0",
    };

    fn plain() -> OutputArgs {
        OutputArgs::default()
    }

    fn json() -> OutputArgs {
        OutputArgs { json: true }
    }

    #[test]
    fn ok_body_exits_zero() {
        let code = run(&plain(), META, || Ok::<_, RsomicsError>(()));
        let expected: process::ExitCode = ExitCode::Ok.into();
        assert_eq!(format!("{code:?}"), format!("{expected:?}"));
    }

    #[test]
    fn invalid_input_body_maps_to_exit_one() {
        let mut reported = None;
        let code = run_with_emitters(
            &plain(),
            META,
            || Err::<(), _>(RsomicsError::InvalidInput("bad".into())),
            |_, _| Ok(()),
            |_, _| Ok(()),
            |error| {
                reported = Some(error.to_string());
                Ok(())
            },
        );
        let expected: process::ExitCode = ExitCode::InvalidInput.into();
        assert_eq!(format!("{code:?}"), format!("{expected:?}"));
        assert_eq!(reported.as_deref(), Some("invalid input: bad"));
    }

    #[test]
    fn successful_body_with_failed_json_emission_exits_nonzero() {
        let mut reported = None;
        let code = run_with_emitters(
            &json(),
            META,
            || Ok::<_, RsomicsError>(()),
            |_, _| Err(RsomicsError::Io(std::io::Error::other("broken stdout"))),
            |_, _| Ok(()),
            |error| {
                reported = Some(error.to_string());
                Ok(())
            },
        );
        let expected: process::ExitCode = ExitCode::IoError.into();
        assert_eq!(format!("{code:?}"), format!("{expected:?}"));
        assert!(
            reported
                .as_deref()
                .is_some_and(|message| message.contains("emitting JSON success envelope"))
        );
    }

    #[test]
    fn failed_error_envelope_emission_reports_output_exit_code() {
        let mut reported = None;
        let code = run_with_emitters(
            &json(),
            META,
            || Err::<(), _>(RsomicsError::InvalidInput("bad".into())),
            |_, _| Ok(()),
            |_, _| Err(RsomicsError::Io(std::io::Error::other("broken stderr"))),
            |error| {
                reported = Some(error.to_string());
                Ok(())
            },
        );
        let expected: process::ExitCode = ExitCode::IoError.into();
        assert_eq!(format!("{code:?}"), format!("{expected:?}"));
        let reported = reported.unwrap();
        assert!(
            reported.contains("emitting JSON error envelope"),
            "{reported}"
        );
        assert!(reported.contains("invalid input: bad"), "{reported}");
        assert!(reported.contains("broken stderr"), "{reported}");
    }

    #[test]
    fn failed_plain_error_emission_reports_output_exit_code() {
        let code = run_with_emitters(
            &plain(),
            META,
            || Err::<(), _>(RsomicsError::InvalidInput("bad".into())),
            |_, _| Ok(()),
            |_, _| Ok(()),
            |_| Err(RsomicsError::Io(std::io::Error::other("broken stderr"))),
        );
        let expected: process::ExitCode = ExitCode::IoError.into();
        assert_eq!(format!("{code:?}"), format!("{expected:?}"));
    }

    #[test]
    fn plain_error_writer_flushes() {
        #[derive(Default)]
        struct Writer {
            bytes: Vec<u8>,
            flushed: bool,
        }

        impl Write for Writer {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                self.flushed = true;
                Ok(())
            }
        }

        let mut writer = Writer::default();
        write_plain_error(
            &mut writer,
            &RsomicsError::InvalidInput("bad header".into()),
        )
        .unwrap();
        assert_eq!(writer.bytes, b"error: invalid input: bad header\n");
        assert!(writer.flushed);
    }
}
