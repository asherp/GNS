//! Nostr public-key handling: hex <-> npub (NIP-19 bech32) conversions.

use bech32::{Bech32, Hrp};
use thiserror::Error;

const NPUB_HRP: &str = "npub";

#[derive(Debug, Error)]
pub enum KeyError {
    #[error("invalid hex public key: {0}")]
    Hex(String),
    #[error("public key must be 32 bytes, got {0}")]
    Length(usize),
    #[error("invalid bech32: {0}")]
    Bech32(String),
    #[error("expected `npub` prefix, got `{0}`")]
    WrongHrp(String),
}

/// A 32-byte x-only Nostr public key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey([u8; 32]);

impl PublicKey {
    #[allow(dead_code)]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        PublicKey(bytes)
    }

    #[allow(dead_code)]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Parse from 64-char lowercase/uppercase hex.
    pub fn from_hex(s: &str) -> Result<Self, KeyError> {
        let bytes = hex::decode(s).map_err(|e| KeyError::Hex(e.to_string()))?;
        let arr: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| KeyError::Length(bytes.len()))?;
        Ok(PublicKey(arr))
    }

    /// Parse from an `npub1...` bech32 string.
    pub fn from_npub(s: &str) -> Result<Self, KeyError> {
        let (hrp, data) = bech32::decode(s).map_err(|e| KeyError::Bech32(e.to_string()))?;
        if hrp.as_str() != NPUB_HRP {
            return Err(KeyError::WrongHrp(hrp.to_string()));
        }
        let arr: [u8; 32] = data
            .as_slice()
            .try_into()
            .map_err(|_| KeyError::Length(data.len()))?;
        Ok(PublicKey(arr))
    }

    /// Accept either an `npub1...` string or raw hex.
    pub fn parse(s: &str) -> Result<Self, KeyError> {
        let s = s.trim();
        if s.starts_with("npub1") {
            Self::from_npub(s)
        } else {
            Self::from_hex(s)
        }
    }

    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    pub fn to_npub(self) -> String {
        let hrp = Hrp::parse(NPUB_HRP).expect("npub is a valid hrp");
        bech32::encode::<Bech32>(hrp, &self.0).expect("encoding 32 bytes never fails")
    }
}

impl std::fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PublicKey({})", self.to_npub())
    }
}

impl std::fmt::Display for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_npub())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // NIP-19 test vector.
    const HEX: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
    const NPUB: &str = "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6";

    #[test]
    fn hex_to_npub() {
        let pk = PublicKey::from_hex(HEX).unwrap();
        assert_eq!(pk.to_npub(), NPUB);
    }

    #[test]
    fn npub_to_hex() {
        let pk = PublicKey::from_npub(NPUB).unwrap();
        assert_eq!(pk.to_hex(), HEX);
    }

    #[test]
    fn round_trip() {
        let pk = PublicKey::from_hex(HEX).unwrap();
        assert_eq!(PublicKey::from_npub(&pk.to_npub()).unwrap(), pk);
    }

    #[test]
    fn parse_either_form() {
        assert_eq!(
            PublicKey::parse(HEX).unwrap(),
            PublicKey::parse(NPUB).unwrap()
        );
    }

    #[test]
    fn rejects_wrong_hrp() {
        // an nsec-prefixed string should be rejected by from_npub
        let err =
            PublicKey::from_npub("nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5");
        assert!(matches!(err, Err(KeyError::WrongHrp(_))));
    }
}
