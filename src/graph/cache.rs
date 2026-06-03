//! A freshness-aware cache wrapping any `GraphSource`.
//!
//! Contact lists and profiles are cached with a TTL ("freshness window").
//! Within the window we serve cached data; past it we refresh from the inner
//! source. Popular paths are looked up more often and therefore stay fresher —
//! the public-good dynamic described in the GNS vision.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use moka::future::Cache;

use super::source::{ContactList, GraphSource, Profile};

#[derive(Clone)]
pub struct CachedSource {
    inner: Arc<dyn GraphSource>,
    // moka caches `Option` so we also remember "no event exists" for the TTL.
    contacts: Cache<String, Arc<Option<ContactList>>>,
    profiles: Cache<String, Arc<Option<Profile>>>,
}

impl CachedSource {
    pub fn new(inner: Arc<dyn GraphSource>, ttl: Duration, capacity: u64) -> Self {
        CachedSource {
            inner,
            contacts: Cache::builder()
                .max_capacity(capacity)
                .time_to_live(ttl)
                .build(),
            profiles: Cache::builder()
                .max_capacity(capacity)
                .time_to_live(ttl)
                .build(),
        }
    }
}

#[async_trait]
impl GraphSource for CachedSource {
    async fn contacts(&self, pubkey_hex: &str) -> anyhow::Result<Option<ContactList>> {
        if let Some(hit) = self.contacts.get(pubkey_hex).await {
            return Ok((*hit).clone());
        }
        let fetched = self.inner.contacts(pubkey_hex).await?;
        self.contacts
            .insert(pubkey_hex.to_string(), Arc::new(fetched.clone()))
            .await;
        Ok(fetched)
    }

    async fn profile(&self, pubkey_hex: &str) -> anyhow::Result<Option<Profile>> {
        if let Some(hit) = self.profiles.get(pubkey_hex).await {
            return Ok((*hit).clone());
        }
        let fetched = self.inner.profile(pubkey_hex).await?;
        self.profiles
            .insert(pubkey_hex.to_string(), Arc::new(fetched.clone()))
            .await;
        Ok(fetched)
    }
}
