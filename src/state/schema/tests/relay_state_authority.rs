use std::path::{Path, PathBuf};

const NMP_OWNED_TABLES: &[&str] = &[
    "relay_channels",
    "relay_channel_members",
    "relay_channel_member_sets",
    "relay_profiles",
    "relay_status",
    "relay_status_sets",
    "relay_events",
    "relay_reactions",
    "messages",
    "message_recipients",
];

#[test]
fn production_sources_do_not_reintroduce_relay_sql_authority() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let files = production_rust_sources(&source_root);
    assert!(
        !files.is_empty(),
        "source architecture scan found no Rust files"
    );

    for path in files {
        let source = std::fs::read_to_string(&path).expect("read production Rust source");
        let source = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
            .to_ascii_lowercase();
        for table in NMP_OWNED_TABLES {
            for sql in [
                format!("create table {table}"),
                format!("create table if not exists {table}"),
                format!("insert into {table}"),
                format!("update {table}"),
                format!("delete from {table}"),
                format!("from {table}"),
                format!("join {table}"),
            ] {
                assert!(
                    !source.contains(&sql),
                    "{} restored NMP-owned SQL `{sql}`",
                    path.display()
                );
            }
        }
    }
}

fn production_rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(&path).expect("walk source tree") {
            let entry = entry.expect("read source entry");
            let path = entry.path();
            if path.is_dir() {
                if !is_test_or_migration_path(root, &path) {
                    pending.push(path);
                }
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && path.file_name().is_none_or(|name| name != "tests.rs")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn is_test_or_migration_path(root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(root).expect("source path under root");
    relative
        .components()
        .any(|part| part.as_os_str() == "tests")
        || relative.starts_with(Path::new("state/schema/migration"))
}
