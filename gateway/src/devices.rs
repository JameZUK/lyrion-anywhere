//! Device authentication.
//!
//! Phase 0: devices come from a JSON file holding only SHA-256 hashes of their tokens.
//! Phase 1 replaces this with plugin-managed accounts, refresh tokens and short-lived
//! access tokens (SPEC §9).

use std::sync::Arc;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use anywhere_core::proto::{format_mac, parse_mac};

#[derive(Debug)]
pub struct Device {
    pub name: String,
    pub mac: [u8; 6],
    token_hash: [u8; 32],
}

impl Device {
    pub fn mac_string(&self) -> String {
        format_mac(&self.mac)
    }
}

#[derive(Deserialize)]
struct DeviceEntry {
    name: String,
    mac: String,
    token_sha256: String,
}

#[derive(Debug, Default)]
pub struct Devices {
    list: Vec<Arc<Device>>,
}

impl Devices {
    pub fn from_json(json: &str) -> Result<Self, String> {
        let entries: Vec<DeviceEntry> =
            serde_json::from_str(json).map_err(|e| format!("devices file: {e}"))?;
        let mut list: Vec<Arc<Device>> = Vec::new();
        for (i, e) in entries.into_iter().enumerate() {
            let mac =
                parse_mac(&e.mac).ok_or_else(|| format!("devices[{i}]: bad mac {:?}", e.mac))?;
            if mac[0] & 0x02 == 0 {
                return Err(format!(
                    "devices[{i}]: mac must be locally administered (02:…)"
                ));
            }
            if list.iter().any(|d| d.mac == mac) {
                return Err(format!("devices[{i}]: duplicate mac"));
            }
            let h = hex::decode(&e.token_sha256).ok().filter(|h| h.len() == 32);
            let Some(h) = h else {
                return Err(format!(
                    "devices[{i}]: token_sha256 must be 64 hex characters"
                ));
            };
            let mut token_hash = [0u8; 32];
            token_hash.copy_from_slice(&h);
            list.push(Arc::new(Device {
                name: e.name,
                mac,
                token_hash,
            }));
        }
        Ok(Devices { list })
    }

    /// Build from plaintext tokens (tests and tooling).
    pub fn from_tokens(entries: &[(&str, &str, [u8; 6])]) -> Self {
        let list = entries
            .iter()
            .map(|(token, name, mac)| {
                Arc::new(Device {
                    name: name.to_string(),
                    mac: *mac,
                    token_hash: hash(token),
                })
            })
            .collect();
        Devices { list }
    }

    /// The device holding `token`, if any. Every device is compared in constant time, so
    /// response timing reveals nothing about near matches or which device matched.
    pub fn authenticate(&self, token: &str) -> Option<Arc<Device>> {
        if token.is_empty() {
            return None;
        }
        let h = hash(token);
        let mut found = None;
        for d in &self.list {
            if bool::from(d.token_hash.ct_eq(&h)) {
                found = Some(d.clone());
            }
        }
        found
    }
}

fn hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// A new random 256-bit token (URL-safe hex) and the SHA-256 to store.
pub fn new_token() -> (String, String) {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("OS random source");
    let token = hex::encode(b);
    let h = hex::encode(hash(&token));
    (token, h)
}

pub fn random_id() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("OS random source");
    hex::encode(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticates_only_exact_tokens() {
        let d = Devices::from_tokens(&[
            ("secret-a", "a", [2, 0, 0, 0, 0, 1]),
            ("secret-b", "b", [2, 0, 0, 0, 0, 2]),
        ]);
        assert_eq!(d.authenticate("secret-a").unwrap().name, "a");
        assert_eq!(d.authenticate("secret-b").unwrap().name, "b");
        for bad in [
            "",
            "secret-",
            "secret-a ",
            "SECRET-A",
            "secret-c",
            "secret-a\0",
        ] {
            assert!(d.authenticate(bad).is_none(), "{bad:?} accepted");
        }
    }

    #[test]
    fn devices_file_validation() {
        let (_, h) = new_token();
        let ok = format!(r#"[{{"name":"x","mac":"02:00:00:00:00:01","token_sha256":"{h}"}}]"#);
        assert!(Devices::from_json(&ok).is_ok());
        for bad in [
            format!(r#"[{{"name":"x","mac":"00:11:22:33:44:55","token_sha256":"{h}"}}]"#), // global MAC
            format!(r#"[{{"name":"x","mac":"02:00","token_sha256":"{h}"}}]"#),
            r#"[{"name":"x","mac":"02:00:00:00:00:01","token_sha256":"abcd"}]"#.to_string(),
            format!(
                r#"[{{"name":"x","mac":"02:00:00:00:00:01","token_sha256":"{h}"}},{{"name":"y","mac":"02:00:00:00:00:01","token_sha256":"{h}"}}]"#
            ),
            "not json".to_string(),
        ] {
            assert!(Devices::from_json(&bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn tokens_and_ids_are_random() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let (t, h) = new_token();
            assert_eq!(t.len(), 64);
            assert_eq!(h.len(), 64);
            assert!(seen.insert(t));
            assert!(seen.insert(random_id()));
        }
    }
}
