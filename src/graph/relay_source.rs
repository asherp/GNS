//! A `GraphSource` backed by a configurable set of live Nostr relays.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::future::join_all;

use crate::nostr::client::query_relay;
use crate::nostr::event::{Event, KIND_CONTACTS, KIND_METADATA};

use super::source::{parse_profile_content, ContactList, GraphSource, Profile};

/// Fetches follow lists and profiles by fanning out across all configured relays.
pub struct RelaySource {
    relays: Vec<String>,
    request_timeout: Duration,
    /// Drop events that fail signature verification.
    verify: bool,
}

impl RelaySource {
    pub fn new(relays: Vec<String>, request_timeout: Duration, verify: bool) -> Self {
        RelaySource {
            relays,
            request_timeout,
            verify,
        }
    }

    /// Query every relay for the given author/kind and return the newest event
    /// together with the relays that served that exact event id.
    async fn newest_event(&self, pubkey_hex: &str, kind: u32) -> Option<(Event, Vec<String>)> {
        let authors = vec![pubkey_hex.to_string()];
        let queries = self.relays.iter().map(|relay| {
            let authors = authors.clone();
            async move {
                let events = query_relay(relay, &authors, kind, 4, self.request_timeout).await;
                (relay.clone(), events)
            }
        });
        let results = join_all(queries).await;

        // Collect the newest valid event per relay, keyed by event id.
        let mut newest: Option<Event> = None;
        let mut relays_by_id: HashMap<String, Vec<String>> = HashMap::new();

        for (relay, events) in results {
            for ev in events {
                if ev.pubkey != pubkey_hex || ev.kind != kind {
                    continue;
                }
                if self.verify && !ev.verify() {
                    continue;
                }
                relays_by_id
                    .entry(ev.id.clone())
                    .or_default()
                    .push(relay.clone());
                if newest
                    .as_ref()
                    .map(|n| ev.created_at > n.created_at)
                    .unwrap_or(true)
                {
                    newest = Some(ev);
                }
            }
        }

        let event = newest?;
        let mut relays = relays_by_id.remove(&event.id).unwrap_or_default();
        relays.sort();
        relays.dedup();
        Some((event, relays))
    }
}

#[async_trait]
impl GraphSource for RelaySource {
    async fn contacts(&self, pubkey_hex: &str) -> anyhow::Result<Option<ContactList>> {
        let Some((event, relays)) = self.newest_event(pubkey_hex, KIND_CONTACTS).await else {
            return Ok(None);
        };
        Ok(Some(ContactList {
            owner: pubkey_hex.to_string(),
            event_id: event.id.clone(),
            relays,
            created_at: event.created_at,
            follows: event.referenced_pubkeys(),
        }))
    }

    async fn profile(&self, pubkey_hex: &str) -> anyhow::Result<Option<Profile>> {
        let Some((event, _relays)) = self.newest_event(pubkey_hex, KIND_METADATA).await else {
            return Ok(None);
        };
        Ok(Some(parse_profile_content(&event.content)))
    }
}
