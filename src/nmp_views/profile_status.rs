use crate::state::{Profile, Status};

use super::NmpViews;

impl NmpViews {
    pub(crate) fn profile(&self, pubkey: &str) -> Option<Profile> {
        #[cfg(test)]
        if self.test_relay_delivery().is_some() {
            return self
                .projected_profiles()
                .into_iter()
                .find(|profile| profile.pubkey == pubkey);
        }
        self.observed_profile(pubkey)
            .map(|profile| profile.as_state_profile())
    }

    pub(crate) fn profiles(&self) -> Vec<Profile> {
        self.projected_profiles()
    }

    pub(crate) fn backend_profiles(&self) -> Vec<Profile> {
        let mut profiles = self
            .projected_profiles()
            .into_iter()
            .filter(|profile| profile.is_backend)
            .collect::<Vec<_>>();
        profiles
            .sort_by(|left, right| (&left.host, &left.pubkey).cmp(&(&right.host, &right.pubkey)));
        profiles
    }

    pub(crate) fn resolve_agent_pubkey(&self, slug: &str, host: &str) -> Option<String> {
        let name = crate::idref::agent_label(slug, host);
        let mut matches = self
            .projected_profiles()
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
        matches.into_iter().next()
    }

    pub(crate) fn resolve_profile_handle_pubkey(
        &self,
        handle: &str,
    ) -> anyhow::Result<Option<String>> {
        let handle = handle.trim();
        let mut matches = self
            .projected_profiles()
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

    pub(crate) fn pubkey_for_backend_label(&self, label: &str) -> Option<String> {
        let mut matches = self
            .projected_profiles()
            .into_iter()
            .filter(|profile| profile.is_backend && profile.host == label)
            .map(|profile| profile.pubkey)
            .collect::<Vec<_>>();
        matches.sort();
        matches.into_iter().next()
    }

    pub(crate) fn slug_for_pubkey(&self, pubkey: &str) -> Option<String> {
        self.profile(pubkey)
            .map(|profile| {
                crate::idref::session_handle_from_profile_name(&profile.slug, &profile.agent_slug)
            })
            .filter(|slug| !slug.is_empty())
    }

    pub(crate) fn status(&self, pubkey: &str, channel: &str) -> Option<Status> {
        self.projected_statuses_for_channel(channel)
            .into_iter()
            .find(|status| status.pubkey == pubkey)
    }

    pub(crate) fn statuses_in_channel(&self, channel: &str) -> Vec<Status> {
        let mut statuses = self.projected_statuses_for_channel(channel);
        statuses.sort_by_key(|status| std::cmp::Reverse(status.updated_at));
        statuses
    }

    pub(crate) fn statuses(&self, agent: Option<&str>, since: Option<u64>) -> Vec<Status> {
        let mut statuses = self
            .projected_statuses()
            .into_iter()
            .filter(|status| {
                agent
                    .filter(|agent| !agent.is_empty())
                    .is_none_or(|agent| status.pubkey == agent || status.slug == agent)
                    && since.is_none_or(|since| status.updated_at >= since)
            })
            .collect::<Vec<_>>();
        statuses.sort_by(|left, right| {
            (&left.channel_h, std::cmp::Reverse(left.updated_at))
                .cmp(&(&right.channel_h, std::cmp::Reverse(right.updated_at)))
        });
        statuses
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nmp::{Row, RowDelta};
    use nostr::{EventBuilder, Keys, Kind, RelayUrl, Tag, Timestamp};
    use std::collections::BTreeSet;

    fn status_row() -> Row {
        let event = EventBuilder::new(Kind::from(30315_u16), "writing focused tests")
            .tags([
                Tag::parse(["d", "status"]).unwrap(),
                Tag::parse(["h", "alpha"]).unwrap(),
                Tag::parse(["h", "beta"]).unwrap(),
                Tag::parse(["title", "NMP-owned status"]).unwrap(),
                Tag::parse(["state", "working"]).unwrap(),
                Tag::parse(["state-since", "100"]).unwrap(),
                Tag::parse(["host", "workstation"]).unwrap(),
                Tag::parse(["workspace", "mosaico"]).unwrap(),
                Tag::parse(["branch", "feature/nmp-views"]).unwrap(),
                Tag::parse(["slug", "amber-fox-123-codex"]).unwrap(),
                Tag::parse(["rel-cwd", "worktree"]).unwrap(),
                Tag::parse(["expiration", "1"]).unwrap(),
                Tag::parse(["e", "dispatch-event"]).unwrap(),
            ])
            .custom_created_at(Timestamp::from(123_u64))
            .sign_with_keys(&Keys::generate())
            .unwrap();
        Row {
            event,
            sources: BTreeSet::from([RelayUrl::parse("wss://relay.example").unwrap()]),
        }
    }

    #[test]
    fn status_projection_follows_exact_observation_scope_and_disappearance() {
        let views = NmpViews::default();
        let row = status_row();
        let id = row.event.id;

        views.apply_frame(
            "mosaico-h-alpha",
            1,
            vec![RowDelta::Added(row.clone())],
            vec![],
        );
        let alpha = views
            .observed_statuses_for_channel("alpha")
            .pop()
            .expect("alpha observation owns the row");
        assert_eq!(alpha.row.event.id, id);
        assert_eq!(alpha.row.sources, row.sources);
        assert_eq!(alpha.status.channels, ["alpha", "beta"]);
        assert_eq!(alpha.status.activity, "writing focused tests");
        assert_eq!(alpha.status.rel_cwd, "worktree");
        assert_eq!(
            alpha.status.dispatch_event.as_deref(),
            Some("dispatch-event")
        );
        assert_eq!(alpha.status.expires_at, Some(1));
        assert!(views.statuses_in_channel("beta").is_empty());

        views.apply_frame("mosaico-h-beta", 1, vec![RowDelta::Added(row)], vec![]);
        assert_eq!(views.statuses_in_channel("beta").len(), 1);

        let beta_departure = views.close_observation("mosaico-h-beta", 1).removed;
        assert_eq!(beta_departure.len(), 1);
        assert_eq!(beta_departure[0].observation_id, "mosaico-h-beta");
        assert_eq!(beta_departure[0].row.event.id, id);
        assert!(views.statuses_in_channel("beta").is_empty());
        assert_eq!(views.statuses_in_channel("alpha").len(), 1);

        let alpha_departure =
            views.apply_frame("mosaico-h-alpha", 1, vec![RowDelta::Removed(id)], vec![]);
        assert_eq!(alpha_departure.removed.len(), 1);
        assert_eq!(alpha_departure.removed[0].row.event.id, id);
        assert!(views.statuses_in_channel("alpha").is_empty());
        assert!(views.row(&id).is_none());
    }
}
