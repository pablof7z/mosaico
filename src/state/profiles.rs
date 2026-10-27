//! Profile reads projected from the complete current NIP-29 view.
//!
//! The feed holds only the current member set's kind:0 rows (one retained
//! `authors:[members]` observation), not a union of all stored events. The
//! profile-list and resolve surface reads it directly; `get_profile` reads it
//! by pubkey.

use super::*;
use crate::idref;

impl Store {
    pub(crate) fn current_profiles(&self) -> Vec<Profile> {
        #[cfg(test)]
        if let Some(delivery) = self
            .test_snapshots
            .read()
            .expect("test snapshots poisoned")
            .relay
            .as_ref()
        {
            return delivery.profiles.clone();
        }
        self.nip29_view
            .as_ref()
            .map(|view| view.profiles())
            .unwrap_or_default()
    }

    pub fn get_profile(&self, pubkey: &str) -> Result<Option<Profile>> {
        Ok(self
            .current_profiles()
            .into_iter()
            .find(|profile| profile.pubkey == pubkey))
    }

    /// Every current profile keyed by pubkey (first wins, as `get_profile`),
    /// for callers that look up many pubkeys.
    pub fn profiles_by_pubkey(&self) -> std::collections::BTreeMap<String, Profile> {
        let mut map = std::collections::BTreeMap::new();
        for profile in self.current_profiles() {
            map.entry(profile.pubkey.clone()).or_insert(profile);
        }
        map
    }

    pub fn list_backend_profiles(&self) -> Result<Vec<Profile>> {
        let mut profiles = self
            .current_profiles()
            .into_iter()
            .filter(|profile| profile.is_backend)
            .collect::<Vec<_>>();
        profiles
            .sort_by(|left, right| (&left.host, &left.pubkey).cmp(&(&right.host, &right.pubkey)));
        Ok(profiles)
    }

    pub fn resolve_agent_pubkey(&self, slug: &str, host: &str) -> Result<Option<String>> {
        let name = idref::agent_label(slug, host);
        let mut matches = self
            .current_profiles()
            .into_iter()
            .filter(|profile| {
                !profile.is_backend
                    && profile.agent_slug.is_empty()
                    && profile.host == host
                    && (profile.slug == slug || profile.name == name)
            })
            .map(|profile| profile.pubkey)
            .collect::<Vec<_>>();
        matches.sort();
        Ok(matches.into_iter().next())
    }

    pub fn resolve_profile_handle_pubkey(&self, handle: &str) -> Result<Option<String>> {
        let handle = handle.trim();
        let mut matches = self
            .current_profiles()
            .into_iter()
            .filter(|profile| {
                !profile.is_backend
                    && !profile.agent_slug.is_empty()
                    && (profile.name == handle || profile.slug == handle)
            })
            .map(|profile| profile.pubkey)
            .collect::<Vec<_>>();
        matches.sort();
        matches.dedup();
        match matches.as_slice() {
            [] => Ok(None),
            [pubkey] => Ok(Some(pubkey.clone())),
            _ => anyhow::bail!("remote handle {handle:?} is ambiguous"),
        }
    }

    pub fn pubkey_for_backend_label(&self, label: &str) -> Result<Option<String>> {
        let mut matches = self
            .current_profiles()
            .into_iter()
            .filter(|profile| profile.is_backend && profile.host == label)
            .map(|profile| profile.pubkey)
            .collect::<Vec<_>>();
        matches.sort();
        Ok(matches.into_iter().next())
    }

    pub fn resolve_slug_for_pubkey(&self, pubkey: &str) -> Result<Option<String>> {
        Ok(self
            .current_profiles()
            .into_iter()
            .find(|profile| profile.pubkey == pubkey)
            .map(|profile| {
                idref::session_handle_from_profile_name(&profile.slug, &profile.agent_slug)
            })
            .filter(|slug| !slug.is_empty()))
    }
}
