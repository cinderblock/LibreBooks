//! Sign-in ceremonies in flight, kept in memory.
//!
//! A ceremony is the server half of a challenge–response: the WebAuthn state
//! between `…/start` and `…/finish`, or a device-login challenge. They live for
//! minutes and are used once, so memory is the right place: taking one out of
//! the map is what makes it single use, and a server restart costs nothing but
//! a "please try again". (`webauthn_rp` keeps its server state unserialisable
//! by default for the same reasons.) The map is capped so unauthenticated
//! callers can't grow it without bound; expired entries are swept on insert.
//!
//! The cost: every request of one ceremony must reach the same server process.
//! LibreBooks runs as one process, so that holds.

use std::collections::HashMap;
use std::sync::Mutex;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use webauthn_rp::{DiscoverableAuthenticationServerState, RegistrationServerState};

use crate::error::{AuthError, Result};
use crate::model::EnrolmentPurpose;
use crate::secret;

/// Names one ceremony in flight. Random, so ceremonies can't be guessed or
/// enumerated.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(transparent)]
#[ts(export, type = "string")]
pub struct ChallengeId(pub String);

impl ChallengeId {
    fn new() -> ChallengeId {
        ChallengeId(secret::b64(&secret::random_bytes::<16>()))
    }
}

impl std::fmt::Display for ChallengeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The server's half of a ceremony.
#[derive(Debug)]
pub(crate) enum Ceremony {
    /// A passkey being registered with an enrolment token.
    Register {
        state: RegistrationServerState<16>,
        /// The token the ceremony was started with; finishing must present the
        /// same one.
        token_hash: [u8; 32],
        purpose: EnrolmentPurpose,
        /// Index into the store's relying parties.
        rp: usize,
        /// The user the passkey will belong to (new or existing).
        user_id: lb_core::UserId,
        /// The name the person gave at the start, if any (new users).
        display_name: Option<String>,
    },
    /// A passkey sign-in.
    Login {
        state: DiscoverableAuthenticationServerState,
        rp: usize,
    },
    /// A device-key sign-in.
    Device { challenge: [u8; 32] },
}

struct Entry {
    ceremony: Ceremony,
    expires: Timestamp,
}

pub(crate) struct Ceremonies {
    max: usize,
    map: Mutex<HashMap<ChallengeId, Entry>>,
}

impl Ceremonies {
    pub fn new(max: usize) -> Ceremonies {
        Ceremonies {
            max,
            map: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ChallengeId, Entry>> {
        self.map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Remember a ceremony until `expires`.
    pub fn insert(
        &self,
        ceremony: Ceremony,
        now: Timestamp,
        expires: Timestamp,
    ) -> Result<ChallengeId> {
        let mut map = self.lock();
        if map.len() >= self.max {
            map.retain(|_, e| e.expires > now);
            if map.len() >= self.max {
                return Err(AuthError::Busy);
            }
        }
        let id = ChallengeId::new();
        map.insert(id.clone(), Entry { ceremony, expires });
        Ok(id)
    }

    /// Take a ceremony out (so it can't be used again). `None` if there is no
    /// such ceremony or it has expired.
    pub fn take(&self, id: &ChallengeId, now: Timestamp) -> Option<Ceremony> {
        self.take_entry(id, now).map(|(c, _)| c)
    }

    /// [`Ceremonies::take`], with the ceremony's expiry so it can be put back.
    pub fn take_entry(&self, id: &ChallengeId, now: Timestamp) -> Option<(Ceremony, Timestamp)> {
        let entry = self.lock().remove(id)?;
        (entry.expires > now).then_some((entry.ceremony, entry.expires))
    }

    /// Return a ceremony taken with [`Ceremonies::take_entry`], when the
    /// request failed for a reason the person can fix and retry (a missing
    /// name), so they don't have to repeat the authenticator prompt.
    pub fn put_back(&self, id: ChallengeId, ceremony: Ceremony, expires: Timestamp) {
        self.lock().insert(id, Entry { ceremony, expires });
    }

    pub fn prune(&self, now: Timestamp) {
        self.lock().retain(|_, e| e.expires > now);
    }
}
