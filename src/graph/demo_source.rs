//! An in-memory `GraphSource` with a small fixed social graph, so the server
//! and dashboard can run without any relay access (`--demo`). Also used by the
//! resolver tests.

use std::collections::HashMap;

use async_trait::async_trait;

use super::source::{ContactList, GraphSource, Profile};

/// 32-byte hex pubkey built by repeating a single byte. Handy for fixtures.
fn pk(byte: u8) -> String {
    let mut s = String::with_capacity(64);
    for _ in 0..32 {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

pub struct DemoSource {
    contacts: HashMap<String, ContactList>,
    profiles: HashMap<String, Profile>,
}

impl Default for DemoSource {
    fn default() -> Self {
        Self::new()
    }
}

impl DemoSource {
    /// Mirrors the worked example from the GNS vision:
    /// You → Michael → Alex → Barbara, plus alternate routes via Carol.
    pub fn new() -> Self {
        let you = pk(0x01);
        let michael = pk(0x02);
        let alex = pk(0x03);
        let barbara = pk(0x04);
        let carol = pk(0x05);
        let dave = pk(0x06);

        let mut contacts = HashMap::new();
        let mut profiles = HashMap::new();

        let edge = |owner: &str, follows: &[&str], contacts: &mut HashMap<String, ContactList>| {
            let event_id = format!("demoevent{}", &owner[..16]);
            contacts.insert(
                owner.to_string(),
                ContactList {
                    owner: owner.to_string(),
                    event_id,
                    relays: vec!["wss://demo.relay.invalid".to_string()],
                    created_at: 1_700_000_000,
                    follows: follows.iter().map(|s| s.to_string()).collect(),
                },
            );
        };

        edge(&you, &[&michael, &carol], &mut contacts);
        edge(&michael, &[&alex, &dave], &mut contacts);
        edge(&alex, &[&barbara], &mut contacts);
        edge(&carol, &[&barbara], &mut contacts);
        edge(&dave, &[&barbara], &mut contacts);
        edge(&barbara, &[], &mut contacts);

        let profile = |hex: &str, name: &str, profiles: &mut HashMap<String, Profile>| {
            profiles.insert(
                hex.to_string(),
                Profile {
                    name: Some(name.to_string()),
                    display_name: Some(name.to_string()),
                    picture: None,
                    nip05: None,
                    about: Some(format!("Demo profile for {name}")),
                },
            );
        };
        profile(&you, "You", &mut profiles);
        profile(&michael, "Michael", &mut profiles);
        profile(&alex, "Alex", &mut profiles);
        profile(&barbara, "Barbara", &mut profiles);
        profile(&carol, "Carol", &mut profiles);
        profile(&dave, "Dave", &mut profiles);

        DemoSource { contacts, profiles }
    }

    /// Convenience accessors used by tests / the dashboard hint.
    #[allow(dead_code)]
    pub fn you() -> String {
        pk(0x01)
    }
    #[allow(dead_code)]
    pub fn barbara() -> String {
        pk(0x04)
    }
}

#[async_trait]
impl GraphSource for DemoSource {
    async fn contacts(&self, pubkey_hex: &str) -> anyhow::Result<Option<ContactList>> {
        Ok(self.contacts.get(pubkey_hex).cloned())
    }

    async fn profile(&self, pubkey_hex: &str) -> anyhow::Result<Option<Profile>> {
        Ok(self.profiles.get(pubkey_hex).cloned())
    }
}
