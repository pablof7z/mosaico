//! Failure-only artifact retention.

use std::path::Path;

use anyhow::Result;

use super::MosaicoWorld;

impl MosaicoWorld {
    pub fn retain_failure_artifacts(&mut self, scenario: &str) {
        for backend in self.backends.values_mut() {
            backend.stop();
        }
        let Some(sandbox) = self.sandbox.as_ref() else {
            return;
        };
        let destination = artifact_path(scenario);
        let _ = std::fs::remove_dir_all(&destination);
        copy_tree(sandbox.path(), &destination)
            .unwrap_or_else(|error| panic!("retain BDD failure artifacts: {error:#}"));
        eprintln!(
            "BDD failure artifacts retained at {}",
            destination.display()
        );
    }

    pub fn remove_failure_artifacts(scenario: &str) {
        let _ = std::fs::remove_dir_all(artifact_path(scenario));
    }
}

fn artifact_path(scenario: &str) -> std::path::PathBuf {
    let slug = scenario
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/bdd-artifacts")
        .join(slug.trim_matches('-'))
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        if private_artifact(entry.file_name().to_string_lossy().as_ref()) {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&source_path, &destination_path)?;
        } else if source_path.extension().and_then(|value| value.to_str()) == Some("json") {
            copy_redacted_json(&source_path, &destination_path)?;
        } else {
            std::fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}

fn private_artifact(name: &str) -> bool {
    matches!(
        name,
        "state.db"
            | "state.db-wal"
            | "state.db-shm"
            | "nmp.redb"
            | ".claude"
            | ".claude.json"
            | ".claude.json.backup"
            | ".codex"
            | ".grok"
            | ".opencode"
    )
}

fn copy_redacted_json(source: &Path, destination: &Path) -> Result<()> {
    let bytes = std::fs::read(source)?;
    let Ok(mut document) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        std::fs::write(destination, bytes)?;
        return Ok(());
    };
    redact_json(&mut document);
    std::fs::write(destination, serde_json::to_vec_pretty(&document)?)?;
    Ok(())
}

fn redact_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                let normalized = key
                    .chars()
                    .filter(|character| character.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>();
                if [
                    "secret",
                    "privatekey",
                    "nsec",
                    "apikey",
                    "token",
                    "credential",
                ]
                .iter()
                .any(|needle| normalized.contains(needle))
                {
                    *value = serde_json::Value::String("[REDACTED]".to_string());
                } else {
                    redact_json(value);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_json(item);
            }
        }
        _ => {}
    }
}
