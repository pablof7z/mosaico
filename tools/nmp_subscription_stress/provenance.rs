use std::process::Command;

use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::Url;

const LOCKFILE: &str = include_str!("../../Cargo.lock");

#[derive(Deserialize)]
struct Lockfile {
    package: Vec<LockedPackage>,
}

#[derive(Deserialize)]
struct LockedPackage {
    name: String,
    version: String,
    source: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
}

pub(crate) fn nmp_revision() -> Result<String> {
    revision_from_lock(LOCKFILE, env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")).context(
        "the direct NMP dependency is not one unambiguous full-revision Git package; pin the candidate in Cargo.toml and regenerate Cargo.lock before running the harness",
    )
}

pub(crate) fn harness_revision() -> Result<String> {
    let root = env!("CARGO_MANIFEST_DIR");
    let status = Command::new("git")
        .args([
            "-C",
            root,
            "status",
            "--porcelain",
            "--untracked-files=normal",
        ])
        .output()
        .context("checking harness worktree status")?;
    ensure!(
        status.status.success(),
        "git status failed for harness worktree"
    );
    ensure!(
        status.stdout.is_empty(),
        "the harness worktree is dirty; commit the exact harness and dependency pins before recording results"
    );
    let output = Command::new("git")
        .args(["-C", root, "rev-parse", "HEAD"])
        .output()
        .context("resolving harness revision")?;
    ensure!(
        output.status.success(),
        "git rev-parse failed for harness worktree"
    );
    let revision = String::from_utf8(output.stdout)
        .context("harness revision was not UTF-8")?
        .trim()
        .to_owned();
    validate_revision(&revision)?;
    Ok(revision)
}

pub(crate) fn lock_hash() -> String {
    format!("{:x}", Sha256::digest(LOCKFILE.as_bytes()))
}

fn revision_from_lock(lockfile: &str, root_name: &str, root_version: &str) -> Result<String> {
    let lock: Lockfile = toml::from_str(lockfile).context("parsing Cargo.lock")?;
    let roots: Vec<_> = lock
        .package
        .iter()
        .filter(|package| {
            package.name == root_name && package.version == root_version && package.source.is_none()
        })
        .collect();
    ensure!(
        roots.len() == 1,
        "Cargo.lock must contain exactly one source-less root package {root_name} {root_version}, found {}",
        roots.len()
    );
    let direct_refs: Vec<_> = roots[0]
        .dependencies
        .iter()
        .filter(|dependency| dependency_name(dependency) == "nmp")
        .collect();
    ensure!(
        direct_refs.len() == 1,
        "root package must have exactly one direct nmp dependency, found {}",
        direct_refs.len()
    );

    let named: Vec<_> = lock
        .package
        .iter()
        .filter(|package| package.name == "nmp")
        .collect();
    ensure!(
        named.len() == 1,
        "Cargo.lock must contain exactly one package named nmp, found {}",
        named.len()
    );
    let package = named[0];
    ensure!(
        dependency_ref_matches(direct_refs[0], package),
        "the root nmp dependency reference does not resolve to the sole locked nmp package"
    );
    let source = package
        .source
        .as_deref()
        .context("the direct locked nmp package is path-sourced")?;
    revision_from_git_source(source)
}

fn dependency_name(reference: &str) -> &str {
    reference.split_whitespace().next().unwrap_or_default()
}

fn dependency_ref_matches(reference: &str, package: &LockedPackage) -> bool {
    if reference == package.name {
        return true;
    }
    let versioned = format!("{} {}", package.name, package.version);
    if reference == versioned {
        return true;
    }
    package
        .source
        .as_ref()
        .is_some_and(|source| reference == format!("{versioned} ({source})"))
}

fn revision_from_git_source(source: &str) -> Result<String> {
    let raw_url = source
        .strip_prefix("git+")
        .context("the direct locked nmp package is not git-sourced")?;
    let url = Url::parse(raw_url).context("the direct locked nmp Git source is malformed")?;
    let resolved = url
        .fragment()
        .context("the direct locked nmp Git source has no resolved revision")?;
    validate_revision(resolved).context("validating the resolved NMP revision")?;
    let query: Vec<_> = url.query_pairs().collect();
    ensure!(
        query.len() == 1 && query[0].0 == "rev",
        "the direct locked nmp Git source must use exactly one rev query"
    );
    let requested = query[0].1.as_ref();
    validate_revision(requested).context("validating the requested NMP revision")?;
    ensure!(
        requested == resolved,
        "the requested NMP revision does not equal Cargo's resolved revision"
    );
    Ok(resolved.to_owned())
}

fn validate_revision(revision: &str) -> Result<()> {
    if revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(());
    }
    bail!("NMP revision must be one full 40-character git commit, got {revision:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const REVISION: &str = "0123456789abcdef0123456789abcdef01234567";
    const OTHER_REVISION: &str = "89abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn extracts_the_exact_direct_git_dependency() {
        let lock = lockfile(
            &[&format!(
                "git+https://github.com/pablof7z/nmp.git?rev={REVISION}#{REVISION}"
            )],
            "nmp",
        );
        assert_eq!(
            revision_from_lock(&lock, "mosaico", "0.1.2").unwrap(),
            REVISION
        );
    }

    #[test]
    fn refuses_a_direct_path_package_even_with_an_indirect_git_namesake() {
        let git = format!("git+https://github.com/pablof7z/nmp.git?rev={REVISION}#{REVISION}");
        let lock = lockfile(&["path", &git], "nmp 0.1.0");
        let error = revision_from_lock(&lock, "mosaico", "0.1.2").unwrap_err();
        assert!(error.to_string().contains("exactly one package named nmp"));
    }

    #[test]
    fn refuses_two_git_packages_with_the_same_name() {
        let first = format!("git+https://github.com/pablof7z/nmp.git?rev={REVISION}#{REVISION}");
        let second =
            format!("git+https://example.test/nmp.git?rev={OTHER_REVISION}#{OTHER_REVISION}");
        let lock = lockfile(&[&first, &second], "nmp 0.1.0");
        let error = revision_from_lock(&lock, "mosaico", "0.1.2").unwrap_err();
        assert!(error.to_string().contains("exactly one package named nmp"));
    }

    #[test]
    fn refuses_a_direct_path_package_without_a_namesake() {
        let lock = lockfile(&["path"], "nmp");
        let error = revision_from_lock(&lock, "mosaico", "0.1.2").unwrap_err();
        assert!(error.to_string().contains("path-sourced"));
    }

    #[test]
    fn refuses_an_indirect_only_git_package() {
        let source = format!("git+https://github.com/pablof7z/nmp.git?rev={REVISION}#{REVISION}");
        let lock = lockfile(&[&source], "serde");
        let error = revision_from_lock(&lock, "mosaico", "0.1.2").unwrap_err();
        assert!(error.to_string().contains("direct nmp dependency"));
    }

    #[test]
    fn refuses_a_direct_reference_that_does_not_resolve_to_the_locked_package() {
        let source = format!("git+https://github.com/pablof7z/nmp.git?rev={REVISION}#{REVISION}");
        let lock = lockfile(&[&source], "nmp 9.9.9");
        let error = revision_from_lock(&lock, "mosaico", "0.1.2").unwrap_err();
        assert!(error.to_string().contains("does not resolve"));
    }

    #[test]
    fn refuses_a_short_requested_revision() {
        let source = format!("git+https://github.com/pablof7z/nmp.git?rev=01234567#{REVISION}");
        let lock = lockfile(&[&source], "nmp");
        let error = revision_from_lock(&lock, "mosaico", "0.1.2").unwrap_err();
        assert!(error.to_string().contains("requested NMP revision"));
    }

    #[test]
    fn refuses_requested_and_resolved_revision_mismatch() {
        let source =
            format!("git+https://github.com/pablof7z/nmp.git?rev={REVISION}#{OTHER_REVISION}");
        let lock = lockfile(&[&source], "nmp");
        let error = revision_from_lock(&lock, "mosaico", "0.1.2").unwrap_err();
        assert!(error.to_string().contains("does not equal"));
    }

    fn lockfile(nmp_sources: &[&str], root_dependency: &str) -> String {
        let mut lock = format!(
            "version = 4\n\n[[package]]\nname = \"mosaico\"\nversion = \"0.1.2\"\ndependencies = [\n \"{root_dependency}\",\n]\n"
        );
        for source in nmp_sources {
            lock.push_str("\n[[package]]\nname = \"nmp\"\nversion = \"0.1.0\"\n");
            if *source != "path" {
                lock.push_str(&format!("source = \"{source}\"\n"));
            }
        }
        lock
    }
}
