//! Number handling of the preparation (port of Go `convenio/amount.go`).
//!
//! Two kinds of numbers coexist, exactly as in Go:
//!
//! * the per-invoice **cap and totals** are exact decimals
//!   ([`rust_decimal::Decimal`], parsed with `util::decimal`, which keeps the
//!   written scale like Java's `BigDecimal`, so `1000 + 2000.50` prints
//!   `3000.50`);
//! * the **unit-price check** uses the legacy lenient parser
//!   ([`parse_loose_number`], an `f64`) and prints with Java's
//!   `DecimalFormat("0.##")` ([`format_decimal2`], HALF_EVEN on the exact
//!   binary value of the `f64`), because the messages stored in
//!   `proceso_estado` were produced that way.

use rust_decimal::Decimal;

/// Legacy lenient number parser (`parseNumero`): `$` and spaces are removed;
/// with both separators `.` is thousands and `,` the decimal; a lone `,` is
/// the decimal. Unparseable (or non-finite) values are 0.
pub fn parse_loose_number(text: &str) -> f64 {
    let cleaned: String = text
        .trim()
        .chars()
        .filter(|c| *c != '$' && *c != ' ')
        .collect();
    if cleaned.is_empty() {
        return 0.0;
    }
    let normalized = match (cleaned.contains(','), cleaned.contains('.')) {
        (true, true) => cleaned.replace('.', "").replace(',', "."),
        (true, false) => cleaned.replace(',', "."),
        _ => cleaned,
    };
    match normalized.parse::<f64>() {
        Ok(value) if value.is_finite() => value,
        _ => 0.0,
    }
}

/// Java `DecimalFormat("0.##")`: at most two decimals, trailing zeros removed,
/// ties to even on the *exact binary value* of `value`.
///
/// ```
/// use tessia::phases::convenio_preparar::amount::format_decimal2;
/// assert_eq!(format_decimal2(0.125), "0.12");
/// assert_eq!(format_decimal2(10.005), "10.01"); // the double is slightly above
/// assert_eq!(format_decimal2(1.005), "1");      // the double is slightly below
/// ```
pub fn format_decimal2(value: f64) -> String {
    let negative = value.is_sign_negative();
    let bits = value.abs().to_bits();
    let exponent_bits = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    let (mantissa, exponent) = if exponent_bits == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1u64 << 52), exponent_bits - 1075)
    };
    // |value| * 100 = scaled * 2^exponent, with scaled < 2^60.
    let scaled = u128::from(mantissa) * 100;
    let whole = if exponent >= 0 {
        if exponent > 60 {
            return format!("{value:.0}");
        }
        scaled << exponent
    } else {
        round_half_even(scaled, exponent.unsigned_abs())
    };
    let text = format!("{}.{:02}", whole / 100, whole % 100);
    let text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
    if negative && text != "0" {
        format!("-{text}")
    } else {
        text
    }
}

/// `scaled / 2^shift` rounded to nearest, ties to even.
fn round_half_even(scaled: u128, shift: u32) -> u128 {
    if shift >= 128 {
        return 0;
    }
    let whole = scaled >> shift;
    let remainder = scaled & ((1u128 << shift) - 1);
    let half = 1u128 << (shift - 1);
    match remainder.cmp(&half) {
        std::cmp::Ordering::Greater => whole + 1,
        std::cmp::Ordering::Equal if whole % 2 == 1 => whole + 1,
        _ => whole,
    }
}

/// Plain text of an exact amount as stored by the preparation: at most two
/// decimals, ties to even, trailing zeros removed (`1500.00` -> `1500`).
pub fn price_text(value: Decimal) -> String {
    value
        .round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointNearestEven)
        .normalize()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::decimal::{parse_agreement_amount, parse_max_amount};

    #[test]
    fn loose_numbers_follow_the_legacy_parser() {
        for (input, expected) in [
            ("", 0.0),
            ("12345,67", 12345.67),
            ("$ 1.234,50", 1234.5),
            ("1234.5", 1234.5),
            ("abc", 0.0),
            ("7", 7.0),
            ("1e999", 0.0),
        ] {
            assert_eq!(parse_loose_number(input), expected, "{input}");
        }
    }

    #[test]
    fn decimal_format_matches_java_half_even_on_the_binary_value() {
        for (value, expected) in [
            (1234.5, "1234.5"),
            (1234.0, "1234"),
            (0.125, "0.12"),
            (0.135, "0.14"),
            (10.005, "10.01"),
            (1.005, "1"),
            (0.0, "0"),
            (-2.5, "-2.5"),
            (-0.001, "0"),
        ] {
            assert_eq!(format_decimal2(value), expected, "{value}");
        }
    }

    #[test]
    fn totals_keep_the_written_scale_like_big_decimal() {
        let a = parse_agreement_amount("1000").expect("a");
        let b = parse_agreement_amount("2000,5").expect("b");
        let c = parse_agreement_amount("0,25").expect("c");
        assert_eq!((a + b + c).to_string(), "3000.75");
        let d = parse_agreement_amount("2.000,50").expect("d");
        assert_eq!((a + d).to_string(), "3000.50");
        let max = parse_max_amount("3000.5").expect("max");
        assert_eq!(a + d, max, "compared by value, not by scale");
        assert_eq!(parse_max_amount("0100").expect("zeros").to_string(), "100");
    }

    #[test]
    fn price_text_strips_zeros_and_rounds_half_even() {
        let text = |s: &str| price_text(s.parse().expect("decimal"));
        assert_eq!(text("1500.00"), "1500");
        assert_eq!(text("1234.50"), "1234.5");
        assert_eq!(text("0.125"), "0.12");
        assert_eq!(text("0.135"), "0.14");
        assert_eq!(text("0"), "0");
    }
}
