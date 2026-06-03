//! The `GraphSource` trait abstracts where follow lists and profiles come from,
//! so the resolver can run against live relays, an in-memory cache, or fixtures.

use async_trait::async_trait;
use serde::Serialize;

/// A user's follow list (kind 3) together with provenance.
#[derive(Debug, Clone, Serialize)]
pub struct ContactList {
    /// Hex pubkey of the follower this list belongs to.
    pub owner: String,
    /// Id of the kind-3 event backing this list.
    pub event_id: String,
    /// Relays on which this event was observed.
    pub relays: Vec<String>,
    /// `created_at` of the event (for freshness reasoning).
    pub created_at: i64,
    /// Hex pubkeys this user follows.
    pub follows: Vec<String>,
}

/// Profile metadata (kind 0), trimmed to what the dashboard needs.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Profile {
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub picture: Option<String>,
    pub nip05: Option<String>,
    pub about: Option<String>,
}

#[async_trait]
pub trait GraphSource: Send + Sync {
    /// Fetch the latest contact list for `pubkey_hex`, if any exists.
    async fn contacts(&self, pubkey_hex: &str) -> anyhow::Result<Option<ContactList>>;

    /// Fetch the latest profile metadata for `pubkey_hex`, if any exists.
    async fn profile(&self, pubkey_hex: &str) -> anyhow::Result<Option<Profile>>;
}

/// Parse kind-0 content (a JSON object) into a `Profile`.
pub fn parse_profile_content(content: &str) -> Profile {
    let value: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return Profile::default(),
    };
    let s = |k: &str| value.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
    Profile {
        name: s("name"),
        display_name: s("display_name").or_else(|| s("displayName")),
        picture: s("picture"),
        nip05: s("nip05"),
        about: s("about"),
    }
}
