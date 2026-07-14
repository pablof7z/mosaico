use super::*;
use crate::idref::{parse_ref, Ref};
use crate::state::Message;
use std::collections::HashSet;

pub(super) struct AuthorFilter {
    pubkeys: HashSet<String>,
    labels: Vec<String>,
}

impl AuthorFilter {
    pub(super) fn from_params(
        state: &Arc<DaemonState>,
        scopes: &[String],
        params: &WaitParams,
    ) -> Result<Self> {
        let mut filter = Self {
            pubkeys: params.from_pubkeys.iter().cloned().collect(),
            labels: params
                .from_labels
                .iter()
                .map(|label| clean_label(label))
                .filter(|label| !label.is_empty())
                .collect(),
        };
        if let Some(raw) = params.from.as_deref().map(clean_label) {
            if raw.is_empty() {
                anyhow::bail!("--from must not be empty");
            }
            let first_scope = &scopes[0];
            let mut resolved_any = false;
            if let Ok(resolved) = state.with_store(|store| {
                super::super::channel_send::resolve_recipient(store, first_scope, &state.host, &raw)
            }) {
                resolved_any = true;
                filter.pubkeys.insert(resolved.pubkey);
            }
            state.with_store(|store| {
                for scope in scopes {
                    for member in store.list_channel_members(scope).unwrap_or_default() {
                        let profile = store.get_profile(&member.pubkey).ok().flatten();
                        let session = store.session_for_pubkey(&member.pubkey).ok().flatten();
                        if identity_matches(
                            &raw,
                            profile.as_ref(),
                            session.as_ref(),
                            &member.pubkey,
                        ) {
                            resolved_any = true;
                            filter.pubkeys.insert(member.pubkey);
                        }
                    }
                }
            });
            if !resolved_any {
                anyhow::bail!("can't resolve --from {raw:?} in the selected channels");
            }
            filter.labels.push(raw);
        }
        filter.labels.sort();
        filter.labels.dedup();
        Ok(filter)
    }

    pub(super) fn matches(&self, state: &Arc<DaemonState>, message: &Message) -> bool {
        if self.is_unrestricted() {
            return true;
        }
        if self.pubkeys.contains(&message.author_pubkey) {
            return true;
        }
        let (profile, session) = state.with_store(|store| {
            let profile = store.get_profile(&message.author_pubkey).ok().flatten();
            let session = store
                .session_for_pubkey(&message.author_pubkey)
                .ok()
                .flatten();
            (profile, session)
        });
        self.labels.iter().any(|label| {
            identity_matches(
                label,
                profile.as_ref(),
                session.as_ref(),
                &message.author_pubkey,
            )
        })
    }

    fn is_unrestricted(&self) -> bool {
        self.pubkeys.is_empty() && self.labels.is_empty()
    }
}

fn clean_label(label: &str) -> String {
    label.trim().trim_start_matches('@').to_string()
}

fn identity_matches(
    label: &str,
    profile: Option<&crate::state::Profile>,
    session: Option<&crate::state::Session>,
    pubkey: &str,
) -> bool {
    match parse_ref(label) {
        Ref::Pubkey(raw) => crate::idref::normalize_pubkey(&raw)
            .as_deref()
            .is_some_and(|expected| expected == pubkey),
        Ref::Agent { slug, host } => profile.is_some_and(|profile| {
            profile.host == host && (profile.agent_slug == slug || profile.slug == slug)
        }),
        Ref::Token(token) => {
            profile.is_some_and(|profile| {
                profile.name == token || profile.slug == token || profile.agent_slug == token
            }) || session.is_some_and(|session| session.agent_slug == token)
        }
    }
}
