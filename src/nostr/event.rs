//! Nostr event model (NIP-01) plus id computation and schnorr verification.

use secp256k1::schnorr::Signature;
use secp256k1::{Secp256k1, XOnlyPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const KIND_METADATA: u32 = 0;
pub const KIND_CONTACTS: u32 = 3;

/// A signed Nostr event as delivered by relays.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub pubkey: String,
    pub created_at: i64,
    pub kind: u32,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

impl Event {
    /// Compute the canonical event id per NIP-01:
    /// sha256 of the compact JSON array `[0, pubkey, created_at, kind, tags, content]`.
    pub fn computed_id(&self) -> String {
        let serialized = serde_json::json!([
            0,
            self.pubkey,
            self.created_at,
            self.kind,
            self.tags,
            self.content,
        ]);
        let bytes = serde_json::to_vec(&serialized).expect("event serialization never fails");
        let digest = Sha256::digest(&bytes);
        hex::encode(digest)
    }

    /// Verify the id matches the content and that the schnorr signature is valid.
    pub fn verify(&self) -> bool {
        if self.computed_id() != self.id {
            return false;
        }
        let id_bytes = match hex::decode(&self.id) {
            Ok(b) => b,
            Err(_) => return false,
        };
        let sig = match hex::decode(&self.sig)
            .ok()
            .and_then(|b| Signature::from_slice(&b).ok())
        {
            Some(s) => s,
            None => return false,
        };
        let pk = match hex::decode(&self.pubkey)
            .ok()
            .and_then(|b| XOnlyPublicKey::from_slice(&b).ok())
        {
            Some(p) => p,
            None => return false,
        };
        let secp = Secp256k1::verification_only();
        secp.verify_schnorr(&sig, &id_bytes, &pk).is_ok()
    }

    /// Lowercase-hex pubkeys referenced by `p` tags (the follow list for kind 3).
    pub fn referenced_pubkeys(&self) -> Vec<String> {
        self.tags
            .iter()
            .filter(|t| t.len() >= 2 && t[0] == "p")
            .filter_map(|t| normalize_pubkey(&t[1]))
            .collect()
    }
}

/// Keep only well-formed 64-char hex pubkeys, lowercased.
fn normalize_pubkey(s: &str) -> Option<String> {
    let s = s.trim().to_ascii_lowercase();
    if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(s)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a real, self-consistent signed event so the round-trip is genuine.
    fn signed_event(content: &str) -> Event {
        use secp256k1::{Keypair, Secp256k1, SecretKey};
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[7u8; 32]).unwrap();
        let kp = Keypair::from_secret_key(&secp, &sk);
        let (xonly, _) = kp.x_only_public_key();

        let mut ev = Event {
            id: String::new(),
            pubkey: hex::encode(xonly.serialize()),
            created_at: 1_700_000_000,
            kind: KIND_CONTACTS,
            tags: vec![vec!["p".into(), "ab".repeat(32)]],
            content: content.to_string(),
            sig: String::new(),
        };
        ev.id = ev.computed_id();
        let id_bytes = hex::decode(&ev.id).unwrap();
        let sig = secp.sign_schnorr_no_aux_rand(&id_bytes, &kp);
        ev.sig = hex::encode(sig.to_byte_array());
        ev
    }

    #[test]
    fn computes_and_verifies_signed_event() {
        // Content with non-ASCII to exercise UTF-8 id computation.
        let ev = signed_event("Walled gardens were beginning to look really passé");
        assert_eq!(ev.computed_id(), ev.id);
        assert!(ev.verify(), "freshly signed event should verify");
    }

    #[test]
    fn tampered_event_fails() {
        let mut ev = signed_event("original");
        ev.content = "tampered".to_string(); // id no longer matches content
        assert!(!ev.verify(), "tampered event must not verify");
    }

    #[test]
    fn tampered_signature_fails() {
        let mut ev = signed_event("original");
        // Flip the id back to a recomputed-but-unsigned state: keep id valid for
        // content, but corrupt the signature.
        ev.sig.replace_range(0..2, "00");
        assert!(!ev.verify(), "bad signature must not verify");
    }

    #[test]
    fn extracts_p_tags() {
        let ev = Event {
            id: String::new(),
            pubkey: "a".repeat(64),
            created_at: 0,
            kind: KIND_CONTACTS,
            tags: vec![
                vec!["p".into(), "b".repeat(64)],
                vec!["e".into(), "c".repeat(64)],
                vec!["p".into(), "BAD".into()],
                vec!["p".into(), "d".repeat(64), "wss://relay".into()],
            ],
            content: String::new(),
            sig: String::new(),
        };
        let refs = ev.referenced_pubkeys();
        assert_eq!(refs, vec!["b".repeat(64), "d".repeat(64)]);
    }
}
