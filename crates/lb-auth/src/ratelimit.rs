//! Failed-attempt rate limiting, per client, in memory.
//!
//! Tokens and keys are far too large to guess, so this is defence in depth
//! and a brake on noise: after `max_failures` failures inside `window`, a
//! client (keyed by IP) is refused until its oldest failure ages out. Only
//! failures count, so a busy office behind one NAT address isn't throttled
//! for signing in successfully.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use jiff::Timestamp;

use crate::clock::{self, dur_ms, ms};
use crate::config::RateLimit;
use crate::error::{AuthError, Result};

pub(crate) struct RateLimiter {
    limit: RateLimit,
    failures: Mutex<HashMap<String, VecDeque<i64>>>,
}

/// More keys than this and every insert sweeps out the idle ones.
const SWEEP_AT: usize = 10_000;

impl RateLimiter {
    pub fn new(limit: RateLimit) -> RateLimiter {
        RateLimiter {
            limit,
            failures: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, VecDeque<i64>>> {
        self.failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Refuse if `key` has used up its failures.
    pub fn check(&self, key: &str, now: Timestamp) -> Result<()> {
        let now = ms(now);
        let window = dur_ms(self.limit.window);
        let mut map = self.lock();
        let Some(q) = map.get_mut(key) else {
            return Ok(());
        };
        while q.front().is_some_and(|&t| t <= now - window) {
            q.pop_front();
        }
        if q.len() >= self.limit.max_failures as usize {
            let oldest = q.front().copied().unwrap_or(now);
            let wait = (oldest + window - now).max(1);
            return Err(AuthError::RateLimited {
                retry_after: Duration::from_millis(wait as u64),
            });
        }
        if q.is_empty() {
            map.remove(key);
        }
        Ok(())
    }

    /// Count a failure against `key`.
    pub fn fail(&self, key: &str, now: Timestamp) {
        let mut map = self.lock();
        if map.len() >= SWEEP_AT {
            let cutoff = ms(now) - dur_ms(self.limit.window);
            map.retain(|_, q| q.back().is_some_and(|&t| t > cutoff));
        }
        let q = map.entry(key.to_owned()).or_default();
        q.push_back(ms(now));
        // Never keep more than needed to decide.
        while q.len() > self.limit.max_failures as usize {
            q.pop_front();
        }
    }

    pub fn prune(&self, now: Timestamp) {
        let cutoff = clock::ms(now) - dur_ms(self.limit.window);
        self.lock()
            .retain(|_, q| q.back().is_some_and(|&t| t > cutoff));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_max_failures_until_the_window_passes() {
        let rl = RateLimiter::new(RateLimit {
            max_failures: 3,
            window: Duration::from_secs(60),
        });
        let t0 = Timestamp::from_second(1_000_000).unwrap();
        for _ in 0..3 {
            rl.check("1.2.3.4", t0).unwrap();
            rl.fail("1.2.3.4", t0);
        }
        let err = rl.check("1.2.3.4", t0).unwrap_err();
        assert!(
            matches!(err, AuthError::RateLimited { retry_after } if retry_after == Duration::from_secs(60))
        );
        // Another client is unaffected.
        rl.check("5.6.7.8", t0).unwrap();
        // Once the failures age out, the client may try again.
        let later = clock::add(t0, Duration::from_secs(61));
        rl.check("1.2.3.4", later).unwrap();
    }
}
