pub(crate) const PUBKEY: &str = "MOSAICO_PUBKEY";
pub(crate) const NSEC: &str = "AGENT_NSEC";

pub(crate) fn assign(
    env: &mut Vec<(String, String)>,
    env_remove: &mut Vec<String>,
    pubkey: &str,
    nsec: &str,
) {
    env.retain(|(key, _)| !is_identity_key(key));
    env.extend([
        (PUBKEY.to_string(), pubkey.to_string()),
        (NSEC.to_string(), nsec.to_string()),
    ]);
    env_remove.retain(|key| !is_identity_key(key));
}

pub(crate) fn assign_launch(
    env: &mut Vec<(String, String)>,
    env_remove: &mut Vec<String>,
    spec: &super::transport::LaunchSpec,
) {
    assign(env, env_remove, &spec.pubkey, &spec.agent_nsec);
}

fn is_identity_key(key: &str) -> bool {
    key == PUBKEY || key == NSEC
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigned_identity_replaces_harness_values_and_cannot_be_removed() {
        let mut env = vec![
            (NSEC.to_string(), "wrong-secret".to_string()),
            (PUBKEY.to_string(), "wrong-pubkey".to_string()),
            ("KEEP".to_string(), "yes".to_string()),
        ];
        let mut env_remove = vec![NSEC.to_string(), PUBKEY.to_string(), "DROP".to_string()];

        assign(
            &mut env,
            &mut env_remove,
            "assigned-pubkey",
            "assigned-secret",
        );

        assert_eq!(
            env,
            vec![
                ("KEEP".to_string(), "yes".to_string()),
                (PUBKEY.to_string(), "assigned-pubkey".to_string()),
                (NSEC.to_string(), "assigned-secret".to_string()),
            ]
        );
        assert_eq!(env_remove, vec!["DROP".to_string()]);
    }
}
