//! One-way, transactional upgrades for every deployed stamped schema.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::version;

mod journal;
mod steps;

pub(crate) use journal::{load_pending_writes, replace_pending_writes};

const OLDEST_SUPPORTED_VERSION: u32 = 4;

type Apply = fn(&mut Connection, &Path) -> Result<()>;

#[derive(Clone, Copy)]
struct Migration {
    from: u32,
    apply: Apply,
}

// The array length is derived from SCHEMA_VERSION. A version bump cannot compile
// until the next migration is added, making migration coverage a release gate.
const MIGRATIONS: [Migration; (version::SCHEMA_VERSION - OLDEST_SUPPORTED_VERSION) as usize] = [
    Migration {
        from: 4,
        apply: steps::v4_to_v5,
    },
    Migration {
        from: 5,
        apply: steps::v5_to_v6,
    },
    Migration {
        from: 6,
        apply: steps::v6_to_v7,
    },
    Migration {
        from: 7,
        apply: steps::v7_to_v8,
    },
    Migration {
        from: 8,
        apply: steps::v8_to_v9,
    },
    Migration {
        from: 9,
        apply: steps::v9_to_v10,
    },
    Migration {
        from: 10,
        apply: steps::v10_to_v11,
    },
    Migration {
        from: 11,
        apply: steps::v11_to_v12,
    },
    Migration {
        from: 12,
        apply: steps::v12_to_v13,
    },
];

pub(super) fn upgrade(conn: &mut Connection, path: &Path) -> Result<u32> {
    let mut current = version::check_initial(conn, path, OLDEST_SUPPORTED_VERSION)?;
    while current != 0 && current < version::SCHEMA_VERSION {
        let index = (current - OLDEST_SUPPORTED_VERSION) as usize;
        let migration = MIGRATIONS
            .get(index)
            .with_context(|| format!("schema {current} has no migration to the next version"))?;
        if migration.from != current {
            anyhow::bail!(
                "schema migration chain is broken at version {current} (found {})",
                migration.from
            );
        }
        (migration.apply)(conn, path)
            .with_context(|| format!("migrating {} from schema {current}", path.display()))?;
        let migrated = version::read(conn)?;
        if migrated != current + 1 {
            anyhow::bail!(
                "schema migration {current} did not stamp version {}",
                current + 1
            );
        }
        current = migrated;
    }
    Ok(current)
}

#[cfg(test)]
pub(super) fn supported_versions() -> Vec<u32> {
    MIGRATIONS.iter().map(|migration| migration.from).collect()
}
