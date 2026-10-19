//! Grandfather-father-son retention, as a pure function.
//!
//! The rules are borg's and restic's, which people who run backups already know:
//!
//! - `keep_last: N` keeps the N newest backups.
//! - `daily: N` keeps the newest backup of each of the last N *days that have a
//!   backup*; `weekly` (ISO weeks), `monthly` and `yearly` likewise.
//! - A backup is kept if any rule keeps it; the rest are deleted.
//!
//! "Days that have a backup", not "the last N calendar days": if backups stop (a
//! full disk, a lapsed card), time passing alone never deletes the history that
//! is left. Periods are counted in the time zone of `now`, so "daily" means the
//! company's days, not UTC's.
//!
//! Two safety rules on top: a policy of all zeros keeps everything, and a backup
//! dated after `now` (a clock that was wrong) is kept and does not count against
//! any rule, since nothing sensible can be said about it.

use std::collections::HashSet;

use jiff::{Timestamp, Zoned};

use crate::config::Retention;

/// What retention needs to know about a backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dated {
    pub id: String,
    pub created_at: Timestamp,
}

impl Dated {
    pub fn new(id: impl Into<String>, created_at: Timestamp) -> Self {
        Dated {
            id: id.into(),
            created_at,
        }
    }
}

/// The backups `policy` would delete, given the time and time zone `now`. Oldest
/// first. The input may be in any order; ids are assumed unique.
pub fn plan_retention(backups: &[Dated], policy: &Retention, now: &Zoned) -> Vec<String> {
    if *policy == Retention::ALL {
        return Vec::new();
    }
    let tz = now.time_zone();
    let now = now.timestamp();

    // Newest first; ties broken by id so the result does not depend on input order.
    let mut past: Vec<&Dated> = backups.iter().filter(|b| b.created_at <= now).collect();
    past.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));

    let mut keep: HashSet<&str> = HashSet::new();
    for b in past.iter().take(policy.keep_last as usize) {
        keep.insert(&b.id);
    }

    type Period = (i32, i32);
    let rules: [(u32, fn(&Zoned) -> Period); 4] = [
        (policy.daily, |z| {
            (i32::from(z.year()), i32::from(z.day_of_year()))
        }),
        (policy.weekly, |z| {
            let w = z.date().iso_week_date();
            (i32::from(w.year()), i32::from(w.week()))
        }),
        (policy.monthly, |z| {
            (i32::from(z.year()), i32::from(z.month()))
        }),
        (policy.yearly, |z| (i32::from(z.year()), 0)),
    ];
    for (count, period_of) in rules {
        let mut last: Option<Period> = None;
        let mut kept = 0;
        for b in &past {
            if kept == count {
                break;
            }
            let period = period_of(&b.created_at.to_zoned(tz.clone()));
            if last != Some(period) {
                keep.insert(&b.id);
                last = Some(period);
                kept += 1;
            }
        }
    }

    let mut delete: Vec<&Dated> = past
        .into_iter()
        .filter(|b| !keep.contains(b.id.as_str()))
        .collect();
    delete.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    delete.into_iter().map(|b| b.id.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::{Date, date};
    use jiff::tz::TimeZone;
    use pretty_assertions::assert_eq;

    fn la() -> TimeZone {
        TimeZone::get("America/Los_Angeles").unwrap()
    }

    /// A backup at a local time in Los Angeles; its id is that time, readable.
    fn at(d: Date, h: i8, m: i8) -> Dated {
        let z = d.at(h, m, 0, 0).to_zoned(la()).unwrap();
        Dated::new(z.strftime("%Y-%m-%d %H:%M").to_string(), z.timestamp())
    }

    fn now(d: Date, h: i8) -> Zoned {
        d.at(h, 0, 0, 0).to_zoned(la()).unwrap()
    }

    fn policy(keep_last: u32, daily: u32, weekly: u32, monthly: u32, yearly: u32) -> Retention {
        Retention {
            keep_last,
            daily,
            weekly,
            monthly,
            yearly,
        }
    }

    /// Kept ids, newest first, for readable assertions.
    fn kept(backups: &[Dated], p: &Retention, now: &Zoned) -> Vec<String> {
        let gone = plan_retention(backups, p, now);
        let mut k: Vec<&Dated> = backups.iter().filter(|b| !gone.contains(&b.id)).collect();
        k.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        k.into_iter().map(|b| b.id.clone()).collect()
    }

    /// Every six hours (02, 08, 14, 20 local) for `days` days ending on `last`.
    fn four_a_day(last: Date, days: i32) -> Vec<Dated> {
        let mut out = Vec::new();
        for i in 0..days {
            let d = last.checked_sub(jiff::Span::new().days(i)).unwrap();
            for h in [2, 8, 14, 20] {
                out.push(at(d, h, 0));
            }
        }
        out
    }

    #[test]
    fn an_empty_policy_keeps_everything_and_an_empty_list_deletes_nothing() {
        let b = four_a_day(date(2026, 10, 19), 30);
        assert_eq!(
            plan_retention(&b, &Retention::ALL, &now(date(2026, 10, 19), 23)),
            Vec::<String>::new()
        );
        assert_eq!(
            plan_retention(&[], &Retention::default(), &now(date(2026, 10, 19), 23)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn keep_last_keeps_the_newest_n() {
        let b = four_a_day(date(2026, 10, 19), 2);
        let k = kept(&b, &policy(3, 0, 0, 0, 0), &now(date(2026, 10, 19), 23));
        assert_eq!(
            k,
            ["2026-10-19 20:00", "2026-10-19 14:00", "2026-10-19 08:00"]
        );
        // More than there are: all kept.
        assert_eq!(
            plan_retention(&b, &policy(100, 0, 0, 0, 0), &now(date(2026, 10, 19), 23)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn daily_keeps_the_newest_backup_of_each_day() {
        let b = four_a_day(date(2026, 10, 19), 5);
        let k = kept(&b, &policy(0, 3, 0, 0, 0), &now(date(2026, 10, 19), 23));
        assert_eq!(
            k,
            ["2026-10-19 20:00", "2026-10-18 20:00", "2026-10-17 20:00"]
        );
    }

    #[test]
    fn days_are_the_time_zone_of_now_not_utc() {
        // 20:00 in Los Angeles is 03:00 UTC the next day. In UTC terms the 18th's
        // 20:00 and the 19th's 02:00 share a day; in local terms they do not.
        let b = vec![
            at(date(2026, 10, 18), 20, 0),
            at(date(2026, 10, 19), 2, 0),
            at(date(2026, 10, 19), 8, 0),
        ];
        let p = policy(0, 2, 0, 0, 0);
        assert_eq!(
            kept(&b, &p, &now(date(2026, 10, 19), 23)),
            ["2026-10-19 08:00", "2026-10-18 20:00"]
        );
        // Counted in UTC, all three fall on the 19th (03:00Z, 09:00Z, 15:00Z), so
        // two dailies keep only one.
        let utc = now(date(2026, 10, 19), 23).with_time_zone(TimeZone::UTC);
        assert_eq!(kept(&b, &p, &utc), ["2026-10-19 08:00"]);
    }

    #[test]
    fn days_without_backups_do_not_count() {
        // Backups stopped on the 1st; it is now the 19th. Seven dailies still keeps
        // seven distinct days of backups.
        let b = four_a_day(date(2026, 10, 1), 10);
        let k = kept(&b, &policy(0, 7, 0, 0, 0), &now(date(2026, 10, 19), 12));
        assert_eq!(k.len(), 7);
        assert_eq!(k[0], "2026-10-01 20:00");
        assert_eq!(k[6], "2026-09-25 20:00");
    }

    #[test]
    fn weekly_uses_iso_weeks() {
        // 2026-10-19 is a Monday. Sunday the 18th belongs to the previous ISO week.
        let b = vec![
            at(date(2026, 10, 19), 2, 0), // week 43
            at(date(2026, 10, 18), 2, 0), // week 42 (Sunday)
            at(date(2026, 10, 12), 2, 0), // week 42 (Monday)
            at(date(2026, 10, 11), 2, 0), // week 41
            at(date(2026, 10, 4), 2, 0),  // week 40
        ];
        let k = kept(&b, &policy(0, 0, 3, 0, 0), &now(date(2026, 10, 19), 12));
        assert_eq!(
            k,
            ["2026-10-19 02:00", "2026-10-18 02:00", "2026-10-11 02:00"]
        );
    }

    #[test]
    fn iso_weeks_straddle_new_year() {
        // 2026-01-01 is a Thursday: ISO week 1 of 2026 starts Monday 2025-12-29.
        let b = vec![
            at(date(2026, 1, 2), 2, 0),
            at(date(2025, 12, 29), 2, 0),
            at(date(2025, 12, 28), 2, 0), // Sunday: week 52 of 2025
        ];
        let k = kept(&b, &policy(0, 0, 2, 0, 0), &now(date(2026, 1, 5), 12));
        assert_eq!(k, ["2026-01-02 02:00", "2025-12-28 02:00"]);
    }

    #[test]
    fn monthly_and_yearly_keep_the_newest_of_each_period() {
        let mut b = Vec::new();
        for (y, m) in [
            (2024, 11),
            (2024, 12),
            (2025, 1),
            (2025, 6),
            (2025, 12),
            (2026, 3),
            (2026, 10),
        ] {
            b.push(at(date(y, m, 1), 2, 0));
            b.push(at(date(y, m, 15), 2, 0));
        }
        let k = kept(&b, &policy(0, 0, 0, 3, 0), &now(date(2026, 10, 19), 12));
        assert_eq!(
            k,
            ["2026-10-15 02:00", "2026-03-15 02:00", "2025-12-15 02:00"]
        );
        let k = kept(&b, &policy(0, 0, 0, 0, 5), &now(date(2026, 10, 19), 12));
        assert_eq!(
            k,
            ["2026-10-15 02:00", "2025-12-15 02:00", "2024-12-15 02:00"]
        );
    }

    #[test]
    fn rules_combine_as_a_union() {
        let b = four_a_day(date(2026, 10, 19), 60);
        let p = policy(2, 3, 2, 2, 1);
        let k = kept(&b, &p, &now(date(2026, 10, 19), 23));
        assert_eq!(
            k,
            [
                "2026-10-19 20:00", // last, daily, weekly, monthly, yearly
                "2026-10-19 14:00", // last
                "2026-10-18 20:00", // daily, weekly (Sunday ends week 42)
                "2026-10-17 20:00", // daily
                "2026-09-30 20:00", // monthly
            ]
        );
        assert_eq!(
            plan_retention(&b, &p, &now(date(2026, 10, 19), 23)).len(),
            240 - 5
        );
    }

    #[test]
    fn deletions_are_listed_oldest_first() {
        let b = four_a_day(date(2026, 10, 19), 1);
        let gone = plan_retention(&b, &policy(1, 0, 0, 0, 0), &now(date(2026, 10, 19), 23));
        assert_eq!(
            gone,
            ["2026-10-19 02:00", "2026-10-19 08:00", "2026-10-19 14:00"]
        );
    }

    #[test]
    fn backups_from_the_future_are_left_alone() {
        let b = vec![
            at(date(2026, 10, 25), 2, 0), // clock was wrong
            at(date(2026, 10, 19), 2, 0),
            at(date(2026, 10, 18), 2, 0),
        ];
        let gone = plan_retention(&b, &policy(1, 0, 0, 0, 0), &now(date(2026, 10, 19), 12));
        assert_eq!(gone, ["2026-10-18 02:00"]);
    }

    #[test]
    fn input_order_does_not_matter() {
        let mut b = four_a_day(date(2026, 10, 19), 20);
        let p = Retention::default();
        let n = now(date(2026, 10, 19), 23);
        let first = plan_retention(&b, &p, &n);
        b.reverse();
        b.swap(3, 40);
        assert_eq!(plan_retention(&b, &p, &n), first);
    }

    #[test]
    fn the_newest_backup_always_survives_a_nonzero_policy() {
        let b = four_a_day(date(2026, 10, 19), 3);
        let n = now(date(2026, 10, 19), 23);
        for p in [
            policy(1, 0, 0, 0, 0),
            policy(0, 1, 0, 0, 0),
            policy(0, 0, 1, 0, 0),
            policy(0, 0, 0, 1, 0),
            policy(0, 0, 0, 0, 1),
        ] {
            let k = kept(&b, &p, &n);
            assert_eq!(k, ["2026-10-19 20:00"], "{p:?}");
        }
    }
}
