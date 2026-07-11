use super::{schema, *};
use rusqlite::types::ValueRef;
use serde_json::{Map, Value};

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating parent dir for {}", path.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        // WAL + busy timeout + relaxed sync: the per-machine daemon is the sole
        // writer, but every CLI invocation still opens this file to read. WAL lets
        // readers proceed without blocking the writer; the busy timeout absorbs the
        // brief windows where a reader and the daemon overlap.
        //   journal_mode=WAL   readers don't block the writer; one writer at a time
        //   busy_timeout=5000  block up to 5s on a held lock instead of erroring
        //   synchronous=NORMAL safe under WAL; fsync only at checkpoints
        // No foreign_keys pragma: the schema declares no FK constraints.
        //
        // WAL is a startup invariant, not a best-effort hint. This channel has a
        // documented multi-writer corruption incident: ~16 per-session readers
        // plus the daemon writer rely on WAL actually being engaged. Critically,
        // `PRAGMA journal_mode=WAL` does NOT error when it cannot switch — it
        // returns the resulting mode as a row — so we must read that row back and
        // refuse to open the db unless WAL is truly in effect, rather than run in
        // a rollback-journal mode that reintroduces the corruption surface.
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
            .context("setting journal_mode=WAL")?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            anyhow::bail!(
                "refusing to open {}: journal_mode is {journal_mode:?}, not WAL — \
                 a rollback journal lets readers block the writer and reintroduces \
                 the multi-writer corruption surface",
                path.display(),
            );
        }
        conn.pragma_update(None, "synchronous", "NORMAL")
            .context("setting synchronous=NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .context("setting busy_timeout")?;
        schema::initialize_file(&conn, path)?;
        let store = Self { conn };
        store.backfill_handle_leases()?;
        store.backfill_messages_from_relay_events()?;
        Ok(store)
    }

    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::initialize_memory(&conn)?;
        let store = Self { conn };
        store.backfill_handle_leases()?;
        store.backfill_messages_from_relay_events()?;
        Ok(store)
    }

    /// `PRAGMA integrity_check` → "ok" on a healthy db, else the first problem
    /// line. Used by the concurrency/corruption test to assert no corruption.
    pub fn integrity_check(&self) -> Result<String> {
        Ok(self
            .conn
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))?)
    }

    pub fn application_table_names(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type='table' AND name NOT LIKE 'sqlite_%'
                 ORDER BY name",
            )
            .context("preparing application table inventory")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("reading application table inventory")?;
        Ok(rows)
    }

    pub fn application_table_profile(&self, table: &str) -> Result<Option<(i64, Vec<String>)>> {
        let tables = self.application_table_names()?;
        if !tables.iter().any(|name| name == table) {
            return Ok(None);
        }
        let quoted = quote_identifier(table);
        let row_count = self
            .conn
            .query_row(&format!("SELECT COUNT(*) FROM {quoted}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .with_context(|| format!("counting rows in application table `{table}`"))?;
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info({quoted})"))
            .with_context(|| format!("reading columns for application table `{table}`"))?;
        let columns = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .with_context(|| format!("collecting columns for application table `{table}`"))?;
        Ok(Some((row_count, columns)))
    }

    pub fn application_table_sample_rows(
        &self,
        table: &str,
        requested_columns: &[&str],
        limit: usize,
    ) -> Result<Option<Vec<Value>>> {
        let Some((_row_count, table_columns)) = self.application_table_profile(table)? else {
            return Ok(None);
        };
        let columns = requested_columns
            .iter()
            .filter(|column| table_columns.iter().any(|known| known == **column))
            .copied()
            .collect::<Vec<_>>();
        if columns.is_empty() {
            return Ok(Some(Vec::new()));
        }

        let column_sql = columns
            .iter()
            .map(|column| quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let order_sql = sample_order_sql(table, &table_columns)
            .map(|order| format!(" ORDER BY {order}"))
            .unwrap_or_default();
        let sql = format!(
            "SELECT {column_sql} FROM {}{order_sql} LIMIT {}",
            quote_identifier(table),
            limit.min(10)
        );
        let mut stmt = self
            .conn
            .prepare(&sql)
            .with_context(|| format!("preparing sample rows for application table `{table}`"))?;
        let rows = stmt
            .query_map([], |row| {
                let mut map = Map::new();
                for (idx, column) in columns.iter().enumerate() {
                    map.insert(column.to_string(), sqlite_value_to_json(row.get_ref(idx)?));
                }
                Ok(Value::Object(map))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .with_context(|| format!("reading sample rows for application table `{table}`"))?;
        Ok(Some(rows))
    }

    pub fn application_table_lookup_rows(
        &self,
        table: &str,
        requested_columns: &[&str],
        needle: &str,
        limit: usize,
    ) -> Result<Option<Vec<Value>>> {
        let Some((_row_count, table_columns)) = self.application_table_profile(table)? else {
            return Ok(None);
        };
        let columns = requested_columns
            .iter()
            .filter(|column| table_columns.iter().any(|known| known == **column))
            .copied()
            .collect::<Vec<_>>();
        if columns.is_empty() {
            return Ok(Some(Vec::new()));
        }

        let column_sql = columns
            .iter()
            .map(|column| quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let where_sql = columns
            .iter()
            .map(|column| {
                format!(
                    "CAST({} AS TEXT) LIKE ?1 ESCAPE '\\'",
                    quote_identifier(column)
                )
            })
            .collect::<Vec<_>>()
            .join(" OR ");
        let sql = format!(
            "SELECT {column_sql} FROM {} WHERE {where_sql} LIMIT {}",
            quote_identifier(table),
            limit.min(10)
        );
        let pattern = format!("%{}%", escape_like(needle));
        let mut stmt = self
            .conn
            .prepare(&sql)
            .with_context(|| format!("preparing lookup rows for application table `{table}`"))?;
        let rows = stmt
            .query_map(params![pattern], |row| {
                let mut map = Map::new();
                for (idx, column) in columns.iter().enumerate() {
                    map.insert(column.to_string(), sqlite_value_to_json(row.get_ref(idx)?));
                }
                Ok(Value::Object(map))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .with_context(|| format!("reading lookup rows for application table `{table}`"))?;
        Ok(Some(rows))
    }
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn sample_order_sql(table: &str, table_columns: &[String]) -> Option<&'static str> {
    match table {
        "message_recipients" if has_columns(table_columns, &["delivered_at"]) => {
            Some("CASE WHEN delivered_at > 0 THEN 0 ELSE 1 END, delivered_at DESC")
        }
        "outbox" if has_columns(table_columns, &["state", "last_error", "local_id"]) => Some(
            "CASE \
             WHEN state = 'failed' OR COALESCE(last_error, '') <> '' THEN 0 \
             WHEN state <> 'published' THEN 1 \
             ELSE 2 END, local_id ASC",
        ),
        "relay_status" if has_columns(table_columns, &["expiration", "updated_at"]) => {
            Some("expiration DESC, updated_at DESC")
        }
        "session_aliases" if has_columns(table_columns, &["session_id", "created_at"]) => Some(
            "CASE WHEN EXISTS (\
                 SELECT 1 FROM sessions s \
                 WHERE s.session_id = session_aliases.session_id AND s.alive = 1\
             ) THEN 0 ELSE 1 END, created_at DESC",
        ),
        "session_channels" if has_columns(table_columns, &["session_id", "joined_at"]) => Some(
            "CASE WHEN EXISTS (\
                 SELECT 1 FROM sessions s \
                 WHERE s.session_id = session_channels.session_id AND s.alive = 1\
             ) THEN 0 ELSE 1 END, joined_at DESC",
        ),
        "sessions" if has_columns(table_columns, &["alive", "last_seen", "created_at"]) => {
            Some("alive DESC, last_seen DESC, created_at DESC")
        }
        _ => None,
    }
}

fn has_columns(table_columns: &[String], required: &[&str]) -> bool {
    required
        .iter()
        .all(|column| table_columns.iter().any(|known| known == column))
}

fn sqlite_value_to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::from(i),
        ValueRef::Real(f) => Value::from(f),
        ValueRef::Text(bytes) => String::from_utf8_lossy(bytes).into_owned().into(),
        ValueRef::Blob(bytes) => format!("<{} byte blob>", bytes.len()).into(),
    }
}

#[cfg(test)]
mod tests;
