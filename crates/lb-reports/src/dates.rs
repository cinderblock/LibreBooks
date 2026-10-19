//! Dates as reports print and slice them: QBD's date line under the title, short
//! column headings, period columns (weeks, months, fiscal quarters and years),
//! and the earlier periods comparisons look at.

use jiff::Span;
use jiff::civil::Date;
use lb_core::DateRange;
use lb_core::period::{fiscal_quarter, fiscal_year, week_start};

use crate::request::ColumnLayout;

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

pub fn month_name(month: i8) -> &'static str {
    MONTHS[(month.clamp(1, 12) - 1) as usize]
}

pub fn short_month(month: i8) -> &'static str {
    &month_name(month)[..3]
}

/// "October 19, 2026".
pub fn long_date(d: Date) -> String {
    format!("{} {}, {}", month_name(d.month()), d.day(), d.year())
}

fn is_month_start(d: Date) -> bool {
    d.day() == 1
}

fn is_month_end(d: Date) -> bool {
    d == d.last_of_month()
}

/// The date line under a report's title for a range, as QBD prints it.
pub fn range_line(range: DateRange) -> String {
    match (range.from, range.to) {
        (None, None) => "All Transactions".to_owned(),
        (None, Some(t)) => format!("Through {}", long_date(t)),
        (Some(f), None) => format!("Since {}", long_date(f)),
        (Some(f), Some(t)) if f == t => long_date(f),
        (Some(f), Some(t)) if is_month_start(f) && is_month_end(t) => {
            if f.year() == t.year() && f.month() == t.month() {
                format!("{} {}", month_name(f.month()), f.year())
            } else if f.year() == t.year() {
                format!(
                    "{} through {} {}",
                    month_name(f.month()),
                    month_name(t.month()),
                    t.year()
                )
            } else {
                format!(
                    "{} {} through {} {}",
                    month_name(f.month()),
                    f.year(),
                    month_name(t.month()),
                    t.year()
                )
            }
        }
        (Some(f), Some(t)) if f.year() == t.year() && f.month() == t.month() => format!(
            "{} {} - {}, {}",
            month_name(f.month()),
            f.day(),
            t.day(),
            t.year()
        ),
        (Some(f), Some(t)) if f.year() == t.year() => format!(
            "{} {} through {} {}, {}",
            month_name(f.month()),
            f.day(),
            month_name(t.month()),
            t.day(),
            t.year()
        ),
        (Some(f), Some(t)) => format!("{} through {}", long_date(f), long_date(t)),
    }
}

/// The date line for an "as of" report.
pub fn as_of_line(d: Option<Date>) -> String {
    match d {
        Some(d) => format!("As of {}", long_date(d)),
        None => "All Transactions".to_owned(),
    }
}

fn yy(year: i16) -> String {
    format!("{:02}", year.rem_euclid(100))
}

/// A column heading for a range: "Oct 26", "Jan - Mar 26", "Oct 1 - 19, 26".
pub fn short_range(range: DateRange) -> String {
    match (range.from, range.to) {
        (None, None) => "TOTAL".to_owned(),
        (None, Some(t)) => short_date(t),
        (Some(f), None) => format!("{} -", short_date(f)),
        (Some(f), Some(t)) if f == t => short_date(f),
        (Some(f), Some(t)) if is_month_start(f) && is_month_end(t) => {
            if f.year() == t.year() && f.month() == t.month() {
                format!("{} {}", short_month(f.month()), yy(f.year()))
            } else if f.year() == t.year() {
                format!(
                    "{} - {} {}",
                    short_month(f.month()),
                    short_month(t.month()),
                    yy(t.year())
                )
            } else {
                format!(
                    "{} {} - {} {}",
                    short_month(f.month()),
                    yy(f.year()),
                    short_month(t.month()),
                    yy(t.year())
                )
            }
        }
        (Some(f), Some(t)) if f.year() == t.year() && f.month() == t.month() => format!(
            "{} {} - {}, {}",
            short_month(f.month()),
            f.day(),
            t.day(),
            yy(t.year())
        ),
        (Some(f), Some(t)) if f.year() == t.year() => format!(
            "{} {} - {} {}, {}",
            short_month(f.month()),
            f.day(),
            short_month(t.month()),
            t.day(),
            yy(t.year())
        ),
        (Some(f), Some(t)) => format!("{} - {}", short_date(f), short_date(t)),
    }
}

/// "Oct 19, 26".
pub fn short_date(d: Date) -> String {
    format!("{} {}, {}", short_month(d.month()), d.day(), yy(d.year()))
}

/// The periods a range splits into for a period layout, clipped to the range.
/// Non-period layouts give the range itself.
pub fn buckets(from: Date, to: Date, layout: ColumnLayout, fiscal_start: i8) -> Vec<DateRange> {
    if to < from {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cursor = from;
    while cursor <= to {
        let end = match layout {
            ColumnLayout::Week => week_start(cursor) + Span::new().days(6),
            ColumnLayout::Month => cursor.last_of_month(),
            ColumnLayout::Quarter => fiscal_quarter(cursor, fiscal_start, 0).1,
            ColumnLayout::Year => fiscal_year(cursor, fiscal_start, 0).1,
            _ => to,
        };
        let end = end.min(to);
        out.push(DateRange::new(cursor, end));
        match end.tomorrow() {
            Ok(next) => cursor = next,
            Err(_) => break,
        }
    }
    out
}

/// The first day of the fiscal year containing `d`.
pub fn fiscal_year_start(d: Date, fiscal_start: i8) -> Date {
    fiscal_year(d, fiscal_start, 0).0
}

fn shift_months(d: Date, months: i64) -> Date {
    if is_month_end(d) {
        (d.first_of_month() + Span::new().months(months)).last_of_month()
    } else {
        d.saturating_add(Span::new().months(months))
    }
}

/// The same range a year earlier (month ends stay month ends: Feb 29 → Feb 28).
pub fn previous_year(r: DateRange) -> DateRange {
    DateRange {
        from: r.from.map(|d| shift_months(d, -12)),
        to: r.to.map(|d| shift_months(d, -12)),
    }
}

/// The period just before: whole months shift by their number of months (so the
/// period before October is September), anything else by its length in days.
pub fn previous_period(r: DateRange) -> DateRange {
    let (Some(f), Some(t)) = (r.from, r.to) else {
        return previous_year(r);
    };
    if is_month_start(f) && is_month_end(t) {
        let months = i64::from(t.year() - f.year()) * 12 + i64::from(t.month() - f.month()) + 1;
        let from = f + Span::new().months(-months);
        let to = (from + Span::new().months(months - 1)).last_of_month();
        DateRange::new(from, to)
    } else {
        let days = f.until(t).map_or(0, |s| i64::from(s.get_days())) + 1;
        let to = f - Span::new().days(1);
        let from = to - Span::new().days(days - 1);
        DateRange::new(from, to)
    }
}

/// Whole days from `a` to `b` (negative when `b` is earlier).
pub fn days_between(a: Date, b: Date) -> i64 {
    a.until(b).map_or(0, |s| i64::from(s.get_days()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn r(f: Date, t: Date) -> DateRange {
        DateRange::new(f, t)
    }

    #[test]
    fn date_lines_read_like_qbd() {
        assert_eq!(
            range_line(r(date(2026, 1, 1), date(2026, 10, 19))),
            "January 1 through October 19, 2026"
        );
        assert_eq!(
            range_line(r(date(2026, 1, 1), date(2026, 10, 31))),
            "January through October 2026"
        );
        assert_eq!(
            range_line(r(date(2026, 10, 1), date(2026, 10, 31))),
            "October 2026"
        );
        assert_eq!(
            range_line(r(date(2026, 10, 1), date(2026, 10, 19))),
            "October 1 - 19, 2026"
        );
        assert_eq!(
            range_line(r(date(2025, 7, 1), date(2026, 6, 30))),
            "July 2025 through June 2026"
        );
        assert_eq!(range_line(DateRange::ALL), "All Transactions");
        assert_eq!(
            as_of_line(Some(date(2026, 10, 19))),
            "As of October 19, 2026"
        );
        assert_eq!(
            short_range(r(date(2026, 10, 1), date(2026, 10, 31))),
            "Oct 26"
        );
        assert_eq!(
            short_range(r(date(2026, 1, 1), date(2026, 3, 31))),
            "Jan - Mar 26"
        );
        assert_eq!(
            short_range(r(date(2026, 1, 1), date(2026, 10, 19))),
            "Jan 1 - Oct 19, 26"
        );
    }

    #[test]
    fn buckets_split_and_clip() {
        let months = buckets(date(2026, 1, 15), date(2026, 3, 10), ColumnLayout::Month, 1);
        assert_eq!(
            months,
            vec![
                r(date(2026, 1, 15), date(2026, 1, 31)),
                r(date(2026, 2, 1), date(2026, 2, 28)),
                r(date(2026, 3, 1), date(2026, 3, 10)),
            ]
        );
        // Fiscal quarters for a July year.
        let q = buckets(
            date(2025, 7, 1),
            date(2026, 6, 30),
            ColumnLayout::Quarter,
            7,
        );
        assert_eq!(q.len(), 4);
        assert_eq!(q[2], r(date(2026, 1, 1), date(2026, 3, 31)));
        // Weeks run Sunday to Saturday; 2026-10-19 is a Monday.
        let w = buckets(
            date(2026, 10, 19),
            date(2026, 10, 31),
            ColumnLayout::Week,
            1,
        );
        assert_eq!(
            w,
            vec![
                r(date(2026, 10, 19), date(2026, 10, 24)),
                r(date(2026, 10, 25), date(2026, 10, 31)),
            ]
        );
    }

    #[test]
    fn comparisons_shift_sensibly() {
        assert_eq!(
            previous_period(r(date(2026, 10, 1), date(2026, 10, 31))),
            r(date(2026, 9, 1), date(2026, 9, 30))
        );
        assert_eq!(
            previous_period(r(date(2026, 1, 1), date(2026, 3, 31))),
            r(date(2025, 10, 1), date(2025, 12, 31))
        );
        assert_eq!(
            previous_period(r(date(2026, 10, 10), date(2026, 10, 19))),
            r(date(2026, 9, 30), date(2026, 10, 9))
        );
        assert_eq!(
            previous_year(r(date(2028, 2, 1), date(2028, 2, 29))),
            r(date(2027, 2, 1), date(2027, 2, 28))
        );
    }
}
