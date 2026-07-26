use super::Nip29Provider;
use anyhow::Result;
use std::collections::{HashMap, HashSet};

impl Nip29Provider {
    /// Verify that an outbound event may target `channel` without changing
    /// channel state. Creation and membership repair belong exclusively to
    /// explicit create/join flows.
    pub(super) async fn verify_publish_scope(
        &self,
        channel: &str,
        signer: &str,
        require_member: bool,
    ) -> Result<()> {
        anyhow::ensure!(!channel.is_empty(), "publish: channel must not be empty");
        let (exists, roles, members) = self.fetch_group_state(channel).await?;
        anyhow::ensure!(
            exists,
            "publish: channel {channel} does not exist; create it explicitly before publishing"
        );
        if require_member {
            anyhow::ensure!(
                signer_is_member(signer, &roles, &members),
                "publish: signer is not a member of channel {channel}; join it explicitly before publishing"
            );
        }
        Ok(())
    }
}

fn signer_is_member(
    signer: &str,
    roles: &HashMap<String, String>,
    members: &HashSet<String>,
) -> bool {
    roles.contains_key(signer) || members.contains(signer)
}

#[cfg(test)]
mod tests {
    use super::signer_is_member;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn relay_role_or_membership_authorizes_publication() {
        let roles = HashMap::from([("admin".to_string(), "admin".to_string())]);
        let members = HashSet::from(["agent".to_string()]);

        assert!(signer_is_member("admin", &roles, &members));
        assert!(signer_is_member("agent", &roles, &members));
        assert!(!signer_is_member("outsider", &roles, &members));
    }
}
