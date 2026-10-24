//! OpenPGP verification of apt InRelease clearsigned data (RFC 4880 signed
//! cleartext, SHA-256 hash). Mirrors apt's trust model: verification succeeds
//! when any one embedded trusted key validates the signature.

use pgp::composed::{CleartextSignedMessage, SignedPublicKey};
use std::time::SystemTime;

pub fn verify_inrelease(
    armored: &str,
    keys: &[SignedPublicKey],
    _at: SystemTime,
) -> Result<(), String> {
    let (msg, _headers) = CleartextSignedMessage::from_string(armored)
        .map_err(|e| format!("clearsigned parse: {e}"))?;
    for key in keys {
        match msg.verify(key) {
            Ok(_sig) => return Ok(()),
            Err(_) => continue,
        }
    }
    Err(format!(
        "no trusted key (of {}) verified the InRelease signature",
        keys.len()
    ))
}

/// System time for signature `at` checks. Exposed so callers can pass a fixed
/// time in tests.
pub fn now() -> SystemTime {
    SystemTime::now()
}
