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
    assign_instance(env, env_remove);
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

fn assign_instance(env: &mut Vec<(String, String)>, env_remove: &mut Vec<String>) {
    env.retain(|(key, _)| key != crate::config::INSTANCE_ENV);
    env_remove.retain(|key| key != crate::config::INSTANCE_ENV);
    if let Some(instance) = crate::config::selected_instance_env() {
        env.push((crate::config::INSTANCE_ENV.to_string(), instance));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigned_identity_replaces_harness_values_and_cannot_be_removed() {
        let _selector = crate::test_env::EnvGuard::remove(crate::config::INSTANCE_ENV);
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

    #[test]
    fn selected_instance_replaces_harness_values_and_cannot_be_removed() {
        let _selector = crate::test_env::EnvGuard::set(crate::config::INSTANCE_ENV, "alternative1");
        let mut env = vec![
            (crate::config::INSTANCE_ENV.into(), "wrong-instance".into()),
            ("KEEP".into(), "yes".into()),
        ];
        let mut env_remove = vec![crate::config::INSTANCE_ENV.into(), "DROP".into()];

        assign(&mut env, &mut env_remove, "pubkey", "secret");

        assert!(env.contains(&(crate::config::INSTANCE_ENV.into(), "alternative1".into())));
        assert!(!env.iter().any(|pair| pair.1 == "wrong-instance"));
        assert!(!env_remove
            .iter()
            .any(|key| key == crate::config::INSTANCE_ENV));
    }
}
