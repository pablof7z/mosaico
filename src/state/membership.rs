use super::{MembershipDecision, Store};
use anyhow::Result;
use rusqlite::params;

impl Store {
    // ── Phase 1: canonical read-model accessors ──────────────────────────
    // Write-side primitives the materializer fills; readers come in Phase 2.
    // These tables are additive — no existing reader consults them yet, so
    // none of this changes CLI/RPC output.

    /// Map fabric coordinates to a durable `project_id`, creating the project +
    /// origin on first sight. Idempotent: the same origin always resolves to the
    /// same id and never clobbers `about`.
    pub fn ensure_project_origin(
        &self,
        fabric: &str,
        provider_instance: &str,
        native_project_key: &str,
        display_slug: &str,
        now: u64,
    ) -> Result<String> {
        if let Some(pid) =
            self.project_id_for_origin(fabric, provider_instance, native_project_key)?
        {
            return Ok(pid);
        }
        let pid = super::gen_id("proj");
        self.conn.execute(
            "INSERT INTO projects (project_id, display_slug, about, created_at, updated_at)
             VALUES (?1, ?2, NULL, ?3, ?3)",
            params![pid, display_slug, now],
        )?;
        self.conn.execute(
            "INSERT INTO project_origins (project_id, fabric, provider_instance, native_project_key)
             VALUES (?1, ?2, ?3, ?4)",
            params![pid, fabric, provider_instance, native_project_key],
        )?;
        Ok(pid)
    }

    pub fn project_id_for_origin(
        &self,
        fabric: &str,
        provider_instance: &str,
        native_project_key: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT project_id FROM project_origins
                 WHERE fabric=?1 AND provider_instance=?2 AND native_project_key=?3",
                params![fabric, provider_instance, native_project_key],
                |r| r.get::<_, String>(0),
            )
            .ok())
    }

    /// Admit (or re-admit) a member. Upsert: preserves the original `admitted_at`,
    /// clears any prior `revoked_at`, refreshes role/source/updated_at.
    pub fn admit_member(
        &self,
        project_id: &str,
        pubkey: &str,
        role: &str,
        source: &str,
        ts: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO membership (project_id, pubkey, role, admitted_at, revoked_at, source, updated_at)
             VALUES (?1, ?2, ?3, ?5, NULL, ?4, ?5)
             ON CONFLICT(project_id, pubkey) DO UPDATE SET
               role=excluded.role, source=excluded.source, revoked_at=NULL, updated_at=excluded.updated_at",
            params![project_id, pubkey, role, source, ts],
        )?;
        Ok(())
    }

    pub fn revoke_member(&self, project_id: &str, pubkey: &str, ts: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE membership SET revoked_at=?3, updated_at=?3 WHERE project_id=?1 AND pubkey=?2",
            params![project_id, pubkey, ts],
        )?;
        Ok(())
    }

    /// The admission predicate (write-side) and roster query (read-side) in one.
    /// `Unhydrated` (no rows at all for the project) is distinct from `NotMember`
    /// (rows exist, but not this pubkey) so the materializer can quarantine
    /// inbound events until membership arrives.
    pub fn is_member_at(
        &self,
        project_id: &str,
        pubkey: &str,
        ts: u64,
    ) -> Result<MembershipDecision> {
        let project_rows: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM membership WHERE project_id=?1",
            params![project_id],
            |r| r.get(0),
        )?;
        if project_rows == 0 {
            return Ok(MembershipDecision::Unhydrated);
        }
        let row: Option<(String, u64, Option<u64>)> = self
            .conn
            .query_row(
                "SELECT role, admitted_at, revoked_at FROM membership WHERE project_id=?1 AND pubkey=?2",
                params![project_id, pubkey],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        match row {
            None => Ok(MembershipDecision::NotMember),
            Some((role, admitted_at, revoked_at)) => {
                if let Some(rev) = revoked_at {
                    if rev <= ts {
                        return Ok(MembershipDecision::Revoked);
                    }
                }
                if admitted_at <= ts {
                    Ok(MembershipDecision::Member { role })
                } else {
                    // Admitted in the future relative to ts → not yet a member.
                    Ok(MembershipDecision::NotMember)
                }
            }
        }
    }

    /// Backfill canonical project origins + membership from the legacy tables for
    /// the current NIP-29 fabric. `provider_instance` is the relay-set hash
    /// (the daemon derives it from config and passes it in — not this layer's job).
    /// Idempotent: re-running creates no duplicate origins or membership rows.
    pub fn backfill_nip29_origins(&self, provider_instance: &str, now: u64) -> Result<()> {
        const FABRIC: &str = "nip29";
        const LEGACY_FABRIC: &str = "kind1-nip29";
        // Every project slug ever observed across the legacy tables.
        let slugs: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT project FROM project_meta
                 UNION SELECT project FROM sessions
                 UNION SELECT project FROM peer_sessions
                 UNION SELECT project FROM group_members",
            )?;
            let v: Vec<String> = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            v
        };
        for slug in &slugs {
            if let Some(old_pid) =
                self.project_id_for_origin(LEGACY_FABRIC, provider_instance, slug)?
            {
                if self
                    .project_id_for_origin(FABRIC, provider_instance, slug)?
                    .is_none()
                {
                    self.conn.execute(
                        "UPDATE project_origins
                         SET fabric=?1
                         WHERE fabric=?2 AND provider_instance=?3 AND native_project_key=?4",
                        params![FABRIC, LEGACY_FABRIC, provider_instance, slug],
                    )?;
                } else {
                    self.conn.execute(
                        "DELETE FROM project_origins
                         WHERE project_id=?1 AND fabric=?2 AND provider_instance=?3 AND native_project_key=?4",
                        params![old_pid, LEGACY_FABRIC, provider_instance, slug],
                    )?;
                }
            }
            let pid = self.ensure_project_origin(FABRIC, provider_instance, slug, slug, now)?;
            // project_meta is the authority for `about`; carry it onto the row.
            if let Some(about) = self.get_project_meta(slug)? {
                self.conn.execute(
                    "UPDATE projects SET about=?2, updated_at=?3 WHERE project_id=?1",
                    params![pid, about, now],
                )?;
            }
        }
        // Mirror the nip29 roster snapshot into canonical membership.
        let members: Vec<(String, String, String)> = {
            let mut stmt = self
                .conn
                .prepare("SELECT project, pubkey, role FROM group_members")?;
            let v: Vec<(String, String, String)> = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();
            v
        };
        for (project, pubkey, role) in &members {
            if let Some(pid) = self.project_id_for_origin(FABRIC, provider_instance, project)? {
                self.admit_member(&pid, pubkey, role, "nip29-39002", now)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::MembershipDecision;
    use rusqlite::params;

    #[test]
    fn phase1_ensure_project_origin_is_idempotent() {
        let s = Store::open_memory().unwrap();
        let a = s
            .ensure_project_origin("nip29", "relayhash", "tenex-edge", "tenex-edge", 100)
            .unwrap();
        let b = s
            .ensure_project_origin("nip29", "relayhash", "tenex-edge", "tenex-edge", 200)
            .unwrap();
        assert_eq!(a, b, "same origin → same project_id");
        let count: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "no duplicate project row");
        assert_eq!(
            s.project_id_for_origin("nip29", "relayhash", "tenex-edge")
                .unwrap(),
            Some(a.clone())
        );
        // A different fabric/instance/key is a distinct project.
        let c = s
            .ensure_project_origin("nip29", "relayhash", "other", "other", 100)
            .unwrap();
        assert_ne!(a, c);
    }

    #[test]
    fn phase1_is_member_at_lifecycle() {
        let s = Store::open_memory().unwrap();
        let pid = s
            .ensure_project_origin("nip29", "ri", "p", "p", 10)
            .unwrap();
        // No membership rows at all → Unhydrated.
        assert_eq!(
            s.is_member_at(&pid, "alice", 100).unwrap(),
            MembershipDecision::Unhydrated
        );
        // Admit bob → bob is Member, alice is NotMember (rows now exist).
        s.admit_member(&pid, "bob", "member", "nip29-39002", 50)
            .unwrap();
        assert_eq!(
            s.is_member_at(&pid, "bob", 100).unwrap(),
            MembershipDecision::Member {
                role: "member".into()
            }
        );
        assert_eq!(
            s.is_member_at(&pid, "alice", 100).unwrap(),
            MembershipDecision::NotMember
        );
        // A query before bob's admission time sees him as not-yet-member.
        assert_eq!(
            s.is_member_at(&pid, "bob", 40).unwrap(),
            MembershipDecision::NotMember
        );
        // Revoke bob at t=80 → Revoked when queried at/after 80, still Member before.
        s.revoke_member(&pid, "bob", 80).unwrap();
        assert_eq!(
            s.is_member_at(&pid, "bob", 100).unwrap(),
            MembershipDecision::Revoked
        );
        assert_eq!(
            s.is_member_at(&pid, "bob", 60).unwrap(),
            MembershipDecision::Member {
                role: "member".into()
            }
        );
        // Re-admit clears the revocation.
        s.admit_member(&pid, "bob", "admin", "nip29-39002", 90)
            .unwrap();
        assert_eq!(
            s.is_member_at(&pid, "bob", 100).unwrap(),
            MembershipDecision::Member {
                role: "admin".into()
            }
        );
    }

    #[test]
    fn phase1_backfill_is_idempotent() {
        let s = Store::open_memory().unwrap();
        // Seed legacy state across the four source tables.
        s.upsert_project_meta("tenex-edge", "the edge fabric", 1)
            .unwrap();
        s.upsert_peer_session("ps-1", "pk-peer", "peer", "otherproj", "host", "", 1)
            .unwrap();
        s.replace_group_members(
            "tenex-edge",
            &[
                ("pk-1".into(), "admin".into()),
                ("pk-2".into(), "member".into()),
            ],
            1,
        )
        .unwrap();

        let projects_before = || -> i64 {
            s.conn
                .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
                .unwrap()
        };
        let members_before = || -> i64 {
            s.conn
                .query_row("SELECT COUNT(*) FROM membership", [], |r| r.get(0))
                .unwrap()
        };

        s.backfill_nip29_origins("relayhash", 100).unwrap();
        let p1 = projects_before();
        let m1 = members_before();
        assert!(p1 >= 2, "tenex-edge + otherproj origins created (got {p1})");
        assert_eq!(m1, 2, "two group_members mirrored into membership");

        // about carried from project_meta onto the canonical project row.
        let pid = s
            .project_id_for_origin("nip29", "relayhash", "tenex-edge")
            .unwrap()
            .unwrap();
        let about: Option<String> = s
            .conn
            .query_row(
                "SELECT about FROM projects WHERE project_id=?1",
                params![pid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(about.as_deref(), Some("the edge fabric"));

        // membership reflects the roster.
        assert_eq!(
            s.is_member_at(&pid, "pk-1", 200).unwrap(),
            MembershipDecision::Member {
                role: "admin".into()
            }
        );

        // Second run is a no-op at the row-count level.
        s.backfill_nip29_origins("relayhash", 300).unwrap();
        assert_eq!(
            projects_before(),
            p1,
            "no duplicate project rows on re-backfill"
        );
        assert_eq!(
            members_before(),
            m1,
            "no duplicate membership rows on re-backfill"
        );
    }
}
