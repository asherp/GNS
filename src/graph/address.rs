//! Parsing of GNS addresses such as `barbara@alex.michael.nostr`.
//!
//! ```text
//! barbara@alex.michael.nostr
//! └─target  └────┬────┘ └─ namespace TLD ("nostr")
//!                └─ graph path (address order: alex, michael)
//! ```
//!
//! The address reads right-to-left for the walk from the resolving namespace:
//! `barbara@alex.michael.nostr` means "from here, find michael, then alex in
//! michael's follows, then barbara in alex's follows", i.e. walk labels
//! `[michael, alex, barbara]`.

use thiserror::Error;

use super::name::normalize_label;

/// The namespace TLD stripped from the end of an address, if present.
pub const NAMESPACE_TLD: &str = "nostr";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AddressError {
    #[error("address is empty")]
    Empty,
    #[error("target name has no valid GNS label")]
    EmptyTarget,
    #[error("path segment `{0}` has no valid GNS label")]
    EmptyPathSegment(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAddress {
    /// The original input.
    pub original: String,
    /// Normalized target label (the part before `@`, or the whole input).
    pub target_label: String,
    /// Normalized path labels in address order (e.g. `[alex, michael]`).
    pub path_labels: Vec<String>,
    /// Labels in walk order from the resolving namespace to the target.
    pub walk_labels: Vec<String>,
    /// The namespace TLD if one was present (e.g. `nostr`).
    pub namespace: Option<String>,
}

/// Parse a GNS address into its target, path, and walk order.
///
/// Accepts forms like `barbara@alex.michael.nostr`, `barbara@carol.nostr`,
/// `barbara@nostr`, and a bare `barbara` (compressed / direct lookup).
pub fn parse_gns_address(addr: &str) -> Result<ParsedAddress, AddressError> {
    let trimmed = addr.trim();
    if trimmed.is_empty() {
        return Err(AddressError::Empty);
    }

    let (target_part, domain_part) = match trimmed.split_once('@') {
        Some((t, d)) => (t, d),
        None => (trimmed, ""),
    };

    let target_label = normalize_label(target_part);
    if target_label.is_empty() {
        return Err(AddressError::EmptyTarget);
    }

    // Split the domain into dot-separated segments, dropping a trailing
    // namespace TLD ("nostr") if present.
    let mut segments: Vec<&str> = domain_part.split('.').filter(|s| !s.is_empty()).collect();
    let mut namespace = None;
    if let Some(last) = segments.last() {
        if normalize_label(last) == NAMESPACE_TLD {
            namespace = Some(NAMESPACE_TLD.to_string());
            segments.pop();
        }
    }

    let mut path_labels = Vec::with_capacity(segments.len());
    for seg in segments {
        let label = normalize_label(seg);
        if label.is_empty() {
            return Err(AddressError::EmptyPathSegment(seg.to_string()));
        }
        path_labels.push(label);
    }

    // Walk order: reverse of the address-order path, then the target.
    let mut walk_labels: Vec<String> = path_labels.iter().rev().cloned().collect();
    walk_labels.push(target_label.clone());

    Ok(ParsedAddress {
        original: trimmed.to_string(),
        target_label,
        path_labels,
        walk_labels,
        namespace,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_address() {
        let a = parse_gns_address("barbara@alex.michael.nostr").unwrap();
        assert_eq!(a.target_label, "barbara");
        assert_eq!(a.path_labels, vec!["alex", "michael"]);
        assert_eq!(a.walk_labels, vec!["michael", "alex", "barbara"]);
        assert_eq!(a.namespace.as_deref(), Some("nostr"));
    }

    #[test]
    fn single_hop() {
        let a = parse_gns_address("barbara@carol.nostr").unwrap();
        assert_eq!(a.walk_labels, vec!["carol", "barbara"]);
    }

    #[test]
    fn direct_in_namespace() {
        let a = parse_gns_address("barbara@nostr").unwrap();
        assert!(a.path_labels.is_empty());
        assert_eq!(a.walk_labels, vec!["barbara"]);
        assert_eq!(a.namespace.as_deref(), Some("nostr"));
    }

    #[test]
    fn bare_name_is_compressed_lookup() {
        let a = parse_gns_address("Barbara").unwrap();
        assert_eq!(a.target_label, "barbara");
        assert!(a.path_labels.is_empty());
        assert_eq!(a.walk_labels, vec!["barbara"]);
        assert_eq!(a.namespace, None);
    }

    #[test]
    fn normalizes_each_segment() {
        let a = parse_gns_address("Barbara@Alex .Michael_S.nostr").unwrap();
        assert_eq!(a.target_label, "barbara");
        assert_eq!(a.path_labels, vec!["alex", "michaels"]);
    }

    #[test]
    fn rejects_empty_target() {
        assert_eq!(
            parse_gns_address("⚡@alex.nostr"),
            Err(AddressError::EmptyTarget)
        );
    }

    #[test]
    fn rejects_empty_path_segment() {
        assert_eq!(
            parse_gns_address("barbara@⚡.michael.nostr"),
            Err(AddressError::EmptyPathSegment("⚡".to_string()))
        );
    }
}
