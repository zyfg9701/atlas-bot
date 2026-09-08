//! PKCE (RFC 7636) S256 helpers.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::Rng;
use sha2::{Digest, Sha256};

/// Characters allowed in a code_verifier (RFC 7636 Appendix B).
const VERIFIER_ALPHABET: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";

#[derive(Debug, Clone)]
pub struct PkcePair {
    pub verifier: String,
    pub challenge: String,
    pub method: &'static str,
}

impl PkcePair {
    /// Generate a random S256 PKCE pair (verifier length 64).
    pub fn generate() -> Self {
        Self::from_verifier(random_verifier(64))
    }

    /// Build challenge from an existing verifier (tests / replay).
    pub fn from_verifier(verifier: impl Into<String>) -> Self {
        let verifier = verifier.into();
        let challenge = s256_challenge(&verifier);
        Self {
            verifier,
            challenge,
            method: "S256",
        }
    }
}

pub fn random_verifier(len: usize) -> String {
    let len = len.clamp(43, 128);
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| {
            let i = rng.gen_range(0..VERIFIER_ALPHABET.len());
            VERIFIER_ALPHABET[i] as char
        })
        .collect()
}

pub fn s256_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_shape_s256_base64url_no_pad() {
        // RFC 7636 appendix B example.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = s256_challenge(verifier);
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert!(!challenge.contains('='));
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
        assert!(challenge
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn generate_pair_shape() {
        let p = PkcePair::generate();
        assert_eq!(p.method, "S256");
        assert!(p.verifier.len() >= 43 && p.verifier.len() <= 128);
        assert_eq!(p.challenge, s256_challenge(&p.verifier));
        assert_eq!(p.challenge.len(), 43); // SHA-256 → 32 bytes → 43 base64url chars
    }
}
