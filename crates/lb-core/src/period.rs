//! Date ranges the way QBD's report "Dates" box offers them.

use jiff::Span;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// An inclusive date range; either end may be open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DateRange {
    pub from: Option<Date>,
    pub to: Option<Date>,
}

impl DateRange {
    pub const ALL: DateRange = DateRange {
        from: None,
        to: None,
    };

    pub fn new(from: Date, to: Date) -> DateRange {
        DateRange {
            from: Some(from),
            to: Some(to),
        }
    }

    pub fn contains(&self, d: Date) -> bool {
        self.from.is_none_or(|f| d >= f) && self.to.is_none_or(|t| d <= t)
    }
}

/// QBD's date macros (`DateMacro` in qbXML), plus calendar variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum DateMacro {
    All,
    Today,
    Yesterday,
    ThisWeek,
    ThisWeekToDate,
    LastWeek,
    LastWeekToDate,
    NextWeek,
    NextFourWeeks,
    ThisMonth,
    ThisMonthToDate,
    LastMonth,
    LastMonthToDate,
    NextMonth,
    ThisFiscalQuarter,
    ThisFiscalQuarterToDate,
    LastFiscalQuarter,
    LastFiscalQuarterToDate,
    NextFiscalQuarter,
    ThisFiscalYear,
    ThisFiscalYearToDate,
    LastFiscalYear,
    LastFiscalYearToDate,
    NextFiscalYear,
    ThisCalendarQuarter,
    ThisCalendarQuarterToDate,
    LastCalendarQuarter,
    ThisCalendarYear,
    ThisCalendarYearToDate,
    LastCalendarYear,
}

impl DateMacro {
    pub const ALL_MACROS: [DateMacro; 30] = [
        DateMacro::All,
        DateMacro::Today,
        DateMacro::Yesterday,
        DateMacro::ThisWeek,
        DateMacro::ThisWeekToDate,
        DateMacro::LastWeek,
        DateMacro::LastWeekToDate,
        DateMacro::NextWeek,
        DateMacro::NextFourWeeks,
        DateMacro::ThisMonth,
        DateMacro::ThisMonthToDate,
        DateMacro::LastMonth,
        DateMacro::LastMonthToDate,
        DateMacro::NextMonth,
        DateMacro::ThisFiscalQuarter,
        DateMacro::ThisFiscalQuarterToDate,
        DateMacro::LastFiscalQuarter,
        DateMacro::LastFiscalQuarterToDate,
        DateMacro::NextFiscalQuarter,
        DateMacro::ThisFiscalYear,
        DateMacro::ThisFiscalYearToDate,
        DateMacro::LastFiscalYear,
        DateMacro::LastFiscalYearToDate,
        DateMacro::NextFiscalYear,
        DateMacro::ThisCalendarQuarter,
        DateMacro::ThisCalendarQuarterToDate,
        DateMacro::LastCalendarQuarter,
        DateMacro::ThisCalendarYear,
        DateMacro::ThisCalendarYearToDate,
        DateMacro::LastCalendarYear,
    ];

    /// The range, for `today`, in a company whose fiscal year starts in
    /// `fiscal_start_month` (1–12). Weeks start on Sunday, as in US QBD.
    pub fn range(self, today: Date, fiscal_start_month: i8) -> DateRange {
        let fy = |offset: i16| fiscal_year(today, fiscal_start_month, offset);
        let fq = |offset: i32| fiscal_quarter(today, fiscal_start_month, offset);
        let cq = |offset: i32| fiscal_quarter(today, 1, offset);
        let week = |offset: i64| {
            let start = week_start(today) + Span::new().weeks(offset);
            (start, start + Span::new().days(6))
        };
        let month = |offset: i64| {
            let start = today.first_of_month() + Span::new().months(offset);
            (start, start.last_of_month())
        };
        let to_date = |(from, _): (Date, Date)| DateRange::new(from, today);
        let r = |(from, to): (Date, Date)| DateRange::new(from, to);
        // "Last X to date": from the start of last X to the same point within it.
        let last_to_date = |(from, to): (Date, Date), this_from: Date| {
            let elapsed = this_from.until(today).unwrap_or_default();
            let end = (from + elapsed).min(to);
            DateRange::new(from, end)
        };
        match self {
            DateMacro::All => DateRange::ALL,
            DateMacro::Today => DateRange::new(today, today),
            DateMacro::Yesterday => {
                let y = today - Span::new().days(1);
                DateRange::new(y, y)
            }
            DateMacro::ThisWeek => r(week(0)),
            DateMacro::ThisWeekToDate => to_date(week(0)),
            DateMacro::LastWeek => r(week(-1)),
            DateMacro::LastWeekToDate => last_to_date(week(-1), week(0).0),
            DateMacro::NextWeek => r(week(1)),
            DateMacro::NextFourWeeks => DateRange::new(week(1).0, week(4).1),
            DateMacro::ThisMonth => r(month(0)),
            DateMacro::ThisMonthToDate => to_date(month(0)),
            DateMacro::LastMonth => r(month(-1)),
            DateMacro::LastMonthToDate => last_to_date(month(-1), month(0).0),
            DateMacro::NextMonth => r(month(1)),
            DateMacro::ThisFiscalQuarter => r(fq(0)),
            DateMacro::ThisFiscalQuarterToDate => to_date(fq(0)),
            DateMacro::LastFiscalQuarter => r(fq(-1)),
            DateMacro::LastFiscalQuarterToDate => last_to_date(fq(-1), fq(0).0),
            DateMacro::NextFiscalQuarter => r(fq(1)),
            DateMacro::ThisFiscalYear => r(fy(0)),
            DateMacro::ThisFiscalYearToDate => to_date(fy(0)),
            DateMacro::LastFiscalYear => r(fy(-1)),
            DateMacro::LastFiscalYearToDate => last_to_date(fy(-1), fy(0).0),
            DateMacro::NextFiscalYear => r(fy(1)),
            DateMacro::ThisCalendarQuarter => r(cq(0)),
            DateMacro::ThisCalendarQuarterToDate => to_date(cq(0)),
            DateMacro::LastCalendarQuarter => r(cq(-1)),
            DateMacro::ThisCalendarYear => r(fiscal_year(today, 1, 0)),
            DateMacro::ThisCalendarYearToDate => to_date(fiscal_year(today, 1, 0)),
            DateMacro::LastCalendarYear => r(fiscal_year(today, 1, -1)),
        }
    }
}

/// Sunday on or before `d`.
pub fn week_start(d: Date) -> Date {
    let back = i64::from(d.weekday().to_sunday_zero_offset());
    d - Span::new().days(back)
}

/// First and last day of the fiscal year containing `d`, shifted by `offset` years.
pub fn fiscal_year(d: Date, start_month: i8, offset: i16) -> (Date, Date) {
    let start_month = start_month.clamp(1, 12);
    let year = if d.month() >= start_month {
        d.year()
    } else {
        d.year() - 1
    } + offset;
    let from = Date::new(year, start_month, 1).expect("valid fiscal start");
    let to = from + Span::new().years(1) - Span::new().days(1);
    (from, to)
}

/// First and last day of the fiscal quarter containing `d`, shifted by `offset`.
pub fn fiscal_quarter(d: Date, start_month: i8, offset: i32) -> (Date, Date) {
    let (fy_start, _) = fiscal_year(d, start_month, 0);
    let months_in = (i32::from(d.year()) - i32::from(fy_start.year())) * 12 + i32::from(d.month())
        - i32::from(fy_start.month());
    let q = months_in.div_euclid(3) + offset;
    let from = fy_start + Span::new().months(i64::from(q * 3));
    let to = from + Span::new().months(3) - Span::new().days(1);
    (from, to)
}

/// QBD's date-field keys: `+`/`-` a day, `T` today, `W`/`K` start/end of week,
/// `M`/`H` start/end of month, `Y`/`R` start/end of year, `[`/`]` a week back or
/// forward. Returns `None` for keys that are not shortcuts.
pub fn date_key(current: Date, today: Date, key: char) -> Option<Date> {
    let d = current;
    Some(match key.to_ascii_lowercase() {
        '+' | '=' => d + Span::new().days(1),
        '-' | '_' => d - Span::new().days(1),
        't' => today,
        'w' => week_start(d),
        'k' => week_start(d) + Span::new().days(6),
        'm' => d.first_of_month(),
        'h' => d.last_of_month(),
        'y' => Date::new(d.year(), 1, 1).ok()?,
        'r' => Date::new(d.year(), 12, 31).ok()?,
        '[' => d - Span::new().weeks(1),
        ']' => d + Span::new().weeks(1),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn weeks_start_sunday() {
        // 2026-10-19 is a Monday.
        assert_eq!(week_start(date(2026, 10, 19)), date(2026, 10, 18));
        assert_eq!(week_start(date(2026, 10, 18)), date(2026, 10, 18));
    }

    #[test]
    fn macros_for_a_calendar_year_company() {
        let today = date(2026, 10, 19);
        assert_eq!(
            DateMacro::ThisMonth.range(today, 1),
            DateRange::new(date(2026, 10, 1), date(2026, 10, 31))
        );
        assert_eq!(
            DateMacro::LastMonth.range(today, 1),
            DateRange::new(date(2026, 9, 1), date(2026, 9, 30))
        );
        assert_eq!(
            DateMacro::ThisFiscalYearToDate.range(today, 1),
            DateRange::new(date(2026, 1, 1), today)
        );
        assert_eq!(
            DateMacro::LastFiscalYear.range(today, 1),
            DateRange::new(date(2025, 1, 1), date(2025, 12, 31))
        );
        assert_eq!(
            DateMacro::ThisFiscalQuarter.range(today, 1),
            DateRange::new(date(2026, 10, 1), date(2026, 12, 31))
        );
        assert_eq!(
            DateMacro::LastFiscalQuarter.range(today, 1),
            DateRange::new(date(2026, 7, 1), date(2026, 9, 30))
        );
        assert_eq!(
            DateMacro::LastMonthToDate.range(today, 1),
            DateRange::new(date(2026, 9, 1), date(2026, 9, 19))
        );
        assert_eq!(
            DateMacro::ThisWeek.range(today, 1),
            DateRange::new(date(2026, 10, 18), date(2026, 10, 24))
        );
    }

    #[test]
    fn macros_for_a_july_fiscal_year() {
        let today = date(2026, 3, 5);
        assert_eq!(
            DateMacro::ThisFiscalYear.range(today, 7),
            DateRange::new(date(2025, 7, 1), date(2026, 6, 30))
        );
        assert_eq!(
            DateMacro::ThisFiscalQuarter.range(today, 7),
            DateRange::new(date(2026, 1, 1), date(2026, 3, 31))
        );
        assert_eq!(
            DateMacro::NextFiscalQuarter.range(today, 7),
            DateRange::new(date(2026, 4, 1), date(2026, 6, 30))
        );
        assert_eq!(
            DateMacro::LastFiscalQuarter.range(today, 7),
            DateRange::new(date(2025, 10, 1), date(2025, 12, 31))
        );
    }

    #[test]
    fn last_month_to_date_clamps_to_short_months() {
        let today = date(2026, 3, 31);
        assert_eq!(
            DateMacro::LastMonthToDate.range(today, 1),
            DateRange::new(date(2026, 2, 1), date(2026, 2, 28))
        );
    }

    #[test]
    fn date_field_keys() {
        let today = date(2026, 10, 19);
        let d = date(2026, 2, 14);
        assert_eq!(date_key(d, today, '+'), Some(date(2026, 2, 15)));
        assert_eq!(date_key(d, today, '-'), Some(date(2026, 2, 13)));
        assert_eq!(date_key(d, today, 't'), Some(today));
        assert_eq!(date_key(d, today, 'M'), Some(date(2026, 2, 1)));
        assert_eq!(date_key(d, today, 'h'), Some(date(2026, 2, 28)));
        assert_eq!(date_key(d, today, 'y'), Some(date(2026, 1, 1)));
        assert_eq!(date_key(d, today, 'r'), Some(date(2026, 12, 31)));
        assert_eq!(date_key(d, today, 'x'), None);
    }
}
