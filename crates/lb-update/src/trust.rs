//! What makes a release trustworthy: `SHA256SUMS`, signed with minisign.
//!
//! Every release carries `SHA256SUMS` (one line per asset, as `sha256sum` writes
//! it) and `SHA256SUMS.minisig`, a minisign signature of it made in CI with the
//! secret key in the repository secret `MINISIGN_SECRET_KEY`. Its *trusted comment*
//! (which minisign signs too) is exactly `librebooks <tag>`, e.g.
//! `librebooks v0.3.0`.
//!
//! The updater trusts nothing else. The GitHub API says which releases exist and
//! where their files are, but a release is only believed once its sums verify with
//! the public key compiled into this binary and the signed comment names the tag
//! the API said it was. That last check stops an old, genuinely signed release
//! being replayed under a newer tag. Each download is then checked against its
//! line in the sums.
//!
//! What this protects against: tampering anywhere between CI and this machine
//! (GitHub's storage, its CDN, a proxy, a compromised mirror). What it does not:
//! someone who can push a tag and run the release workflow with the secret, i.e.
//! a compromise of the repository itself.

use minisign_verify::{PublicKey, Signature};

/// The checksums file of a release.
pub const SUMS: &str = "SHA256SUMS";
/// Its signature.
pub const SIGNATURE: &str = "SHA256SUMS.minisig";
/// What the trusted comment says before the tag.
pub const COMMENT_PREFIX: &str = "librebooks ";

/// The production release key: the text of `minisign.pub` (both lines), or `None`
/// until one is made.
///
/// ┌──────────────────────────────────────────────────────────────────────────┐
/// │ PLACEHOLDER. There is no production key yet. Until there is, self-update  │
/// │ refuses to trust any release (and says so). To create it, follow          │
/// │ "Release signing key" in plans/self-update.md, then paste minisign.pub's  │
/// │ contents here as `Some("untrusted comment: …\nRW…")`.                       │
/// └──────────────────────────────────────────────────────────────────────────┘
pub const PRODUCTION_PUBLIC_KEY: Option<&str> = Some(include_str!("../../../minisign.pub"));

/// The key this build trusts: `LB_UPDATE_PUBLIC_KEY` at build time (for testing a
/// whole update against a throwaway key; it only affects a binary built with it
/// set), otherwise [`PRODUCTION_PUBLIC_KEY`].
pub const BUILD_PUBLIC_KEY: Option<&str> = match option_env!("LB_UPDATE_PUBLIC_KEY") {
    Some(key) => Some(key),
    None => PRODUCTION_PUBLIC_KEY,
};

/// Why a release could not be trusted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TrustError {
    #[error("this build has no release signing key, so it cannot verify updates")]
    NoKey,
    #[error("the release signing key is malformed")]
    Key,
    #[error("the release's signature file is malformed")]
    SignatureFile,
    #[error("the release is signed with a different key")]
    WrongKey,
    #[error("the release's checksums do not match their signature")]
    BadSignature,
    #[error("the release's signed comment {0:?} is not \"librebooks <tag>\"")]
    Comment(String),
    #[error("the signature is for {signed}, but the release claims to be {claimed}")]
    TagMismatch { signed: String, claimed: String },
    #[error("SHA256SUMS line {0} is malformed")]
    Sums(usize),
}

/// A minisign public key.
#[derive(Debug, Clone)]
pub struct TrustedKey(PublicKey);

impl TrustedKey {
    /// From a `minisign.pub` file's text, or just its base64 line.
    ///
    /// # Errors
    /// [`TrustError::Key`] if it does not decode.
    pub fn decode(text: &str) -> Result<TrustedKey, TrustError> {
        PublicKey::decode(text)
            .or_else(|_| PublicKey::from_base64(text.trim()))
            .map(TrustedKey)
            .map_err(|_| TrustError::Key)
    }

    /// The key compiled into this build.
    ///
    /// # Errors
    /// [`TrustError::NoKey`] while there is no production key.
    pub fn built_in() -> Result<TrustedKey, TrustError> {
        TrustedKey::decode(BUILD_PUBLIC_KEY.ok_or(TrustError::NoKey)?)
    }

    /// Check `sums` against `signature` and that the signature is for `tag`;
    /// return the checksums.
    ///
    /// # Errors
    /// [`TrustError`] if the signature is not by this key, does not cover these
    /// exact bytes, names another tag, or the sums do not parse.
    pub fn verify(
        &self,
        signature: &str,
        sums: &[u8],
        tag: &str,
    ) -> Result<SignedSums, TrustError> {
        let sig = Signature::decode(signature).map_err(|_| TrustError::SignatureFile)?;
        self.0.verify(sums, &sig, false).map_err(|e| match e {
            minisign_verify::Error::UnexpectedKeyId => TrustError::WrongKey,
            _ => TrustError::BadSignature,
        })?;
        // Verified together with the signature above, so this is authentic now.
        let comment = sig.trusted_comment().trim();
        let signed_tag = comment
            .strip_prefix(COMMENT_PREFIX)
            .filter(|t| t.starts_with('v') && !t.contains(char::is_whitespace))
            .ok_or_else(|| TrustError::Comment(comment.to_owned()))?;
        if signed_tag != tag {
            return Err(TrustError::TagMismatch {
                signed: signed_tag.to_owned(),
                claimed: tag.to_owned(),
            });
        }
        Ok(SignedSums {
            tag: tag.to_owned(),
            assets: parse_sums(sums)?,
        })
    }
}

/// A release's checksums, signature verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedSums {
    pub tag: String,
    pub assets: Vec<SumEntry>,
}

impl SignedSums {
    pub fn get(&self, name: &str) -> Option<&SumEntry> {
        self.assets.iter().find(|a| a.name == name)
    }
}

/// One line of `SHA256SUMS`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SumEntry {
    pub name: String,
    pub sha256: [u8; 32],
}

/// `sha256sum` output: 64 hex digits, a space, then a space (text mode) or `*`
/// (binary mode), then the name. Blank lines are allowed; nothing else is.
pub fn parse_sums(sums: &[u8]) -> Result<Vec<SumEntry>, TrustError> {
    let text = std::str::from_utf8(sums).map_err(|_| TrustError::Sums(1))?;
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let bad = || TrustError::Sums(i + 1);
        if line.trim().is_empty() {
            continue;
        }
        let (hex, rest) = line.split_once(' ').ok_or_else(bad)?;
        let name = rest
            .strip_prefix(' ')
            .or_else(|| rest.strip_prefix('*'))
            .ok_or_else(bad)?;
        let mut sha256 = [0u8; 32];
        hex::decode_to_slice(hex, &mut sha256).map_err(|_| bad())?;
        if name.is_empty() || name.contains(['/', '\\']) {
            return Err(bad());
        }
        out.push(SumEntry {
            name: name.to_owned(),
            sha256,
        });
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    pub const TEST_KEY: &str = include_str!("../testdata/test.pub");
    pub const SUMS_FILE: &[u8] = include_bytes!("../testdata/SHA256SUMS");
    pub const SIG: &str = include_str!("../testdata/SHA256SUMS.minisig");
    pub const LINUX_TGZ: &[u8] =
        include_bytes!("../testdata/librebooks-v0.3.0-x86_64-unknown-linux-gnu.tar.gz");

    fn key() -> TrustedKey {
        TrustedKey::decode(TEST_KEY).unwrap()
    }

    #[test]
    fn a_signed_release_verifies_and_lists_its_assets() {
        let s = key().verify(SIG, SUMS_FILE, "v0.3.0").unwrap();
        assert_eq!(s.tag, "v0.3.0");
        assert_eq!(s.assets.len(), 2);
        let tgz = s
            .get("librebooks-v0.3.0-x86_64-unknown-linux-gnu.tar.gz")
            .unwrap();
        let digest: [u8; 32] = Sha256::digest(LINUX_TGZ).into();
        assert_eq!(tgz.sha256, digest);
        assert!(
            s.get("librebooks-v0.3.0-x86_64-pc-windows-msvc.zip")
                .is_some()
        );
    }

    #[test]
    fn line_endings_do_not_matter_for_the_signature_file_or_the_key() {
        // The fixtures were signed on Windows (CRLF); CI signs on Linux (LF).
        let lf_sig = SIG.replace("\r\n", "\n");
        let lf_key = TEST_KEY.replace("\r\n", "\n");
        let k = TrustedKey::decode(&lf_key).unwrap();
        assert!(k.verify(&lf_sig, SUMS_FILE, "v0.3.0").is_ok());
        let b64 = TEST_KEY.lines().nth(1).unwrap();
        assert!(
            TrustedKey::decode(b64)
                .unwrap()
                .verify(SIG, SUMS_FILE, "v0.3.0")
                .is_ok()
        );
    }

    #[test]
    fn tampering_is_refused() {
        let mut tampered = SUMS_FILE.to_vec();
        tampered[0] ^= 1;
        assert_eq!(
            key().verify(SIG, &tampered, "v0.3.0"),
            Err(TrustError::BadSignature)
        );
        // The trusted comment is signed too: editing it breaks the signature.
        let edited = SIG.replace("librebooks v0.3.0", "librebooks v9.3.0");
        assert_eq!(
            key().verify(&edited, SUMS_FILE, "v9.3.0"),
            Err(TrustError::BadSignature)
        );
        assert_eq!(
            key().verify("not a signature", SUMS_FILE, "v0.3.0"),
            Err(TrustError::SignatureFile)
        );
    }

    #[test]
    fn another_key_is_refused() {
        let other = include_str!("../testdata/other-key.minisig");
        assert_eq!(
            key().verify(other, SUMS_FILE, "v0.3.0"),
            Err(TrustError::WrongKey)
        );
        assert_eq!(TrustedKey::decode("nope").unwrap_err(), TrustError::Key);
    }

    #[test]
    fn the_comment_must_name_the_tag() {
        // Properly signed, but not "librebooks <tag>".
        let wrong = include_str!("../testdata/wrong-comment.minisig");
        assert_eq!(
            key().verify(wrong, SUMS_FILE, "v0.3.0"),
            Err(TrustError::Comment("librebooks 0.3.0".into()))
        );
        // Properly signed for v0.2.9, presented as v0.3.0: a replay.
        let replay = include_str!("../testdata/replayed-tag.minisig");
        assert_eq!(
            key().verify(replay, SUMS_FILE, "v0.3.0"),
            Err(TrustError::TagMismatch {
                signed: "v0.2.9".into(),
                claimed: "v0.3.0".into()
            })
        );
        // And the real one, presented as something else.
        assert!(matches!(
            key().verify(SIG, SUMS_FILE, "v0.3.1"),
            Err(TrustError::TagMismatch { .. })
        ));
    }

    #[test]
    fn sums_parse_as_sha256sum_writes_them() {
        let hash = "ab".repeat(32);
        let text = format!("{hash}  a.zip\n{hash} *b.tar.gz\r\n\n");
        let assets = parse_sums(text.as_bytes()).unwrap();
        assert_eq!(
            assets.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            ["a.zip", "b.tar.gz"]
        );
        assert_eq!(assets[0].sha256, [0xAB; 32]);
        for bad in [
            format!("{hash} a.zip"),
            format!("{}  a.zip", &hash[1..]),
            format!("{}  a.zip", "zz".repeat(32)),
            format!("{hash}  dir/a.zip"),
            format!("{hash}  "),
        ] {
            assert!(parse_sums(bad.as_bytes()).is_err(), "{bad:?} parsed");
        }
    }

    #[test]
    fn without_a_production_key_nothing_is_trusted() {
        if BUILD_PUBLIC_KEY.is_none() {
            assert_eq!(TrustedKey::built_in().unwrap_err(), TrustError::NoKey);
        } else {
            assert!(TrustedKey::built_in().is_ok(), "the built-in key decodes");
        }
    }
}
