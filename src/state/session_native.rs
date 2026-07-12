use super::*;

impl Store {
    /// Bind a late-discovered harness-native resume token to a session.
    /// Headless Codex exec exposes this id only after launch, so the row is born
    /// under a watch-pid alias and receives the resumable token later.
    pub fn set_session_native_id(
        &self,
        id: &str,
        harness: &str,
        native_id: &str,
        now: u64,
    ) -> Result<()> {
        let Some(canonical) = self.resolve_canonical_id(id)? else {
            return Ok(());
        };
        if self.is_durable_agent_session(&canonical)? {
            self.put_alias(harness, "harness_session", native_id, &canonical, now)?;
            return Ok(());
        }
        self.conn.execute(
            "UPDATE sessions SET resume_id=?2 WHERE session_id=?1",
            params![canonical, native_id],
        )?;
        self.conn.execute(
            "UPDATE identities SET native_id=?2 WHERE session_id=?1",
            params![canonical, native_id],
        )?;
        self.put_alias(harness, "harness_session", native_id, &canonical, now)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_session_native_id_updates_row_identity_and_alias() {
        let store = Store::open_memory().unwrap();
        let session_id = store
            .register_session(&RegisterSession {
                harness: "codex".to_string(),
                external_id_kind: "watch_pid".to_string(),
                external_id: "123".to_string(),
                agent_pubkey: "pk".to_string(),
                agent_slug: "codex".to_string(),
                channel_h: "chan".to_string(),
                child_pid: Some(123),
                transcript_path: None,
                resume_id: String::new(),
                now: 10,
            })
            .unwrap();
        store
            .upsert_identity(&Identity {
                pubkey: "pk".to_string(),
                agent_slug: "codex".to_string(),
                codename: "willow-echo-042".to_string(),
                session_id: session_id.clone(),
                channel_h: "chan".to_string(),
                native_id: String::new(),
                alive: true,
                created_at: 10,
            })
            .unwrap();

        store
            .set_session_native_id("123", "codex", "native-codex", 20)
            .unwrap();

        let rec = store.get_session(&session_id).unwrap().unwrap();
        let identity = store.identity_for_session(&session_id).unwrap().unwrap();
        assert_eq!(rec.resume_id, "native-codex");
        assert_eq!(identity.native_id, "native-codex");
        assert_eq!(
            store
                .resolve_session_by_alias("codex", "harness_session", "native-codex")
                .unwrap()
                .as_deref(),
            Some(session_id.as_str())
        );
    }
}
