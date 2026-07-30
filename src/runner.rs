use std::process;

use serde::Serialize;

use crate::error::Result;
use crate::exit::ExitCode;
use crate::flags::CommonFlags;
use crate::json::{ToolMeta, try_emit_error, try_emit_ok};
use crate::log::StderrLog;

pub fn run<T, F>(common: &CommonFlags, meta: ToolMeta, body: F) -> process::ExitCode
where
    F: FnOnce() -> Result<T>,
    T: Serialize,
{
    run_with_emitters(common, meta, body, try_emit_ok, try_emit_error)
}

fn run_with_emitters<T, F, O, E>(
    common: &CommonFlags,
    meta: ToolMeta,
    body: F,
    mut emit_ok: O,
    mut emit_error: E,
) -> process::ExitCode
where
    F: FnOnce() -> Result<T>,
    T: Serialize,
    O: FnMut(&ToolMeta, &T) -> Result<()>,
    E: FnMut(&ToolMeta, &crate::error::RsomicsError) -> Result<()>,
{
    let log = StderrLog::from_flags(common);

    if let Err(e) = common.install_rayon_pool() {
        if common.json
            && let Err(output_error) = emit_error(&meta, &e)
        {
            log.error(format_args!("{e}"));
            log.error(format_args!(
                "failed to emit JSON error envelope: {output_error}"
            ));
            return ExitCode::from(&output_error).into();
        }
        log.error(format_args!("{e}"));
        return ExitCode::from(&e).into();
    }

    match body() {
        Ok(result) => {
            if common.json
                && let Err(e) = emit_ok(&meta, &result)
            {
                log.error(format_args!("failed to emit JSON success envelope: {e}"));
                return ExitCode::from(&e).into();
            }
            ExitCode::Ok.into()
        }
        Err(e) => {
            if common.json
                && let Err(output_error) = emit_error(&meta, &e)
            {
                log.error(format_args!("{e}"));
                log.error(format_args!(
                    "failed to emit JSON error envelope: {output_error}"
                ));
                return ExitCode::from(&output_error).into();
            }
            log.error(format_args!("{e}"));
            ExitCode::from(&e).into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::RsomicsError;
    use clap::Parser;

    const META: ToolMeta = ToolMeta {
        name: "rsomics-runner-test",
        version: "0.0.0",
    };

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        common: CommonFlags,
    }

    fn cli() -> CommonFlags {
        Cli::parse_from(["test", "--threads", "2"]).common
    }

    fn json_cli() -> CommonFlags {
        Cli::parse_from(["test", "--threads", "2", "--json"]).common
    }

    #[test]
    fn ok_body_exits_zero() {
        let common = cli();
        let code = run(&common, META, || Ok::<_, RsomicsError>(()));
        let expected: process::ExitCode = ExitCode::Ok.into();
        assert_eq!(format!("{code:?}"), format!("{expected:?}"));
    }

    #[test]
    fn invalid_input_body_maps_to_exit_one() {
        let common = cli();
        let code = run(&common, META, || -> Result<()> {
            Err(RsomicsError::InvalidInput("bad".into()))
        });
        let expected: process::ExitCode = ExitCode::InvalidInput.into();
        assert_eq!(format!("{code:?}"), format!("{expected:?}"));
    }

    #[test]
    fn rayon_pool_double_install_is_tolerated() {
        let common = cli();
        let code = run(&common, META, || Ok::<_, RsomicsError>(()));
        let expected: process::ExitCode = ExitCode::Ok.into();
        assert_eq!(format!("{code:?}"), format!("{expected:?}"));
    }

    #[test]
    fn successful_body_with_failed_json_emission_exits_nonzero() {
        let common = json_cli();
        let code = run_with_emitters(
            &common,
            META,
            || Ok::<_, RsomicsError>(()),
            |_, _| Err(RsomicsError::Io(std::io::Error::other("broken stdout"))),
            |_, _| Ok(()),
        );
        let expected: process::ExitCode = ExitCode::IoError.into();
        assert_eq!(format!("{code:?}"), format!("{expected:?}"));
    }

    #[test]
    fn failed_error_envelope_emission_reports_output_exit_code() {
        let common = json_cli();
        let code = run_with_emitters(
            &common,
            META,
            || Err::<(), _>(RsomicsError::InvalidInput("bad".into())),
            |_, _| Ok(()),
            |_, _| Err(RsomicsError::Io(std::io::Error::other("broken stderr"))),
        );
        let expected: process::ExitCode = ExitCode::IoError.into();
        assert_eq!(format!("{code:?}"), format!("{expected:?}"));
    }
}
