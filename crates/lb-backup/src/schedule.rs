//! When the next backup is due, as a pure function of the schedule, the last
//! backup and the time.

use jiff::civil::Date;
use jiff::{Timestamp, Zoned};

use crate::config::Schedule;

/// When the next backup should run. `None` when the schedule is off or has no
/// rule. A company never backed up is due `now`. A time already past (the machine
/// was asleep at 02:00) is returned as is: the caller runs it at once.
///
/// `now` carries the time zone the daily time is read in.
pub fn next_run(schedule: &Schedule, last: Option<Timestamp>, now: &Zoned) -> Option<Timestamp> {
    if !schedule.enabled || (schedule.every.is_none() && schedule.daily_at.is_none()) {
        return None;
    }
    let Some(last) = last else {
        return Some(now.timestamp());
    };
    let by_interval = schedule
        .every
        .map(|every| last.checked_add(every).unwrap_or(Timestamp::MAX));
    let by_time = schedule.daily_at.map(|time| {
        let tz = now.time_zone();
        // In a DST gap (02:30 on a spring-forward day) jiff moves the time forward
        // by the gap, which is what a person expects.
        let on = |day: Date| {
            day.to_datetime(time)
                .to_zoned(tz.clone())
                .map_or(Timestamp::MAX, |z| z.timestamp())
        };
        let last_day = last.to_zoned(tz.clone()).date();
        let same_day = on(last_day);
        if same_day > last {
            same_day
        } else {
            last_day.tomorrow().map_or(Timestamp::MAX, on)
        }
    });
    match (by_interval, by_time) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::SignedDuration;
    use jiff::civil::{date, time};
    use jiff::tz::TimeZone;

    fn la(y: i16, m: i8, d: i8, h: i8, min: i8) -> Zoned {
        date(y, m, d)
            .at(h, min, 0, 0)
            .to_zoned(TimeZone::get("America/Los_Angeles").unwrap())
            .unwrap()
    }

    fn every(hours: i64) -> Schedule {
        Schedule {
            enabled: true,
            every: Some(SignedDuration::from_hours(hours)),
            daily_at: None,
        }
    }

    fn daily(h: i8, m: i8) -> Schedule {
        Schedule {
            enabled: true,
            every: None,
            daily_at: Some(time(h, m, 0, 0)),
        }
    }

    #[test]
    fn off_or_ruleless_schedules_never_run() {
        let now = la(2026, 10, 19, 12, 0);
        let off = Schedule {
            enabled: false,
            ..every(6)
        };
        assert_eq!(next_run(&off, None, &now), None);
        let empty = Schedule {
            enabled: true,
            every: None,
            daily_at: None,
        };
        assert_eq!(next_run(&empty, None, &now), None);
    }

    #[test]
    fn a_company_never_backed_up_is_due_now() {
        let now = la(2026, 10, 19, 12, 0);
        assert_eq!(next_run(&every(6), None, &now), Some(now.timestamp()));
        assert_eq!(next_run(&daily(2, 0), None, &now), Some(now.timestamp()));
    }

    #[test]
    fn an_interval_counts_from_the_last_backup() {
        let now = la(2026, 10, 19, 12, 0);
        let last = la(2026, 10, 19, 9, 0).timestamp();
        assert_eq!(
            next_run(&every(6), Some(last), &now),
            Some(la(2026, 10, 19, 15, 0).timestamp())
        );
        // Long overdue: in the past, so it runs at once.
        let old = la(2026, 10, 1, 9, 0).timestamp();
        assert!(next_run(&every(6), Some(old), &now).unwrap() < now.timestamp());
    }

    #[test]
    fn a_daily_time_is_the_first_one_after_the_last_backup() {
        let now = la(2026, 10, 19, 12, 0);
        // Last at 01:00: today's 02:30 is next (already past, so: run now).
        let last = la(2026, 10, 19, 1, 0).timestamp();
        assert_eq!(
            next_run(&daily(2, 30), Some(last), &now),
            Some(la(2026, 10, 19, 2, 30).timestamp())
        );
        // Last at 02:30 exactly: tomorrow's.
        let last = la(2026, 10, 19, 2, 30).timestamp();
        assert_eq!(
            next_run(&daily(2, 30), Some(last), &now),
            Some(la(2026, 10, 20, 2, 30).timestamp())
        );
    }

    #[test]
    fn whichever_rule_comes_first_wins() {
        let now = la(2026, 10, 19, 12, 0);
        let last = la(2026, 10, 19, 11, 0).timestamp();
        let both = Schedule {
            enabled: true,
            every: Some(SignedDuration::from_hours(6)),
            daily_at: Some(time(13, 0, 0, 0)),
        };
        assert_eq!(
            next_run(&both, Some(last), &now),
            Some(la(2026, 10, 19, 13, 0).timestamp())
        );
        let last = la(2026, 10, 19, 6, 0).timestamp();
        assert_eq!(
            next_run(&both, Some(last), &now),
            Some(la(2026, 10, 19, 12, 0).timestamp())
        );
    }

    #[test]
    fn daily_times_follow_daylight_saving() {
        // US DST ends 2026-11-01: 02:00 local is 09:00Z before and 10:00Z after.
        let now = la(2026, 11, 1, 12, 0);
        let last = la(2026, 10, 31, 2, 0).timestamp();
        let next = next_run(&daily(2, 0), Some(last), &now).unwrap();
        assert_eq!(next, la(2026, 11, 1, 2, 0).timestamp());
        assert_eq!(next.duration_since(last), SignedDuration::from_hours(25));
        // Spring forward 2027-03-14: 02:30 does not exist; it runs at 03:30.
        let now = la(2027, 3, 14, 12, 0);
        let last = la(2027, 3, 13, 2, 30).timestamp();
        let next = next_run(&daily(2, 30), Some(last), &now).unwrap();
        assert_eq!(next, la(2027, 3, 14, 3, 30).timestamp());
    }
}
