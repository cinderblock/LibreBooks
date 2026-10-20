//! How long to wait before restarting cloudflared.
//!
//! Doubling from one second to five minutes. A process that stayed connected for
//! a while before it stopped is treated as a fresh start (the delay resets), so a
//! tunnel that drops once a week restarts at once, while one that cannot start at
//! all (no network, a revoked token) settles into one attempt every five minutes
//! instead of hammering Cloudflare. No jitter: there is one connector per server.

use std::time::Duration;

/// The restart schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    pub initial: Duration,
    pub max: Duration,
    /// A run at least this long resets the delay.
    pub healthy_after: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Backoff {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(300),
            healthy_after: Duration::from_secs(60),
        }
    }
}

impl Backoff {
    /// The wait before restart number `attempt` (0 for the first restart after a
    /// healthy run).
    pub fn delay(&self, attempt: u32) -> Duration {
        let factor = 1u32.checked_shl(attempt.min(31)).unwrap_or(u32::MAX);
        self.initial.saturating_mul(factor).min(self.max)
    }

    /// The wait before the next start, given how long the run that just ended
    /// was healthy for; advances `attempt` (start it at 0).
    pub fn after_run(&self, attempt: &mut u32, healthy_for: Duration) -> Duration {
        if healthy_for >= self.healthy_after {
            *attempt = 0;
        }
        let wait = self.delay(*attempt);
        *attempt = attempt.saturating_add(1);
        wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_double_up_to_the_cap() {
        let b = Backoff::default();
        let secs: Vec<u64> = (0..12).map(|a| b.delay(a).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 32, 64, 128, 256, 300, 300, 300]);
        assert_eq!(b.delay(u32::MAX), Duration::from_secs(300));
    }

    #[test]
    fn a_healthy_run_resets_the_schedule() {
        let b = Backoff::default();
        let mut attempt = 0;
        let quick = Duration::from_secs(3);
        let waits: Vec<u64> = (0..4)
            .map(|_| b.after_run(&mut attempt, quick).as_secs())
            .collect();
        assert_eq!(waits, [1, 2, 4, 8]);
        // Connected for over a minute, then dropped: back to one second.
        assert_eq!(
            b.after_run(&mut attempt, Duration::from_secs(61)).as_secs(),
            1
        );
        assert_eq!(b.after_run(&mut attempt, quick).as_secs(), 2);
        let mut attempt = u32::MAX;
        assert_eq!(b.after_run(&mut attempt, quick), b.max);
        assert_eq!(attempt, u32::MAX);
    }

    #[test]
    fn custom_schedules() {
        let b = Backoff {
            initial: Duration::from_millis(50),
            max: Duration::from_millis(300),
            healthy_after: Duration::from_secs(1),
        };
        let ms: Vec<u128> = (0..5).map(|a| b.delay(a).as_millis()).collect();
        assert_eq!(ms, [50, 100, 200, 300, 300]);
    }
}
