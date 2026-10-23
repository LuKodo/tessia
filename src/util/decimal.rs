//! Helpers around [`rust_decimal::Decimal`], the type used for every amount.
//!
//! Money is stored as `NUMERIC` in Postgres and handled as `Decimal` in Rust;
//! never use floating point for amounts.

use std::sync::LazyLock;

use regex::Regex;
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

static MAX_AMOUNT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[0-9]{1,12}(\.[0-9]{1,2})?$").unwrap_or_else(|_| unreachable!("constant regex"))
});
static COMMA_AMOUNT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[0-9]+(\.[0-9]{3})*(,[0-9]{1,2})?$")
        .unwrap_or_else(|_| unreachable!("constant regex"))
});

/// Why an amount text was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AmountError {
    #[error(
        "monto máximo inválido: use pesos sin separador de miles y máximo dos decimales, p. ej. 100000"
    )]
    InvalidMaximum,
    #[error("el monto máximo debe ser positivo")]
    NotPositive,
    #[error("valor de convenio inválido: {0:?}")]
    InvalidAgreement(String),
}

/// Validates the per-invoice cap (Go `ParseMaxAmount`): up to 12 integer
/// digits, at most two decimals, strictly positive.
pub fn parse_max_amount(text: &str) -> Result<Decimal, AmountError> {
    if !MAX_AMOUNT_RE.is_match(text) {
        return Err(AmountError::InvalidMaximum);
    }
    let value = text
        .parse::<Decimal>()
        .map_err(|_| AmountError::InvalidMaximum)?;
    if value <= Decimal::ZERO {
        return Err(AmountError::NotPositive);
    }
    Ok(value)
}

/// Parses a convenio value (Go `ParseAgreementAmount`): with a comma it is
/// `1.234,56` (dot thousands), otherwise `1234.56`.
pub fn parse_agreement_amount(text: &str) -> Result<Decimal, AmountError> {
    let invalid = || AmountError::InvalidAgreement(text.to_owned());
    let mut v = text.trim().to_owned();
    if v.contains(',') {
        if !COMMA_AMOUNT_RE.is_match(&v) {
            return Err(invalid());
        }
        v = v.replace('.', "").replace(',', ".");
    }
    if !MAX_AMOUNT_RE.is_match(&v) {
        return Err(invalid());
    }
    v.parse::<Decimal>().map_err(|_| invalid())
}

/// Parses a plain decimal such as `"1234.5"` or `"-10"` (`.` as separator, no
/// thousands separators). Surrounding blanks are ignored.
pub fn parse_plain(text: &str) -> Result<Decimal, rust_decimal::Error> {
    text.trim().parse::<Decimal>()
}

/// Rounds half away from zero to two decimals, the precision of `NUMERIC(18,2)`.
pub fn round2(value: Decimal) -> Decimal {
    value.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
}

/// Prints `value` with exactly two decimals (`"10.50"`, `"0.00"`), like Go's
/// `whole + "." + (frac + "00")[:2]` rendering of the agreement totals.
pub fn format2(value: Decimal) -> String {
    format!("{:.2}", round2(value))
}

/// Sums amounts; an empty iterator is zero.
pub fn sum<I: IntoIterator<Item = Decimal>>(values: I) -> Decimal {
    values.into_iter().fold(Decimal::ZERO, |acc, v| acc + v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_two_decimals() {
        let value = parse_plain(" 1234.5 ").expect("valid decimal");
        assert_eq!(format2(value), "1234.50");
        assert_eq!(format2(Decimal::ZERO), "0.00");
    }

    #[test]
    fn rounds_half_away_from_zero() {
        assert_eq!(format2(parse_plain("0.005").expect("valid")), "0.01");
        assert_eq!(format2(parse_plain("-0.005").expect("valid")), "-0.01");
    }

    #[test]
    fn rejects_text_that_is_not_a_number() {
        assert!(parse_plain("12,5").is_err());
    }

    #[test]
    fn max_amount_follows_the_go_rules() {
        assert_eq!(
            parse_max_amount("100000").expect("ok"),
            Decimal::from(100000)
        );
        assert_eq!(parse_max_amount("0"), Err(AmountError::NotPositive));
        assert_eq!(parse_max_amount("1,5"), Err(AmountError::InvalidMaximum));
        assert_eq!(parse_max_amount("1.234"), Err(AmountError::InvalidMaximum));
        assert!(parse_max_amount("1234567890123").is_err());
    }

    #[test]
    fn agreement_amounts_accept_both_notations() {
        assert_eq!(
            format2(parse_agreement_amount("1.234,56").expect("comma")),
            "1234.56"
        );
        assert_eq!(
            format2(parse_agreement_amount("1234.5").expect("dot")),
            "1234.50"
        );
        assert!(parse_agreement_amount("1,234.56").is_err());
        assert!(parse_agreement_amount("abc").is_err());
    }

    #[test]
    fn sums_exactly() {
        let parts = ["0.1", "0.2"].map(|s| parse_plain(s).expect("valid"));
        assert_eq!(sum(parts), parse_plain("0.3").expect("valid"));
    }
}
