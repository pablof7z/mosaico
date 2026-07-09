use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_COMMAND_NAME: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchCommand {
    pub name: String,
    pub argv: Vec<String>,
}

impl LaunchCommand {
    pub fn new(name: impl Into<String>, argv: Vec<String>) -> Option<Self> {
        let name = name.into().trim().to_string();
        if name.is_empty() || argv.is_empty() {
            return None;
        }
        Some(Self { name, argv })
    }

    pub fn default(argv: Vec<String>) -> Option<Self> {
        Self::new(DEFAULT_COMMAND_NAME, argv)
    }

    pub fn display(&self) -> String {
        format!("{}: {}", self.name, self.argv.join(" "))
    }
}

pub(super) fn normalize_commands(commands: Vec<LaunchCommand>) -> Vec<LaunchCommand> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for command in commands {
        let Some(command) = LaunchCommand::new(command.name, command.argv) else {
            continue;
        };
        if seen.insert(command.name.clone()) {
            out.push(command);
        }
    }
    out
}

pub fn adapt_argv_for_slug(argv: &[String], source_slug: &str, target_slug: &str) -> Vec<String> {
    argv.iter()
        .enumerate()
        .map(|(i, arg)| {
            if i == 0 {
                // argv[0] is the binary executable — never rewrite it to the
                // target slug just because an agent's slug happens to match the
                // binary name (e.g. agent "codex" with command `codex --yolo`
                // must not become `planner --yolo`). Only honor explicit {slug}
                // placeholders, which are user-intentional templating.
                if arg.contains("{slug}") {
                    arg.replace("{slug}", target_slug)
                } else {
                    arg.clone()
                }
            } else {
                adapt_arg_for_slug(arg, source_slug, target_slug)
            }
        })
        .collect()
}

fn adapt_arg_for_slug(arg: &str, source_slug: &str, target_slug: &str) -> String {
    if arg.contains("{slug}") {
        return arg.replace("{slug}", target_slug);
    }
    if arg == source_slug {
        return target_slug.to_string();
    }
    replace_path_stem(arg, source_slug, target_slug).unwrap_or_else(|| arg.to_string())
}

fn replace_path_stem(arg: &str, source_slug: &str, target_slug: &str) -> Option<String> {
    let path = Path::new(arg);
    let stem = path.file_stem()?.to_str()?;
    if stem != source_slug {
        return None;
    }
    let new_name = match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{target_slug}.{ext}"),
        None => target_slug.to_string(),
    };
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    Some(match parent {
        Some(parent) => parent.join(new_name).to_string_lossy().to_string(),
        None => PathBuf::from(new_name).to_string_lossy().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn normalize_drops_empty_and_duplicate_commands() {
        let commands = vec![
            LaunchCommand {
                name: " default ".into(),
                argv: argv(&["claude"]),
            },
            LaunchCommand {
                name: "default".into(),
                argv: argv(&["codex"]),
            },
            LaunchCommand {
                name: "".into(),
                argv: argv(&["ignored"]),
            },
            LaunchCommand {
                name: "empty".into(),
                argv: vec![],
            },
        ];

        let got = normalize_commands(commands);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "default");
        assert_eq!(got[0].argv, argv(&["claude"]));
    }

    #[test]
    fn adapt_uses_explicit_placeholder() {
        let got = adapt_argv_for_slug(
            &argv(&["runner", "--file", "/tmp/{slug}.json"]),
            "source",
            "target",
        );
        assert_eq!(got, argv(&["runner", "--file", "/tmp/target.json"]));
    }

    #[test]
    fn adapt_replaces_exact_slug_token() {
        let got = adapt_argv_for_slug(&argv(&["claude", "-p", "source"]), "source", "target");
        assert_eq!(got, argv(&["claude", "-p", "target"]));
    }

    #[test]
    fn adapt_replaces_path_filename_stem_only() {
        let got = adapt_argv_for_slug(
            &argv(&["runner", "--file", "/home/me/source.json"]),
            "source",
            "target",
        );
        assert_eq!(got, argv(&["runner", "--file", "/home/me/target.json"]));
    }

    #[test]
    fn adapt_keeps_binary_when_source_slug_matches_binary() {
        // Agent "codex" with command `codex --yolo` adapted to "planner" must
        // keep `codex` as argv[0] — only explicit {slug} placeholders or
        // slug-matching path stems further down the argv get rewritten.
        let got = adapt_argv_for_slug(&argv(&["codex", "--yolo"]), "codex", "planner");
        assert_eq!(got, argv(&["codex", "--yolo"]));
    }

    #[test]
    fn adapt_expands_explicit_placeholder_in_binary_position() {
        let got = adapt_argv_for_slug(&argv(&["{slug}"]), "codex", "planner");
        assert_eq!(got, argv(&["planner"]));
    }

    #[test]
    fn adapt_does_not_broadly_rewrite_substrings() {
        let got = adapt_argv_for_slug(
            &argv(&["runner", "--file", "/home/source-data/source-extra.json"]),
            "source",
            "target",
        );
        assert_eq!(
            got,
            argv(&["runner", "--file", "/home/source-data/source-extra.json"])
        );
    }
}
