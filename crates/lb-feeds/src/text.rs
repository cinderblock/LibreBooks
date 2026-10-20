//! Text from banks: decoding bytes, reading their dates and amounts, and cleaning
//! up payee names so that the same merchant compares equal from one download to
//! the next.
//!
//! Everything here is lenient on purpose. A bank file is not a contract; the
//! same bank sends different shapes from one year to the next, and the person
//! importing it can't fix it. So a parser accepts whatever it can understand and
//! reports what it had to skip, rather than refusing the whole file.

use jiff::civil::Date;
use lb_core::{Money, MoneyParseError};
use rust_decimal::Decimal;

/// Turn a file's bytes into text.
///
/// - A UTF-8 byte order mark is dropped; UTF-16 (either order) is honoured when
///   its byte order mark says so.
/// - Otherwise, bytes that are valid UTF-8 are read as UTF-8 whatever the file's
///   header claims: banks routinely declare `CHARSET:1252` and send UTF-8.
/// - Anything else is Windows-1252, which is what US banks actually mean when they
///   say ASCII, Latin-1 or 1252.
pub fn decode(bytes: &[u8]) -> String {
    if let Some((encoding, bom_len)) = encoding_rs::Encoding::for_bom(bytes) {
        let (text, _) = encoding.decode_without_bom_handling(&bytes[bom_len..]);
        return text.into_owned();
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        s.to_owned()
    } else {
        let (text, _) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes);
        text.into_owned()
    }
}

/// An amount as banks write it: `-12.34`, `+12.34`, `1,234.56`, `$1,234.56`,
/// `(12.34)`, `12.34-`, European `1.234,56` and `-12,34`, with stray spaces and
/// currency codes. Never a float: the result is exact or an error. More than two
/// decimal places round half away from zero (a few banks send `12.3400`, which
/// is exact, or `0.125` interest accruals, which are not).
pub fn parse_amount(s: &str) -> Result<Money, MoneyParseError> {
    let original = s;
    let mut t: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '$' | '€' | '£' | '\u{a0}' | '\''))
        .collect();
    for code in ["USD", "CAD", "EUR", "GBP"] {
        if let Some(rest) = t.strip_suffix(code) {
            t = rest.to_owned();
        } else if let Some(rest) = t.strip_prefix(code) {
            t = rest.to_owned();
        }
    }
    // A trailing minus: "12.34-".
    let mut negative = false;
    if let Some(rest) = t.strip_suffix('-')
        && !rest.is_empty()
    {
        negative = true;
        t = rest.to_owned();
    }
    let last_dot = t.rfind('.');
    let last_comma = t.rfind(',');
    let normalised = match (last_dot, last_comma) {
        // Both: whichever comes last is the decimal separator.
        (Some(d), Some(c)) if c > d => t.replace('.', "").replace(',', "."),
        (Some(_), Some(_)) => t.replace(',', ""),
        // Only commas: one comma followed by one or two digits is a decimal comma
        // ("-12,34"); otherwise they group thousands ("1,234").
        (None, Some(c)) => {
            let after = &t[c + 1..];
            if t.matches(',').count() == 1
                && (1..=2).contains(&after.len())
                && after.chars().all(|ch| ch.is_ascii_digit())
            {
                t.replace(',', ".")
            } else {
                t.replace(',', "")
            }
        }
        _ => t,
    };
    let money = match normalised.parse::<Money>() {
        Ok(m) => m,
        Err(MoneyParseError::TooPrecise(_)) => {
            let unsigned = normalised.trim_start_matches(['+', '-']);
            let mut d: Decimal = unsigned
                .parse()
                .map_err(|_| MoneyParseError::Invalid(original.to_owned()))?;
            if normalised.starts_with('-') {
                d = -d;
            }
            Money::from_decimal(d).ok_or_else(|| MoneyParseError::Overflow(original.to_owned()))?
        }
        Err(MoneyParseError::Empty) => return Err(MoneyParseError::Empty),
        Err(MoneyParseError::Invalid(_)) => {
            return Err(MoneyParseError::Invalid(original.to_owned()));
        }
        Err(e) => return Err(e),
    };
    Ok(if negative { -money } else { money })
}

/// An OFX date-time: `YYYYMMDD`, `YYYYMMDDHHMMSS`, `YYYYMMDDHHMMSS.XXX`, any of
/// them followed by a zone like `[-8:PST]` or `[+5.30:IST]`, and the ISO
/// `YYYY-MM-DD…` some exporters write instead.
///
/// Only the calendar date is kept, as the bank wrote it: the time and zone never
/// move a transaction to another day (see `plans/feeds.md`, decision 3).
pub fn parse_ofx_date(s: &str) -> Option<Date> {
    let s = s.trim();
    let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() >= 8 {
        return ymd(&digits[0..4], &digits[4..6], &digits[6..8]);
    }
    // ISO with dashes.
    let b = s.as_bytes();
    if b.len() >= 10 && b[4] == b'-' && b[7] == b'-' {
        return ymd(&s[0..4], &s[5..7], &s[8..10]);
    }
    None
}

fn ymd(y: &str, m: &str, d: &str) -> Option<Date> {
    Date::new(y.parse().ok()?, m.parse().ok()?, d.parse().ok()?).ok()
}

/// The order of day, month and year in a date written with separators.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS,
)]
#[ts(export)]
pub enum DateOrder {
    /// 2026-10-01, 2026/10/01, 20261001.
    Ymd,
    /// 10/01/2026, 10/1/26, Oct 1 2026 (US).
    #[default]
    Mdy,
    /// 01/10/2026, 1 Oct 2026 (most of the world).
    Dmy,
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// A date as a spreadsheet or a bank's CSV writes it, in a given order. Accepts
/// any separators, one- or two-digit days and months, two- or four-digit years
/// (two-digit years are 2000–2099), month names (`Oct 01, 2026`, `1-Oct-2026`),
/// weekday names, and a trailing time (`10/01/2026 13:45:00`), which is ignored.
pub fn parse_date(s: &str, order: DateOrder) -> Option<Date> {
    let s = s.trim().trim_matches('"');
    if s.is_empty() {
        return None;
    }
    // Compact 20261001 (optionally with a time after it).
    let lead: String = s.chars().take_while(char::is_ascii_digit).collect();
    if lead.len() == 8 && order == DateOrder::Ymd {
        return ymd(&lead[0..4], &lead[4..6], &lead[6..8]);
    }
    // Split into runs of letters and digits. A time after the date only adds
    // numbers after the first three, which are all that is read.
    let mut numbers: Vec<&str> = Vec::new();
    let mut month_name: Option<i8> = None;
    let mut current_start: Option<usize> = None;
    let mut tokens: Vec<&str> = Vec::new();
    for (i, ch) in s.char_indices() {
        if ch.is_ascii_alphanumeric() {
            if current_start.is_none() {
                current_start = Some(i);
            }
        } else if let Some(st) = current_start.take() {
            tokens.push(&s[st..i]);
        }
    }
    if let Some(st) = current_start {
        tokens.push(&s[st..]);
    }
    for tok in tokens {
        if tok.chars().all(|c| c.is_ascii_digit()) {
            numbers.push(tok);
        } else if tok.chars().all(|c| c.is_ascii_alphabetic()) {
            let lower = tok.to_ascii_lowercase();
            if lower.len() >= 3
                && let Some(m) = MONTHS.iter().position(|m| lower.starts_with(m))
            {
                month_name = Some(m as i8 + 1);
            }
            // Weekday names and AM/PM are ignored.
        } else {
            // "1st", "2nd": keep the digits.
            let digits: String = tok.chars().take_while(char::is_ascii_digit).collect();
            if !digits.is_empty() {
                numbers.push(&tok[..digits.len()]);
            }
        }
    }
    let year_of = |y: &str| -> Option<i16> {
        let v: i16 = y.parse().ok()?;
        match y.len() {
            2 => Some(2000 + v),
            4 => Some(v),
            _ => None,
        }
    };
    let (y, m, d): (i16, i8, i8) = if let Some(m) = month_name {
        // Month by name: the day and year are the remaining numbers, the year
        // being the four-digit (or last) one.
        let nums: Vec<&str> = numbers.into_iter().take(3).collect();
        match nums.as_slice() {
            [a, b, ..] if a.len() == 4 => (year_of(a)?, m, b.parse().ok()?),
            [a, b, ..] => (year_of(b)?, m, a.parse().ok()?),
            _ => return None,
        }
    } else {
        let nums: Vec<&str> = numbers.into_iter().take(3).collect();
        let [a, b, c] = nums.as_slice() else {
            return None;
        };
        if a.len() == 4 {
            // A four-digit first field is always year-month-day.
            (year_of(a)?, b.parse().ok()?, c.parse().ok()?)
        } else {
            match order {
                DateOrder::Ymd => (year_of(a)?, b.parse().ok()?, c.parse().ok()?),
                DateOrder::Mdy => (year_of(c)?, a.parse().ok()?, b.parse().ok()?),
                DateOrder::Dmy => (year_of(c)?, b.parse().ok()?, a.parse().ok()?),
            }
        }
    };
    Date::new(y, m, d).ok()
}

/// Words banks wrap around the merchant's name, which say how it was paid rather
/// than who was paid.
const NOISE: &[&str] = &[
    "POS",
    "PURCHASE",
    "DEBIT",
    "CARD",
    "CHECKCARD",
    "CHECK CARD",
    "VISA",
    "MASTERCARD",
    "RECURRING",
    "PAYMENT",
    "ACH",
    "WITHDRAWAL",
    "DEPOSIT",
    "ONLINE",
    "WEB",
    "PMT",
    "ID",
    "PPD",
    "CCD",
    "WEB ID",
    "SQ",
    "TST",
    "PP",
    "AUTHORIZED",
    "ON",
    "XX",
    "XXXX",
    "DES",
    "INDN",
    "CO",
];

/// A payee cleaned for comparing, not for showing: upper case, punctuation to
/// spaces, payment-method words and anything containing a digit (store numbers,
/// dates, card numbers, reference numbers) dropped. `"SQ *BLUE BOTTLE #0412
/// 10/01"` and `"Blue Bottle"` both become `"BLUE BOTTLE"`.
pub fn clean_payee(s: &str) -> String {
    let upper: String = s
        .to_uppercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let words: Vec<&str> = upper
        .split_whitespace()
        .filter(|w| !w.chars().any(|c| c.is_ascii_digit()))
        .filter(|w| !NOISE.contains(w))
        .collect();
    if words.is_empty() {
        // Nothing but noise: keep the plain upper-cased text so it still compares.
        return upper.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    words.join(" ")
}

/// A payee squeezed for building stable ids: upper case, letters and digits only.
pub fn squeeze(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}

/// How alike two names are, 0.0–1.0: the Dice coefficient over the words of their
/// cleaned forms, with a word counting as shared when one is a prefix of the
/// other of at least four letters (banks truncate: `COMCAST CABLE COMM` vs
/// `Comcast`). Used for scoring, never for money.
pub fn similarity(a: &str, b: &str) -> f64 {
    let a = clean_payee(a);
    let b = clean_payee(b);
    let wa: Vec<&str> = a.split_whitespace().collect();
    let wb: Vec<&str> = b.split_whitespace().collect();
    if wa.is_empty() || wb.is_empty() {
        return 0.0;
    }
    let same = |x: &str, y: &str| {
        x == y || (x.len().min(y.len()) >= 4 && (x.starts_with(y) || y.starts_with(x)))
    };
    let shared_a = wa.iter().filter(|x| wb.iter().any(|y| same(x, y))).count();
    let shared_b = wb.iter().filter(|y| wa.iter().any(|x| same(x, y))).count();
    (shared_a + shared_b) as f64 / (wa.len() + wb.len()) as f64
}

/// Mask an account number to its last four characters: `****1234`.
pub fn mask_account(id: &str) -> String {
    let compact: Vec<char> = id.chars().filter(|c| c.is_alphanumeric()).collect();
    if compact.is_empty() {
        return String::new();
    }
    let tail: String = compact[compact.len().saturating_sub(4)..].iter().collect();
    format!("****{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn amounts_from_every_kind_of_bank() {
        let cases = [
            ("-12.34", -1234),
            ("+12.34", 1234),
            ("12", 1200),
            ("1,234.56", 123_456),
            ("-1,234.56", -123_456),
            ("$1,234.56", 123_456),
            ("($12.34)", -1234),
            ("12.34-", -1234),
            ("1.234,56", 123_456),
            ("-12,34", -1234),
            ("12,345", 1_234_500),
            ("1,234,567", 123_456_700),
            (" 7.10 ", 710),
            ("12.3400", 1234),
            ("0.125", 13),
            ("-0.125", -13),
            ("USD 5.00", 500),
            ("5.00 USD", 500),
            ("'-5.00", -500),
        ];
        for (text, minor) in cases {
            assert_eq!(parse_amount(text), Ok(Money(minor)), "{text:?}");
        }
        assert!(parse_amount("").is_err());
        assert!(parse_amount("abc").is_err());
        assert!(parse_amount("1.2.3").is_err());
    }

    #[test]
    fn ofx_dates_keep_the_banks_calendar_day() {
        let d = date(2026, 10, 1);
        for s in [
            "20261001",
            "20261001120000",
            "20261001120000.000",
            "20261001120000.000[-8:PST]",
            "20261001000000[0:GMT]",
            "20261001235959.123[+5.30:IST]",
            "2026-10-01",
            "2026-10-01T23:00:00-08:00",
            " 20261001 ",
        ] {
            assert_eq!(parse_ofx_date(s), Some(d), "{s:?}");
        }
        assert_eq!(parse_ofx_date("2026100"), None);
        assert_eq!(parse_ofx_date("20261301"), None);
        assert_eq!(parse_ofx_date(""), None);
    }

    #[test]
    fn spreadsheet_dates_in_each_order() {
        let d = date(2026, 10, 1);
        let cases = [
            ("10/01/2026", DateOrder::Mdy),
            ("10/1/26", DateOrder::Mdy),
            ("2026-10-01", DateOrder::Mdy),
            ("2026-10-01", DateOrder::Dmy),
            ("2026/10/01", DateOrder::Ymd),
            ("20261001", DateOrder::Ymd),
            ("01/10/2026", DateOrder::Dmy),
            ("1.10.2026", DateOrder::Dmy),
            ("Oct 01, 2026", DateOrder::Mdy),
            ("1-Oct-2026", DateOrder::Dmy),
            ("01 October 2026", DateOrder::Dmy),
            ("10/01/2026 Thu", DateOrder::Mdy),
            ("10/01/2026 13:45:00", DateOrder::Mdy),
            ("2026-10-01T18:12:34Z", DateOrder::Mdy),
            ("2026-10-01T18:12:34.000Z", DateOrder::Mdy),
        ];
        for (s, order) in cases {
            assert_eq!(parse_date(s, order), Some(d), "{s:?} {order:?}");
        }
        assert_eq!(parse_date("13/01/2026", DateOrder::Mdy), None);
        assert_eq!(parse_date("Total", DateOrder::Mdy), None);
        assert_eq!(parse_date("", DateOrder::Mdy), None);
    }

    #[test]
    fn payees_clean_up_to_the_merchant() {
        assert_eq!(clean_payee("SQ *BLUE BOTTLE #0412 10/01"), "BLUE BOTTLE");
        assert_eq!(
            clean_payee("POS PURCHASE 4021 SAFEWAY STORE 1234"),
            "SAFEWAY STORE"
        );
        assert_eq!(clean_payee("Comcast Cable Comm."), "COMCAST CABLE COMM");
        assert_eq!(clean_payee("CHECK 1041"), "CHECK");
        assert!(similarity("COMCAST CABLE COMM", "Comcast") > 0.4);
        assert!(similarity("Blue Bottle Coffee", "SQ *BLUE BOTTLE #0412") > 0.7);
        assert!(similarity("Safeway", "Comcast") < 0.01);
        assert_eq!(similarity("", "Comcast"), 0.0);
    }

    #[test]
    fn decodes_whatever_the_bank_sent() {
        assert_eq!(decode(b"\xEF\xBB\xBFabc"), "abc");
        assert_eq!(decode(b"Caf\xE9"), "Café");
        assert_eq!(decode("Café".as_bytes()), "Café");
        assert_eq!(decode(b"\xFF\xFEa\x00b\x00"), "ab");
        assert_eq!(decode(b"\x93quoted\x94"), "\u{201c}quoted\u{201d}");
    }

    #[test]
    fn masks_account_numbers() {
        assert_eq!(mask_account("000123456789"), "****6789");
        assert_eq!(mask_account("12"), "****12");
        assert_eq!(mask_account("XXXX-XXXX-1234"), "****1234");
        assert_eq!(mask_account(""), "");
    }
}
