//! Locate the shell profile Mosaico may own a wrapper block inside.
//!
//! Kept apart from the block rewriting so that "which file, which syntax" stays
//! a pure mapping that can be tested without touching a real home directory.

use anyhow::{bail, Context as _, Result};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Alias syntax accepted by the profile's shell family.
#[derive(Clone, Copy)]
pub(super) enum Syntax {
    Posix,
    Fish,
}

pub(super) struct Profile {
    pub path: PathBuf,
    pub syntax: Syntax,
}

/// The profile for the shell the operator actually logs in with.
pub(super) fn current_profile() -> Result<Profile> {
    let home = super::super::config::home_dir()?;
    let shell = std::env::var("SHELL").context(
        "SHELL is not set; pass setup options non-interactively or set SHELL to select a profile",
    )?;
    profile_for_shell(
        Path::new(&shell),
        &home,
        std::env::var_os("ZDOTDIR").as_deref(),
    )
}

fn profile_for_shell(shell: &Path, home: &Path, zdotdir: Option<&OsStr>) -> Result<Profile> {
    let name = shell
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    match name {
        "zsh" => Ok(Profile {
            path: zdotdir
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.to_path_buf())
                .join(".zshrc"),
            syntax: Syntax::Posix,
        }),
        "bash" => Ok(Profile {
            path: home.join(".bashrc"),
            syntax: Syntax::Posix,
        }),
        "sh" | "dash" | "ksh" => Ok(Profile {
            path: home.join(".profile"),
            syntax: Syntax::Posix,
        }),
        "fish" => Ok(Profile {
            path: home.join(".config/fish/config.fish"),
            syntax: Syntax::Fish,
        }),
        _ => bail!("unsupported login shell {shell:?}; supported: zsh, bash, sh, dash, ksh, fish"),
    }
}

/// Every profile a previous `mosaico setup` could have written a block into.
/// Uninstall sweeps all of them so a shell change cannot strand a wrapper.
pub(super) fn known_profiles() -> Result<Vec<Profile>> {
    let home = super::super::config::home_dir()?;
    let mut profiles = vec![
        posix(home.join(".zshrc")),
        posix(home.join(".bashrc")),
        posix(home.join(".bash_profile")),
        posix(home.join(".profile")),
        Profile {
            path: home.join(".config/fish/config.fish"),
            syntax: Syntax::Fish,
        },
    ];
    if let Some(dir) = std::env::var_os("ZDOTDIR").filter(|path| !path.is_empty()) {
        profiles.push(posix(PathBuf::from(dir).join(".zshrc")));
    }
    let mut seen = HashSet::new();
    profiles.retain(|profile| seen.insert(profile.path.clone()));
    Ok(profiles)
}

fn posix(path: PathBuf) -> Profile {
    Profile {
        path,
        syntax: Syntax::Posix,
    }
}

/// A missing profile reads as empty; setup creates it when it writes the block.
pub(super) fn read_optional(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_names_map_to_their_native_profiles() {
        let home = Path::new("/Users/alice");
        let path = |shell: &str| profile_for_shell(Path::new(shell), home, None).map(|p| p.path);

        assert_eq!(path("/bin/zsh").unwrap(), home.join(".zshrc"));
        assert_eq!(path("/bin/bash").unwrap(), home.join(".bashrc"));
        assert_eq!(
            path("/usr/bin/fish").unwrap(),
            home.join(".config/fish/config.fish")
        );
        assert!(path("/bin/tcsh").is_err());
    }

    #[test]
    fn zdotdir_moves_the_zsh_profile() {
        let home = Path::new("/Users/alice");
        let profile =
            profile_for_shell(Path::new("/bin/zsh"), home, Some(OsStr::new("/cfg/zsh"))).unwrap();

        assert_eq!(profile.path, Path::new("/cfg/zsh/.zshrc"));
    }
}
