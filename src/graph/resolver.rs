//! Shortest-path resolution over the Nostr follow graph.
//!
//! Given a `from` and `to` pubkey, walk kind-3 follow lists breadth-first to
//! find the shortest chain `from → … → to`. Each hop `A → B` is backed by A's
//! contact-list event and the relays on which it was observed.

use std::collections::{HashMap, HashSet};

use futures_util::future::join_all;
use serde::Serialize;

use crate::nostr::{hex_to_npub, PublicKey};

use super::source::{GraphSource, Profile};

/// One person in a resolved path.
#[derive(Debug, Clone, Serialize)]
pub struct PathNode {
    pub npub: String,
    pub pubkey: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
}

/// One hop `from → to`, with the follow event that justifies it.
#[derive(Debug, Clone, Serialize)]
pub struct PathEdge {
    pub from: String,
    pub to: String,
    /// Id of the follower's kind-3 event.
    pub follow_event_id: String,
    /// Relays on which that event was observed.
    pub relays: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Resolution {
    pub from: String,
    pub to: String,
    pub found: bool,
    pub hops: usize,
    /// Nodes ordered from `from` to `to`.
    pub path: Vec<PathNode>,
    /// Edges ordered to match consecutive `path` nodes.
    pub edges: Vec<PathEdge>,
    /// Number of distinct pubkeys visited during the search (diagnostics).
    pub visited: usize,
}

/// Parameters bounding the breadth-first search.
#[derive(Debug, Clone, Copy)]
pub struct ResolveOptions {
    pub max_depth: usize,
    /// Cap follows expanded per node to bound fan-out on huge accounts.
    pub max_fanout: usize,
}

impl Default for ResolveOptions {
    fn default() -> Self {
        ResolveOptions {
            max_depth: 6,
            max_fanout: 5000,
        }
    }
}

/// Records how we reached a node: its parent and the edge backing the hop.
struct Backref {
    parent: String,
    follow_event_id: String,
    relays: Vec<String>,
}

pub async fn resolve(
    source: &dyn GraphSource,
    from: PublicKey,
    to: PublicKey,
    opts: ResolveOptions,
) -> anyhow::Result<Resolution> {
    let from_hex = from.to_hex();
    let to_hex = to.to_hex();

    let mut visited: HashSet<String> = HashSet::new();
    let mut backref: HashMap<String, Backref> = HashMap::new();
    visited.insert(from_hex.clone());

    let mut frontier: Vec<String> = vec![from_hex.clone()];
    let mut found = from_hex == to_hex;

    let mut depth = 0;
    while !found && depth < opts.max_depth && !frontier.is_empty() {
        // Fetch contact lists for the whole frontier concurrently.
        let fetches = frontier.iter().map(|owner| {
            let owner = owner.clone();
            async move {
                let cl = source.contacts(&owner).await;
                (owner, cl)
            }
        });
        let results = join_all(fetches).await;

        let mut next: Vec<String> = Vec::new();
        for (owner, cl) in results {
            let contacts = match cl {
                Ok(Some(c)) => c,
                Ok(None) => continue,
                Err(_) => continue,
            };
            for follow in contacts.follows.iter().take(opts.max_fanout) {
                if visited.contains(follow) {
                    continue;
                }
                visited.insert(follow.clone());
                backref.insert(
                    follow.clone(),
                    Backref {
                        parent: owner.clone(),
                        follow_event_id: contacts.event_id.clone(),
                        relays: contacts.relays.clone(),
                    },
                );
                if *follow == to_hex {
                    found = true;
                    break;
                }
                next.push(follow.clone());
            }
            if found {
                break;
            }
        }
        frontier = next;
        depth += 1;
    }

    let mut resolution = Resolution {
        from: from.to_npub(),
        to: to.to_npub(),
        found,
        hops: 0,
        path: Vec::new(),
        edges: Vec::new(),
        visited: visited.len(),
    };

    if !found {
        return Ok(resolution);
    }

    // Reconstruct the chain from `to` back to `from`.
    let mut chain_hex: Vec<String> = vec![to_hex.clone()];
    let mut cursor = to_hex.clone();
    while cursor != from_hex {
        let Some(b) = backref.get(&cursor) else { break };
        resolution.edges.push(PathEdge {
            from: hex_to_npub(&b.parent),
            to: hex_to_npub(&cursor),
            follow_event_id: b.follow_event_id.clone(),
            relays: b.relays.clone(),
        });
        cursor = b.parent.clone();
        chain_hex.push(cursor.clone());
    }
    chain_hex.reverse();
    resolution.edges.reverse();
    resolution.hops = resolution.edges.len();

    // Attach profiles for every node in the path, concurrently.
    let profile_fetches = chain_hex.iter().map(|hex| {
        let hex = hex.clone();
        async move {
            let p = source.profile(&hex).await.ok().flatten();
            (hex, p)
        }
    });
    let profiles: HashMap<String, Option<Profile>> =
        join_all(profile_fetches).await.into_iter().collect();

    resolution.path = chain_hex
        .into_iter()
        .map(|hex| PathNode {
            npub: hex_to_npub(&hex),
            profile: profiles.get(&hex).cloned().flatten(),
            pubkey: hex,
        })
        .collect();

    Ok(resolution)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::demo_source::DemoSource;

    #[tokio::test]
    async fn finds_shortest_path() {
        let src = DemoSource::new();
        let from = PublicKey::from_hex(&DemoSource::you()).unwrap();
        let to = PublicKey::from_hex(&DemoSource::barbara()).unwrap();
        let res = resolve(&src, from, to, ResolveOptions::default())
            .await
            .unwrap();

        assert!(res.found);
        // You → Michael → Alex → Barbara  OR  You → Carol → Barbara (shorter).
        // BFS must return the shortest: 2 hops via Carol.
        assert_eq!(res.hops, 2);
        assert_eq!(res.path.len(), 3);
        assert_eq!(res.path.first().unwrap().pubkey, DemoSource::you());
        assert_eq!(res.path.last().unwrap().pubkey, DemoSource::barbara());
        // Every edge carries provenance.
        for e in &res.edges {
            assert!(!e.follow_event_id.is_empty());
            assert!(!e.relays.is_empty());
        }
    }

    #[tokio::test]
    async fn same_node_is_zero_hops() {
        let src = DemoSource::new();
        let you = PublicKey::from_hex(&DemoSource::you()).unwrap();
        let res = resolve(&src, you, you, ResolveOptions::default())
            .await
            .unwrap();
        assert!(res.found);
        assert_eq!(res.hops, 0);
        assert_eq!(res.path.len(), 1);
    }

    #[tokio::test]
    async fn unreachable_returns_not_found() {
        let src = DemoSource::new();
        let from = PublicKey::from_hex(&DemoSource::barbara()).unwrap(); // follows nobody
        let to = PublicKey::from_hex(&DemoSource::you()).unwrap();
        let res = resolve(&src, from, to, ResolveOptions::default())
            .await
            .unwrap();
        assert!(!res.found);
        assert!(res.path.is_empty());
    }
}
