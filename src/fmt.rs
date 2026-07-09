//! Numeric formatting helpers shared across rsomics tools.

/// Format a float like C's `printf("%g")` with 6 significant figures.
///
/// Fixed notation is used when the (post-rounding) decimal exponent falls in
/// `-4..6`, otherwise scientific notation with a two-digit exponent. Trailing
/// zeros — and a resulting bare `.` — are stripped. `nan`/`inf` render as C does.
///
/// The exponent is taken from the value already rounded to 6 significant
/// figures (via `{:.5e}`), so values that round up across a power of ten pick
/// the same branch C would (e.g. `999999.6` → `1e+06`, not `1000000`).
pub fn format_g6(x: f64) -> String {
    if x.is_nan() {
        return "nan".to_owned();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-inf" } else { "inf" }.to_owned();
    }
    if x == 0.0 {
        return "0".to_owned();
    }
    const PRECISION: i32 = 6;
    let sci = format!("{:.*e}", (PRECISION - 1) as usize, x);
    let (mantissa, exp_str) = sci.split_once('e').unwrap();
    let exp: i32 = exp_str.parse().unwrap();
    if (-4..PRECISION).contains(&exp) {
        let decimals = (PRECISION - 1 - exp).max(0) as usize;
        strip_trailing_zeros(format!("{x:.decimals$}"))
    } else {
        let sign = if exp < 0 { '-' } else { '+' };
        format!(
            "{}e{}{:02}",
            strip_trailing_zeros(mantissa.to_owned()),
            sign,
            exp.abs()
        )
    }
}

fn strip_trailing_zeros(mut s: String) -> String {
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::format_g6;

    #[test]
    fn matches_c_printf_g() {
        // (input, C `%.6g` output). Cases chosen to pin the exponent-boundary
        // and trailing-zero behaviour that earlier ad-hoc copies got wrong.
        let cases = [
            (1.0, "1"),
            (-1.0, "-1"),
            (123450.0, "123450"),
            (999999.6, "1e+06"),
            (100000.0, "100000"),
            (1_000_000.0, "1e+06"),
            (1234567.0, "1.23457e+06"),
            (0.0001, "0.0001"),
            (0.00009999, "9.999e-05"),
            (1e-5, "1e-05"),
            (1e-10, "1e-10"),
            (5e-7, "5e-07"),
            (1.2345678, "1.23457"),
            (99999.95, "99999.9"),
            (0.0, "0"),
        ];
        for (x, want) in cases {
            assert_eq!(format_g6(x), want, "format_g6({x})");
        }
    }
}
