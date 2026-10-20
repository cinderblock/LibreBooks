//! Where "now" comes from.
//!
//! Expiry is the heart of auth (tokens, challenges, sessions), and testing it by
//! sleeping is slow and flaky. So every time check goes through a [`Clock`]: the
//! system clock in production, a hand-wound one in tests (this crate's and the
//! server's).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::Timestamp;

/// The source of the current time.
#[derive(Clone, Default)]
pub struct Clock {
    manual: Option<Arc<Mutex<Timestamp>>>,
}

impl std::fmt::Debug for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.manual {
            None => f.write_str("Clock::system"),
            Some(_) => write!(f, "Clock::manual({})", self.now()),
        }
    }
}

impl Clock {
    /// The real time.
    pub fn system() -> Clock {
        Clock { manual: None }
    }

    /// A clock stopped at `start` that moves only when [`Clock::advance`] is
    /// called. Clones share the same time.
    pub fn manual(start: Timestamp) -> Clock {
        Clock {
            manual: Some(Arc::new(Mutex::new(start))),
        }
    }

    pub fn now(&self) -> Timestamp {
        match &self.manual {
            None => Timestamp::now(),
            Some(t) => *t.lock().unwrap_or_else(std::sync::PoisonError::into_inner),
        }
    }

    /// Move a manual clock forward.
    ///
    /// # Panics
    ///
    /// On the system clock, which cannot be wound.
    pub fn advance(&self, by: Duration) {
        let t = self
            .manual
            .as_ref()
            .expect("Clock::advance only works on a manual clock");
        let mut t = t.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *t = add(*t, by);
    }
}

/// A timestamp as stored: Unix milliseconds.
pub(crate) fn ms(t: Timestamp) -> i64 {
    t.as_millisecond()
}

/// A stored time back to a timestamp. Out-of-range values (a corrupt row) clamp
/// rather than fail, since every use is a comparison or a display.
pub(crate) fn from_ms(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(ms).unwrap_or(if ms < 0 {
        Timestamp::MIN
    } else {
        Timestamp::MAX
    })
}

pub(crate) fn opt_from_ms(ms: Option<i64>) -> Option<Timestamp> {
    ms.map(from_ms)
}

/// A duration in milliseconds, saturating (configured durations are small).
pub(crate) fn dur_ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

/// `t + d`, saturating at the end of time.
pub(crate) fn add(t: Timestamp, d: Duration) -> Timestamp {
    from_ms(ms(t).saturating_add(dur_ms(d)))
}
