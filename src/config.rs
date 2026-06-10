//! Server configuration, loaded from a TOML file with sensible defaults.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Address to bind the HTTP server to.
    pub bind: String,
    /// Nostr relays to query.
    pub relays: Vec<String>,
    /// Per-relay request timeout, in milliseconds.
    pub relay_timeout_ms: u64,
    /// Cache freshness window, in seconds.
    pub cache_ttl_secs: u64,
    /// Max distinct pubkeys to cache (per kind).
    pub cache_capacity: u64,
    /// Verify schnorr signatures on fetched events.
    pub verify_signatures: bool,
    /// Default max BFS depth for resolution.
    pub max_depth: usize,
    /// Max follows inspected per namespace during name resolution.
    pub max_fanout: usize,
    /// Max alternate paths carried forward when a label is ambiguous.
    pub max_name_paths: usize,
    /// Max kind-3 events requested per relay for a reverse-follower query.
    pub follower_query_limit: u32,
    /// Directory of static dashboard assets.
    pub static_dir: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bind: "127.0.0.1:8080".to_string(),
            relays: vec![
                "wss://relay.damus.io".to_string(),
                "wss://nos.lol".to_string(),
                "wss://relay.nostr.band".to_string(),
                "wss://relay.primal.net".to_string(),
            ],
            relay_timeout_ms: 5_000,
            cache_ttl_secs: 300,
            cache_capacity: 100_000,
            verify_signatures: true,
            max_depth: 6,
            max_fanout: 5000,
            max_name_paths: 16,
            follower_query_limit: 500,
            static_dir: "static".to_string(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let cfg: Config = toml::from_str(&text)?;
        Ok(cfg)
    }

    pub fn bind_addr(&self) -> anyhow::Result<SocketAddr> {
        Ok(self.bind.parse()?)
    }

    pub fn relay_timeout(&self) -> Duration {
        Duration::from_millis(self.relay_timeout_ms)
    }

    pub fn cache_ttl(&self) -> Duration {
        Duration::from_secs(self.cache_ttl_secs)
    }
}
