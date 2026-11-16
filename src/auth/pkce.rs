//! PKCE (RFC 7636) verifier and S256 challenge, plus opaque random state.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Self {
        let verifier = random_token();
        let challenge = challenge_for(&verifier);
        Self { verifier, challenge }
    }
}

/// 32 random bytes, base64url without padding.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS random source is unavailable");
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_matches_rfc7636_vector() {
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn generated_pairs_are_related_and_unique() {
        let a = Pkce::generate();
        let b = Pkce::generate();
        assert_eq!(a.challenge, challenge_for(&a.verifier));
        assert_ne!(a.verifier, b.verifier);
        assert_eq!(a.verifier.len(), 43);
    }
}
