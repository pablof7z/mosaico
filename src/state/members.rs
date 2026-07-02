//! `relay_channel_members` — kind:39001 (admins) + kind:39002 (members) cache.
//!
//! `role='admin'` is THE ONLY management authority over a channel. Materializing
//! kind:39001 replaces the admin rows; kind:39002 replaces the member rows; each
//! preserves the other set. A pubkey appears at most once per channel.

use super::*;

fn row_to_member(row: &rusqlite::Row) -> rusqlite::Result<ChannelMember> {
    Ok(ChannelMember {
        channel_h: row.get(0)?,
        pubkey: row.get(1)?,
        role: row.get(2)?,
        updated_at: row.get(3)?,
    })
}

const COLS: &str = "channel_h, pubkey, role, updated_at";

impl Store {
    /// Replace the admin set for a channel (kind:39001 materialization). Member
    /// rows are preserved; a pubkey promoted to admin supersedes its member row.
    pub fn replace_channel_admins(
        &self,
        channel_h: &str,
        admins: &[String],
        updated_at: u64,
    ) -> Result<()> {
        self.replace_role(channel_h, "admin", admins, updated_at)
    }

    /// Replace the member set for a channel (kind:39002 materialization). Admin
    /// rows are preserved; a pubkey already an admin keeps its admin role.
    pub fn replace_channel_members(
        &self,
        channel_h: &str,
        members: &[String],
        updated_at: u64,
    ) -> Result<()> {
        self.replace_role(channel_h, "member", members, updated_at)
    }

    fn replace_role(
        &self,
        channel_h: &str,
        role: &str,
        pubkeys: &[String],
        updated_at: u64,
    ) -> Result<()> {
        self.conn.execute(
            "DELETE FROM relay_channel_members WHERE channel_h=?1 AND role=?2",
            params![channel_h, role],
        )?;
        for pk in pubkeys {
            // A pubkey is unique per channel: if it already holds the other role,
            // promoting to admin wins; demoting to member only applies if absent.
            if role == "admin" {
                self.conn.execute(
                    "INSERT INTO relay_channel_members (channel_h, pubkey, role, updated_at)
                     VALUES (?1, ?2, 'admin', ?3)
                     ON CONFLICT(channel_h, pubkey) DO UPDATE SET role='admin', updated_at=?3",
                    params![channel_h, pk, updated_at],
                )?;
            } else {
                self.conn.execute(
                    "INSERT INTO relay_channel_members (channel_h, pubkey, role, updated_at)
                     VALUES (?1, ?2, 'member', ?3)
                     ON CONFLICT(channel_h, pubkey) DO UPDATE SET updated_at=?3",
                    params![channel_h, pk, updated_at],
                )?;
            }
        }
        Ok(())
    }

    /// Can this pubkey manage the channel? (role='admin')
    pub fn is_channel_admin(&self, channel_h: &str, pubkey: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM relay_channel_members
                 WHERE channel_h=?1 AND pubkey=?2 AND role='admin'",
                params![channel_h, pubkey],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Is this pubkey a member of the channel? (admin OR member)
    pub fn is_channel_member(&self, channel_h: &str, pubkey: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM relay_channel_members WHERE channel_h=?1 AND pubkey=?2",
                params![channel_h, pubkey],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// All members (admins and members) of a channel.
    pub fn list_channel_members(&self, channel_h: &str) -> Result<Vec<ChannelMember>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM relay_channel_members WHERE channel_h=?1 ORDER BY role, pubkey"
        ))?;
        let rows = stmt.query_map(params![channel_h], row_to_member)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Channels this pubkey belongs to in ANY role (admin or member). Used by the
    /// subscription planner to cover every channel a local/ordinal pubkey is in.
    pub fn list_channels_where_member(&self, pubkey: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT channel_h FROM relay_channel_members WHERE pubkey=?1 ORDER BY channel_h",
        )?;
        let rows = stmt.query_map(params![pubkey], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Upsert ONE membership row (optimistic local cache write while a relay grant
    /// is being confirmed). Admin supersedes member; a pubkey is unique per channel.
    pub fn upsert_channel_member(
        &self,
        channel_h: &str,
        pubkey: &str,
        role: &str,
        updated_at: u64,
    ) -> Result<()> {
        if role == "admin" {
            self.conn.execute(
                "INSERT INTO relay_channel_members (channel_h, pubkey, role, updated_at)
                 VALUES (?1, ?2, 'admin', ?3)
                 ON CONFLICT(channel_h, pubkey) DO UPDATE SET role='admin', updated_at=?3",
                params![channel_h, pubkey, updated_at],
            )?;
        } else {
            // Never demote an existing admin to member.
            self.conn.execute(
                "INSERT INTO relay_channel_members (channel_h, pubkey, role, updated_at)
                 VALUES (?1, ?2, 'member', ?3)
                 ON CONFLICT(channel_h, pubkey) DO UPDATE SET updated_at=?3",
                params![channel_h, pubkey, updated_at],
            )?;
        }
        Ok(())
    }

    /// Remove one cached channel membership. This is an optimistic local reflection
    /// of a management-key removal; the next relay 39002 materialization remains the
    /// source of truth.
    pub fn remove_channel_member(&self, channel_h: &str, pubkey: &str) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM relay_channel_members WHERE channel_h=?1 AND pubkey=?2",
            params![channel_h, pubkey],
        )?;
        Ok(n > 0)
    }

    /// Channels this pubkey can manage (every channel where it is an admin).
    pub fn list_channels_where_admin(&self, pubkey: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT channel_h FROM relay_channel_members
             WHERE pubkey=?1 AND role='admin' ORDER BY channel_h",
        )?;
        let rows = stmt.query_map(params![pubkey], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Number of members (admins + members) in a channel.
    pub fn count_channel_members(&self, channel_h: &str) -> Result<u64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM relay_channel_members WHERE channel_h=?1",
            params![channel_h],
            |r| r.get::<_, i64>(0),
        )? as u64)
    }
}
