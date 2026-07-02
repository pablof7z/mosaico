use super::*;

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
        // WAL is a startup invariant, not a best-effort hint. This project has a
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
        check_schema_version(&conn, path)?;
        // Stamped schema. We still do not run ALTER TABLE migrations, but the DB
        // is not blindly wipeable: relay_* rows are rebuildable projections while
        // sessions, aliases, identities, inbox, outbox, and project_roots are
        // local state. A missing/incompatible stamp fails loudly above.
        conn.execute_batch(SCHEMA).context("creating schema")?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .context("stamping schema version")?;
        Ok(Self { conn })
    }

    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(Self { conn })
    }

    /// `PRAGMA integrity_check` → "ok" on a healthy db, else the first problem
    /// line. Used by the concurrency/corruption test to assert no corruption.
    pub fn integrity_check(&self) -> Result<String> {
        Ok(self
            .conn
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))?)
    }
}

fn check_schema_version(conn: &Connection, path: &Path) -> Result<()> {
    let version: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .context("reading schema user_version")?;
    let has_tables = conn
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master
                WHERE type='table' AND name NOT LIKE 'sqlite_%'
            )",
            [],
            |row| row.get::<_, bool>(0),
        )
        .context("checking for existing schema tables")?;
    if version == 0 && has_tables {
        anyhow::bail!(
            "refusing to open {}: existing state.db has no schema version stamp; \
             move it aside or export non-rebuildable local state before rebuilding",
            path.display()
        );
    }
    if version != 0 && version != SCHEMA_VERSION {
        anyhow::bail!(
            "refusing to open {}: schema version {version} is incompatible with expected {SCHEMA_VERSION}",
            path.display()
        );
    }
    Ok(())
}
