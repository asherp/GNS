//! Resolve a GNS address by walking its label sequence through the follow graph.
//!
//! Starting from the resolving namespace (`from`), each label is looked up via
//! the membership rule. An unambiguous match advances the walk; an ambiguous
//! label branches into alternate paths and marks the resolution non-resolving
//! (per the ambiguity rule, an ambiguous address must not resolve to a single
//! pubkey, but alternates are returned for disambiguation).

use serde::Serialize;

use crate::nostr::{hex_to_npub, PublicKey};

use super::address::ParsedAddress;
use super::name::{load_namespace, normalize_label, resolve_label, Membership};
use super::resolver::PathEdge;
use super::source::{GraphSource, Profile};

/// One person along a named path.
#[derive(Debug, Clone, Serialize)]
pub struct NamedNode {
    pub npub: String,
    pub pubkey: String,
    /// Normalized GNS label of this node (empty for the root / unlabelled).
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
}

/// A complete labelled path from the resolving namespace to a target.
#[derive(Debug, Clone, Serialize)]
pub struct NamedPath {
    pub nodes: Vec<NamedNode>,
    pub edges: Vec<PathEdge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NameResolution {
    pub query: String,
    pub from: String,
    pub target_label: String,
    pub walk_labels: Vec<String>,
    pub found: bool,
    /// True if any label along the walk was ambiguous, or multiple paths exist.
    pub ambiguous: bool,
    /// The unique target npub, only when resolution is unambiguous.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved: Option<String>,
    /// All complete paths found (one when unambiguous; alternates otherwise).
    pub paths: Vec<NamedPath>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Bounds on the named walk.
#[derive(Debug, Clone, Copy)]
pub struct NameResolveOptions {
    /// Cap follows inspected per namespace (each needs a profile fetch).
    pub max_fanout: usize,
    /// Cap on alternate paths carried forward while branching.
    pub max_paths: usize,
}

impl Default for NameResolveOptions {
    fn default() -> Self {
        NameResolveOptions {
            max_fanout: 5000,
            max_paths: 16,
        }
    }
}

#[derive(Clone)]
struct Partial {
    nodes: Vec<NamedNode>,
    edges: Vec<PathEdge>,
}

pub async fn resolve_address(
    source: &dyn GraphSource,
    from: PublicKey,
    parsed: &ParsedAddress,
    opts: NameResolveOptions,
) -> anyhow::Result<NameResolution> {
    let root_hex = from.to_hex();
    let root_profile = source.profile(&root_hex).await.ok().flatten();
    let root_label = root_profile
        .as_ref()
        .and_then(|p| p.name.as_deref())
        .map(normalize_label)
        .unwrap_or_default();

    let mut partials = vec![Partial {
        nodes: vec![NamedNode {
            npub: from.to_npub(),
            pubkey: root_hex,
            label: root_label,
            profile: root_profile,
        }],
        edges: Vec::new(),
    }];

    let mut ambiguous = false;

    for label in &parsed.walk_labels {
        let mut next: Vec<Partial> = Vec::new();
        for p in &partials {
            let current = &p.nodes.last().expect("partial always has a node").pubkey;
            let ns = load_namespace(source, current, opts.max_fanout).await?;
            let matches = match resolve_label(&ns, label) {
                Membership::None => Vec::new(),
                Membership::One(m) => vec![m],
                Membership::Ambiguous(v) => {
                    ambiguous = true;
                    v
                }
            };
            for m in matches {
                let mut nodes = p.nodes.clone();
                let mut edges = p.edges.clone();
                edges.push(PathEdge {
                    from: hex_to_npub(current),
                    to: hex_to_npub(&m.pubkey),
                    follow_event_id: ns.follow_event_id.clone(),
                    relays: ns.relays.clone(),
                });
                nodes.push(NamedNode {
                    npub: hex_to_npub(&m.pubkey),
                    pubkey: m.pubkey.clone(),
                    label: m.label.clone(),
                    profile: m.profile.clone(),
                });
                next.push(Partial { nodes, edges });
            }
        }
        next.truncate(opts.max_paths);
        partials = next;
        if partials.is_empty() {
            break;
        }
    }

    let found = !partials.is_empty();
    if partials.len() > 1 {
        ambiguous = true;
    }
    let resolved = if found && !ambiguous && partials.len() == 1 {
        partials[0].nodes.last().map(|n| n.npub.clone())
    } else {
        None
    };

    let note = if !found {
        Some("No follow with the required label was found along this path.".to_string())
    } else if ambiguous {
        Some(
            "Ambiguous label encountered; per the GNS ambiguity rule this address does not \
             resolve to a single pubkey. Alternate paths are returned for disambiguation."
                .to_string(),
        )
    } else {
        None
    };

    Ok(NameResolution {
        query: parsed.original.clone(),
        from: from.to_npub(),
        target_label: parsed.target_label.clone(),
        walk_labels: parsed.walk_labels.clone(),
        found,
        ambiguous,
        resolved,
        paths: partials
            .into_iter()
            .map(|p| NamedPath {
                nodes: p.nodes,
                edges: p.edges,
            })
            .collect(),
        note,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::address::parse_gns_address;
    use crate::graph::demo_source::DemoSource;

    async fn resolve(addr: &str) -> NameResolution {
        let src = DemoSource::new();
        let from = PublicKey::from_hex(&DemoSource::you()).unwrap();
        let parsed = parse_gns_address(addr).unwrap();
        resolve_address(&src, from, &parsed, NameResolveOptions::default())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn resolves_full_named_path() {
        let res = resolve("barbara@alex.michael.nostr").await;
        assert!(res.found);
        assert!(!res.ambiguous);
        assert_eq!(res.paths.len(), 1);
        let labels: Vec<&str> = res.paths[0]
            .nodes
            .iter()
            .map(|n| n.label.as_str())
            .collect();
        assert_eq!(labels, vec!["you", "michael", "alex", "barbara"]);
        assert!(res.resolved.is_some());
        assert_eq!(
            res.resolved.unwrap(),
            res.paths[0].nodes.last().unwrap().npub
        );
        // Provenance present on each hop.
        for e in &res.paths[0].edges {
            assert!(!e.follow_event_id.is_empty());
            assert!(!e.relays.is_empty());
        }
    }

    #[tokio::test]
    async fn resolves_single_hop() {
        let res = resolve("barbara@carol.nostr").await;
        assert!(res.found && !res.ambiguous);
        let labels: Vec<&str> = res.paths[0]
            .nodes
            .iter()
            .map(|n| n.label.as_str())
            .collect();
        assert_eq!(labels, vec!["you", "carol", "barbara"]);
    }

    #[tokio::test]
    async fn ambiguous_label_does_not_resolve() {
        let res = resolve("frank@nostr").await;
        assert!(res.found);
        assert!(res.ambiguous);
        assert!(res.resolved.is_none(), "ambiguous address must not resolve");
        assert_eq!(
            res.paths.len(),
            2,
            "both Frank candidates returned as alternates"
        );
    }

    #[tokio::test]
    async fn missing_label_not_found() {
        let res = resolve("nobody@michael.nostr").await;
        assert!(!res.found);
        assert!(res.resolved.is_none());
        assert!(res.paths.is_empty());
    }
}
