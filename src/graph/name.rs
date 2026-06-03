//! GNS name normalization and namespace membership.
//!
//! A profile is eligible for GNS resolution only if its `name` normalizes to a
//! non-empty label. A label `x` belongs to namespace `y` iff `y`'s kind-3
//! follow list contains a pubkey whose normalized label is `x`. If more than
//! one followed pubkey shares a label, that label is *ambiguous* and must not
//! resolve.

use futures_util::future::join_all;

use super::source::{GraphSource, Profile};

/// Normalize a profile `name` into a GNS label:
/// lowercase, then keep only ASCII `a-z` and `0-9`. May be empty.
pub fn normalize_label(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect()
}

/// One followed pubkey within a namespace, with its computed label.
#[derive(Debug, Clone)]
pub struct Member {
    pub pubkey: String,
    /// Normalized label (may be empty → not GNS-eligible).
    pub label: String,
    pub profile: Option<Profile>,
}

/// A namespace: a follower's contact list resolved to labelled members,
/// carrying the kind-3 event provenance used to justify each edge.
#[derive(Debug, Clone)]
pub struct Namespace {
    pub follow_event_id: String,
    pub relays: Vec<String>,
    pub members: Vec<Member>,
}

/// Outcome of looking up a single label inside a namespace.
#[derive(Debug, Clone)]
pub enum Membership {
    /// No follow carries this label.
    None,
    /// Exactly one follow carries this label.
    One(Member),
    /// Multiple follows carry this label — ambiguous, must not resolve.
    Ambiguous(Vec<Member>),
}

/// Load a namespace: fetch the owner's contact list, then the profile of each
/// follow (up to `fanout`) to compute labels. Follows are fetched concurrently.
pub async fn load_namespace(
    source: &dyn GraphSource,
    owner_hex: &str,
    fanout: usize,
) -> anyhow::Result<Namespace> {
    let Some(contacts) = source.contacts(owner_hex).await? else {
        return Ok(Namespace {
            follow_event_id: String::new(),
            relays: Vec::new(),
            members: Vec::new(),
        });
    };

    let follows: Vec<String> = contacts.follows.into_iter().take(fanout).collect();
    let fetches = follows.into_iter().map(|pk| async move {
        let profile = source.profile(&pk).await.ok().flatten();
        let label = profile
            .as_ref()
            .and_then(|p| p.name.as_deref())
            .map(normalize_label)
            .unwrap_or_default();
        Member {
            pubkey: pk,
            label,
            profile,
        }
    });
    let members = join_all(fetches).await;

    Ok(Namespace {
        follow_event_id: contacts.event_id,
        relays: contacts.relays,
        members,
    })
}

/// Apply the membership + ambiguity rules for a single label in a namespace.
pub fn resolve_label(ns: &Namespace, label: &str) -> Membership {
    if label.is_empty() {
        return Membership::None;
    }
    let matches: Vec<Member> = ns
        .members
        .iter()
        .filter(|m| !m.label.is_empty() && m.label == label)
        .cloned()
        .collect();
    match matches.len() {
        0 => Membership::None,
        1 => Membership::One(matches.into_iter().next().unwrap()),
        _ => Membership::Ambiguous(matches),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::demo_source::DemoSource;

    #[test]
    fn normalization_examples() {
        assert_eq!(normalize_label("Michael"), "michael");
        assert_eq!(normalize_label("Michael Saylor"), "michaelsaylor");
        assert_eq!(normalize_label("Michael_Saylor"), "michaelsaylor");
        assert_eq!(normalize_label("MICHAEL-SAYLOR"), "michaelsaylor");
        assert_eq!(normalize_label("⚡Michael⚡"), "michael");
        assert_eq!(normalize_label("FiatJaf"), "fiatjaf");
    }

    #[test]
    fn digits_kept_punctuation_dropped() {
        assert_eq!(normalize_label("bob123!!!"), "bob123");
        assert_eq!(normalize_label("  spaced  out  "), "spacedout");
    }

    #[test]
    fn empty_when_no_ascii_alnum() {
        assert_eq!(normalize_label("⚡⚡⚡"), "");
        assert_eq!(normalize_label("日本語"), "");
        assert_eq!(normalize_label(""), "");
    }

    #[tokio::test]
    async fn membership_unique_and_ambiguous() {
        let src = DemoSource::new();
        let ns = load_namespace(&src, &DemoSource::you(), 1000)
            .await
            .unwrap();

        // "michael" is uniquely a member of You's namespace.
        match resolve_label(&ns, "michael") {
            Membership::One(m) => assert_eq!(m.label, "michael"),
            other => panic!("expected unique michael, got {other:?}"),
        }
        // "frank" is followed twice (Frank and "Frank ⚡") → ambiguous.
        match resolve_label(&ns, "frank") {
            Membership::Ambiguous(v) => assert_eq!(v.len(), 2),
            other => panic!("expected ambiguous frank, got {other:?}"),
        }
        // A label nobody carries.
        assert!(matches!(resolve_label(&ns, "nobody"), Membership::None));
    }
}
