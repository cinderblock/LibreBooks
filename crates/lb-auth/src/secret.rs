//! Random secrets, their hashes, and comparing them safely.
//!
//! Tokens are high-entropy random strings (160 bits for enrolment, 256 for
//! sessions), so a plain SHA-256 is enough to store them: there is nothing to
//! brute-force, and so nothing for a salt or slow KDF to protect. Looking a
//! token up by its hash leaks nothing useful through timing either, since an
//! attacker who can't invert SHA-256 can't steer which hash they probe.
//! Where we compare a presented secret with one we remember, we compare in
//! constant time anyway.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::TryRng as _;
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;

/// Prefix of enrolment tokens, so they're recognisable in a log or a paste
/// (and to secret scanners).
pub const ENROLMENT_PREFIX: &str = "lbe_";
/// Prefix of session tokens.
pub const SESSION_PREFIX: &str = "lbs_";

/// `N` bytes from the operating system's random number generator.
///
/// # Panics
///
/// If the OS can't supply randomness; nothing safe can be done without it.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    rand::rngs::SysRng
        .try_fill_bytes(&mut out)
        .expect("the operating system's random number generator failed");
    out
}

/// A fresh token: `prefix` + base64url of `N` random bytes.
pub fn new_token<const N: usize>(prefix: &str) -> String {
    format!("{prefix}{}", b64(&random_bytes::<N>()))
}

/// SHA-256 of a token as presented (surrounding whitespace ignored, since
/// people paste).
pub fn hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.trim().as_bytes()).into()
}

/// Constant-time equality of two hashes.
pub fn same(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.ct_eq(b).into()
}

pub fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Decode base64url, with or without padding.
pub fn unb64(s: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(s.trim().trim_end_matches('=')).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_prefixed_long_and_distinct() {
        let a = new_token::<20>(ENROLMENT_PREFIX);
        let b = new_token::<20>(ENROLMENT_PREFIX);
        assert!(a.starts_with("lbe_"));
        assert_eq!(a.len(), 4 + 27);
        assert_ne!(a, b);
        assert_eq!(new_token::<32>(SESSION_PREFIX).len(), 4 + 43);
    }

    #[test]
    fn hash_ignores_surrounding_whitespace() {
        assert!(same(&hash(" lbe_x\n"), &hash("lbe_x")));
        assert!(!same(&hash("lbe_x"), &hash("lbe_y")));
    }

    #[test]
    fn base64_round_trips_with_or_without_padding() {
        assert_eq!(unb64(&b64(b"ab")).unwrap(), b"ab");
        assert_eq!(unb64("YWI=").unwrap(), b"ab");
        assert!(unb64("not base64!").is_none());
    }
}
