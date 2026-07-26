use super::*;

pub(super) fn ensure_only_tables(conn: &Connection, path: Option<&Path>) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master \
         WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let actual = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    let expected = TABLES.iter().copied().map(str::to_string).collect();
    if actual == expected {
        return Ok(());
    }
    let unexpected = actual.difference(&expected).cloned().collect::<Vec<_>>();
    let missing = expected.difference(&actual).cloned().collect::<Vec<_>>();
    non_canonical(
        path,
        format!("table set differs; unexpected={unexpected:?}, missing={missing:?}"),
    )
}

pub(super) fn ensure_table(conn: &Connection, table: &str, path: Option<&Path>) -> Result<()> {
    if table_exists(conn, table)? {
        Ok(())
    } else {
        non_canonical(path, format!("missing table `{table}`"))
    }
}

pub(super) fn ensure_absent_table(
    conn: &Connection,
    table: &str,
    path: Option<&Path>,
) -> Result<()> {
    if table_exists(conn, table)? {
        non_canonical(path, format!("removed table `{table}` is still present"))
    } else {
        Ok(())
    }
}

pub(super) fn ensure_columns(
    conn: &Connection,
    table: &str,
    required: &[&str],
    forbidden: &[&str],
    path: Option<&Path>,
) -> Result<()> {
    let columns = table_columns(conn, table)?;
    for column in required {
        if !columns.contains(*column) {
            return non_canonical(path, format!("`{table}` missing column `{column}`"));
        }
    }
    for column in forbidden {
        if columns.contains(*column) {
            return non_canonical(path, format!("`{table}` still has column `{column}`"));
        }
    }
    Ok(())
}
pub(super) fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )
    .with_context(|| format!("checking for table `{table}`"))
}

pub(super) fn table_columns(conn: &Connection, table: &str) -> Result<BTreeSet<String>> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("reading `{table}` columns"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .with_context(|| format!("collecting `{table}` columns"))?;
    Ok(columns)
}

pub(super) fn non_canonical<T>(path: Option<&Path>, reason: String) -> Result<T> {
    match path {
        Some(path) => anyhow::bail!(
            "refusing to open {}: state.db is not the current canonical schema ({reason})",
            path.display()
        ),
        None => {
            anyhow::bail!("in-memory state schema is not the current canonical schema ({reason})")
        }
    }
}
