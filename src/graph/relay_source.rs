//! A `GraphSource` backed by a configurable set of live Nostr relays.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::future::join_all;

use crate::nostr::client::{query_relay, query_relay_followers};
use crate::nostr::event::{Event, KIND_CONTACTS, KIND_METADATA};

use super::source::{
    parse_profile_content, ContactList, FollowerEdge, FollowerList, GraphSource, Profile,
};

/// Fetches follow lists and profiles by fanning out across all configured relays.
pub struct RelaySource {
    relays: Vec<String>,
    request_timeout: Duration,
    /// Drop events that fail signature verification.
    verify: bool,
    /// Max kind-3 events requested per relay for a reverse-follower query.
    follower_limit: u32,
}

impl RelaySource {
    pub fn new(
        relays: Vec<String>,
        request_timeout: Duration,
        verify: bool,
        follower_limit: u32,
    ) -> Self {
        RelaySource {
            relays,
            request_timeout,
            verify,
            follower_limit,
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

    async fn followers(&self, pubkey_hex: &str) -> anyhow::Result<FollowerList> {
        // Fan out a `{kinds:[3], #p:[target]}` query across every relay.
        let queries = self.relays.iter().map(|relay| async move {
            let events = query_relay_followers(
                relay,
                pubkey_hex,
                KIND_CONTACTS,
                self.follower_limit,
                self.request_timeout,
            )
            .await;
            (relay.clone(), events)
        });
        let results = join_all(queries).await;

        // Keep the newest valid kind-3 event per follower, and remember every
        // relay that served that exact event id (provenance, mirroring forward
        // edges in `newest_event`).
        let mut newest: HashMap<String, Event> = HashMap::new();
        let mut relays_by_id: HashMap<String, Vec<String>> = HashMap::new();

        for (relay, events) in results {
            for ev in events {
                // The relay's `#p` filter is advisory; confirm the target is
                // actually referenced and the event is a well-formed kind 3.
                if ev.kind != KIND_CONTACTS {
                    continue;
                }
                if !ev.referenced_pubkeys().iter().any(|p| p == pubkey_hex) {
                    continue;
                }
                if self.verify && !ev.verify() {
                    continue;
                }
                relays_by_id
                    .entry(ev.id.clone())
                    .or_default()
                    .push(relay.clone());
                match newest.get(&ev.pubkey) {
                    Some(existing) if existing.created_at >= ev.created_at => {}
                    _ => {
                        newest.insert(ev.pubkey.clone(), ev);
                    }
                }
            }
        }

        let mut followers: Vec<FollowerEdge> = newest
            .into_values()
            .map(|ev| {
                let mut relays = relays_by_id.remove(&ev.id).unwrap_or_default();
                relays.sort();
                relays.dedup();
                FollowerEdge {
                    follower: ev.pubkey,
                    event_id: ev.id,
                    relays,
                    created_at: ev.created_at,
                }
            })
            .collect();
        // Newest followers first, with a stable tiebreak for deterministic paging.
        followers.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.follower.cmp(&b.follower))
        });

        Ok(FollowerList {
            target: pubkey_hex.to_string(),
            followers,
        })
    }
}
