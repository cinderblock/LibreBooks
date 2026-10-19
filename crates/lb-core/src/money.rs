//! Money: a signed whole number of minor units (cents for USD).
//!
//! Never a float. Amounts are what QuickBooks Desktop calls an `AMTTYPE`: two
//! decimal places in the home currency. Quantities, rates and prices, which may
//! carry more places, are [`rust_decimal::Decimal`]; turning one of those into
//! money rounds half away from zero, as QBD does.

use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};
use std::str::FromStr;

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Minor units per major unit. Every currency LibreBooks supports today has two
/// decimal places; multi-currency (Phase 8) will carry the exponent per currency.
pub const MINOR_PER_MAJOR: i64 = 100;

/// A signed amount of money in minor units. Serialised as a JSON integer.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS,
)]
#[serde(transparent)]
#[ts(export, type = "number")]
pub struct Money(pub i64);

/// Why text did not parse as money.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MoneyParseError {
    #[error("no amount given")]
    Empty,
    #[error("{0:?} is not an amount")]
    Invalid(String),
    #[error("{0:?} has more than two decimal places")]
    TooPrecise(String),
    #[error("{0:?} is too large")]
    Overflow(String),
}

impl Money {
    pub const ZERO: Money = Money(0);

    pub const fn from_minor(minor: i64) -> Self {
        Money(minor)
    }

    /// Whole major units, e.g. `Money::from_major(12)` is $12.00.
    pub const fn from_major(major: i64) -> Self {
        Money(major * MINOR_PER_MAJOR)
    }

    pub const fn minor(self) -> i64 {
        self.0
    }

    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    #[must_use]
    pub const fn abs(self) -> Self {
        Money(self.0.abs())
    }

    pub fn checked_add(self, other: Money) -> Option<Money> {
        self.0.checked_add(other.0).map(Money)
    }

    pub fn checked_sub(self, other: Money) -> Option<Money> {
        self.0.checked_sub(other.0).map(Money)
    }

    /// The exact decimal value in major units.
    pub fn to_decimal(self) -> Decimal {
        Decimal::new(self.0, 2)
    }

    /// Round a decimal amount of major units to money, half away from zero.
    /// `None` if it does not fit.
    pub fn from_decimal(value: Decimal) -> Option<Money> {
        let rounded = value.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
        (rounded * Decimal::from(MINOR_PER_MAJOR))
            .to_i64()
            .map(Money)
    }

    /// `quantity × rate`, rounded to money, as an invoice or bill line computes it.
    pub fn extend(quantity: Decimal, rate: Decimal) -> Option<Money> {
        quantity.checked_mul(rate).and_then(Money::from_decimal)
    }

    /// `self × percent / 100`, rounded: discounts and sales tax.
    pub fn percent(self, percent: Decimal) -> Option<Money> {
        self.to_decimal()
            .checked_mul(percent)
            .map(|v| v / Decimal::ONE_HUNDRED)
            .and_then(Money::from_decimal)
    }

    /// Split `self` across `weights` in proportion, so that the parts add up to
    /// `self` exactly (largest remainder; ties go to the earliest weight). Used for
    /// cash-basis allocation, discounts spread over lines and the like. All-zero
    /// weights split evenly. Empty weights give an empty result.
    pub fn allocate(self, weights: &[i64]) -> Vec<Money> {
        if weights.is_empty() {
            return Vec::new();
        }
        let total_weight: i128 = weights.iter().map(|w| i128::from(w.unsigned_abs())).sum();
        let (weights, total_weight): (Vec<i128>, i128) = if total_weight == 0 {
            (vec![1; weights.len()], weights.len() as i128)
        } else {
            (
                weights
                    .iter()
                    .map(|w| i128::from(w.unsigned_abs()))
                    .collect(),
                total_weight,
            )
        };
        let amount = i128::from(self.0);
        let mut parts: Vec<i128> = Vec::with_capacity(weights.len());
        let mut remainders: Vec<(usize, i128)> = Vec::with_capacity(weights.len());
        for (i, w) in weights.iter().enumerate() {
            let exact = amount * w;
            let part = exact / total_weight; // truncates toward zero
            parts.push(part);
            remainders.push((i, (exact - part * total_weight).abs()));
        }
        let mut left = amount - parts.iter().sum::<i128>();
        remainders.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let step = left.signum();
        for (i, _) in remainders {
            if left == 0 {
                break;
            }
            parts[i] += step;
            left -= step;
        }
        parts.into_iter().map(|p| Money(p as i64)).collect()
    }
}

impl Add for Money {
    type Output = Money;
    fn add(self, rhs: Money) -> Money {
        Money(self.0 + rhs.0)
    }
}

impl AddAssign for Money {
    fn add_assign(&mut self, rhs: Money) {
        self.0 += rhs.0;
    }
}

impl Sub for Money {
    type Output = Money;
    fn sub(self, rhs: Money) -> Money {
        Money(self.0 - rhs.0)
    }
}

impl SubAssign for Money {
    fn sub_assign(&mut self, rhs: Money) {
        self.0 -= rhs.0;
    }
}

impl Neg for Money {
    type Output = Money;
    fn neg(self) -> Money {
        Money(-self.0)
    }
}

impl Sum for Money {
    fn sum<I: Iterator<Item = Money>>(iter: I) -> Money {
        iter.fold(Money::ZERO, Add::add)
    }
}

impl<'a> Sum<&'a Money> for Money {
    fn sum<I: Iterator<Item = &'a Money>>(iter: I) -> Money {
        iter.copied().sum()
    }
}

/// Plain `-1234.50`: no grouping, no symbol. For people, the UI formats.
impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        let per = MINOR_PER_MAJOR.unsigned_abs();
        write!(f, "{sign}{}.{:02}", abs / per, abs % per)
    }
}

/// Accepts what people and QuickBooks write: `1234.5`, `1,234.50`, `$1,234.50`,
/// `-12`, `(12.00)` for a negative, `+3`, `.5`.
impl FromStr for Money {
    type Err = MoneyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let original = s;
        let mut s = s.trim();
        if s.is_empty() {
            return Err(MoneyParseError::Empty);
        }
        let mut negative = false;
        if s.starts_with('(') && s.ends_with(')') {
            negative = true;
            s = s[1..s.len() - 1].trim();
        }
        if let Some(rest) = s.strip_prefix('-') {
            negative = !negative;
            s = rest.trim_start();
        } else if let Some(rest) = s.strip_prefix('+') {
            s = rest.trim_start();
        }
        s = s.strip_prefix('$').unwrap_or(s).trim_start();
        // A sign may also follow the symbol: "$-5.00".
        if let Some(rest) = s.strip_prefix('-') {
            negative = !negative;
            s = rest;
        }
        let cleaned: String = s.chars().filter(|c| *c != ',' && *c != '_').collect();
        if cleaned.is_empty() {
            return Err(MoneyParseError::Invalid(original.to_owned()));
        }
        let (whole, frac) = match cleaned.split_once('.') {
            Some((w, f)) => (w, f),
            None => (cleaned.as_str(), ""),
        };
        if !whole.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) {
            return Err(MoneyParseError::Invalid(original.to_owned()));
        }
        if whole.is_empty() && frac.is_empty() {
            return Err(MoneyParseError::Invalid(original.to_owned()));
        }
        // Trailing zeros beyond two places are harmless ("1.500").
        let frac_trimmed = if frac.len() > 2 {
            if frac[2..].chars().any(|c| c != '0') {
                return Err(MoneyParseError::TooPrecise(original.to_owned()));
            }
            &frac[..2]
        } else {
            frac
        };
        let whole_val: i64 = if whole.is_empty() {
            0
        } else {
            whole
                .parse()
                .map_err(|_| MoneyParseError::Overflow(original.to_owned()))?
        };
        let frac_val: i64 = match frac_trimmed.len() {
            0 => 0,
            1 => frac_trimmed.parse::<i64>().unwrap_or(0) * 10,
            _ => frac_trimmed.parse().unwrap_or(0),
        };
        let minor = whole_val
            .checked_mul(MINOR_PER_MAJOR)
            .and_then(|v| v.checked_add(frac_val))
            .ok_or_else(|| MoneyParseError::Overflow(original.to_owned()))?;
        Ok(Money(if negative { -minor } else { minor }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rust_decimal_macros::dec;

    #[test]
    fn parses_what_people_type() {
        let cases = [
            ("0", 0),
            ("12", 1200),
            ("12.3", 1230),
            ("12.34", 1234),
            (".5", 50),
            ("1,234.56", 123_456),
            ("$1,234.56", 123_456),
            ("-12.00", -1200),
            ("(12.00)", -1200),
            ("($5)", -500),
            ("$-5", -500),
            ("+3", 300),
            ("27505.49", 2_750_549),
            ("1.500", 150),
            ("  7.10 ", 710),
        ];
        for (text, minor) in cases {
            assert_eq!(text.parse::<Money>(), Ok(Money(minor)), "{text}");
        }
    }

    #[test]
    fn rejects_nonsense() {
        assert_eq!("".parse::<Money>(), Err(MoneyParseError::Empty));
        assert!(matches!(
            "abc".parse::<Money>(),
            Err(MoneyParseError::Invalid(_))
        ));
        assert!(matches!(
            "1.234".parse::<Money>(),
            Err(MoneyParseError::TooPrecise(_))
        ));
        assert!(matches!(
            "1.2.3".parse::<Money>(),
            Err(MoneyParseError::Invalid(_))
        ));
        assert!(matches!(
            ".".parse::<Money>(),
            Err(MoneyParseError::Invalid(_))
        ));
        assert!(matches!(
            "99999999999999999999".parse::<Money>(),
            Err(MoneyParseError::Overflow(_))
        ));
    }

    #[test]
    fn displays_plainly() {
        assert_eq!(Money(0).to_string(), "0.00");
        assert_eq!(Money(5).to_string(), "0.05");
        assert_eq!(Money(-5).to_string(), "-0.05");
        assert_eq!(Money(123_456).to_string(), "1234.56");
        assert_eq!(Money(-123_456).to_string(), "-1234.56");
    }

    #[test]
    fn extends_and_rounds_half_away_from_zero() {
        assert_eq!(Money::extend(dec!(3), dec!(19.99)), Some(Money(5997)));
        assert_eq!(Money::extend(dec!(0.5), dec!(0.05)), Some(Money(3))); // 0.025 → 0.03
        assert_eq!(Money::extend(dec!(-0.5), dec!(0.05)), Some(Money(-3)));
        assert_eq!(Money::extend(dec!(1.333), dec!(100)), Some(Money(13330)));
        assert_eq!(Money(10000).percent(dec!(8.25)), Some(Money(825)));
        assert_eq!(Money(1999).percent(dec!(8.625)), Some(Money(172))); // 1.7241… → 1.72
    }

    #[test]
    fn allocates_exactly() {
        assert_eq!(
            Money(100).allocate(&[1, 1, 1]),
            vec![Money(34), Money(33), Money(33)]
        );
        assert_eq!(
            Money(-100).allocate(&[1, 1, 1]),
            vec![Money(-34), Money(-33), Money(-33)]
        );
        assert_eq!(Money(1000).allocate(&[0, 0]), vec![Money(500), Money(500)]);
        assert_eq!(Money(5).allocate(&[]), Vec::<Money>::new());
        assert_eq!(Money(10).allocate(&[3, 7]), vec![Money(3), Money(7)]);
    }

    proptest! {
        #[test]
        fn allocation_always_sums(total in -10_000_000i64..10_000_000, weights in prop::collection::vec(-1000i64..1000, 1..12)) {
            let parts = Money(total).allocate(&weights);
            prop_assert_eq!(parts.len(), weights.len());
            prop_assert_eq!(parts.iter().sum::<Money>(), Money(total));
        }

        #[test]
        fn display_round_trips(minor in -1_000_000_000_000i64..1_000_000_000_000) {
            let m = Money(minor);
            prop_assert_eq!(m.to_string().parse::<Money>(), Ok(m));
        }
    }
}
