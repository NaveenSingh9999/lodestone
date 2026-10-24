//! Embedded Termux repository signing keys, sourced verbatim from the
//! `termux-keyring` apt package (Debian). These are public keys only; the
//! repository trust chain is: embedded key verifies InRelease -> InRelease
//! SHA256 lines verify the Packages index -> Packages SHA256 verifies the
//! packages themselves.

use pgp::composed::{Deserializable, SignedPublicKey};
use std::io::Cursor;

pub const TERMUX_KEYRING_AUTOBUILDS: &[u8] =
    include_bytes!("../../../assets/termux-keys/termux-autobuilds.gpg");
pub const TERMUX_KEYRING_GRIMLER: &[u8] = include_bytes!("../../../assets/termux-keys/grimler.gpg");

/// The set of keys Lodestone trusts for Termux apt repository data.
///
/// Active signer for `dists/*` since 2026 is the autobuilds key; the grimler
/// personal key remains embedded so historical dists remain verifiable.
pub fn embedded_public_keys() -> Result<Vec<SignedPublicKey>, pgp::errors::Error> {
    let mut keys = Vec::new();
    for (label, blob) in [
        ("autobuilds", TERMUX_KEYRING_AUTOBUILDS),
        ("grimler", TERMUX_KEYRING_GRIMLER),
    ] {
        let reader = Cursor::new(blob);
        let iter = SignedPublicKey::from_bytes_many(reader)?;
        for key in iter {
            let _ = label;
            keys.push(key?);
        }
    }
    Ok(keys)
}
