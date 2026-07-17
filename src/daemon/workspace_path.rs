//! Daemon-owned channel ↔ local filesystem resolution.
//!
//! `workspace_roots` remains the machine-local persistence input. Callers do
//! not query it directly: ancestry and host-path lookup are one policy here.

use anyhow::Result;

pub(crate) fn channel_for_path(path: &std::path::Path) -> Result<String> {
    crate::workspace::resolve(path)
}

pub(crate) fn channel_for_path_or_bail(path: &std::path::Path) -> Result<String> {
    crate::workspace::resolve_or_bail(path)
}

pub(crate) fn root_path_for(path: &std::path::Path) -> Result<Option<std::path::PathBuf>> {
    crate::workspace::workspace_dir(path)
}

pub(crate) struct WorkspacePathResolver<'a> {
    store: &'a crate::state::Store,
}

impl<'a> WorkspacePathResolver<'a> {
    pub(crate) fn new(store: &'a crate::state::Store) -> Self {
        Self { store }
    }

    pub(crate) fn root_for_channel(&self, channel_h: &str) -> Result<String> {
        root_for_reader(self.store.reader(), channel_h)
    }

    pub(crate) fn path_for_channel(&self, channel_h: &str) -> Result<Option<String>> {
        let root = self.root_for_channel(channel_h)?;
        self.store.workspace_path(&root)
    }

    pub(crate) fn bindings(&self) -> Result<Vec<crate::state::WorkspaceBinding>> {
        self.store.list_workspace_bindings()
    }

    pub(crate) fn root_for_session(&self, session: &crate::state::Session) -> Result<String> {
        self.root_for_channel(&session.channel_h)
    }

    pub(crate) fn bind_root_path(&self, root: &str, path: &std::path::Path, at: u64) -> Result<()> {
        self.store
            .upsert_workspace(root, &path.to_string_lossy(), at)
    }
}

pub(crate) fn root_for_reader(
    store: crate::state::StoreReader<'_>,
    channel_h: &str,
) -> Result<String> {
    if let Some(root) = store.root_channel_of(channel_h)? {
        return Ok(root);
    }
    if store.workspace_path(channel_h)?.is_some() {
        return Ok(channel_h.to_string());
    }
    Err(anyhow::anyhow!(
        "workspace resolver: incomplete ancestry for channel {channel_h:?}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descendant_resolves_through_root_binding() {
        let store = crate::state::Store::open_memory().unwrap();
        store.upsert_channel("root", "root", "", "", 1).unwrap();
        store
            .upsert_channel("child", "child", "", "root", 1)
            .unwrap();
        let resolver = WorkspacePathResolver::new(&store);
        resolver
            .bind_root_path("root", std::path::Path::new("/repo"), 2)
            .unwrap();

        assert_eq!(resolver.root_for_channel("child").unwrap(), "root");
        assert_eq!(
            resolver.path_for_channel("child").unwrap().as_deref(),
            Some("/repo")
        );
    }

    #[test]
    fn broken_ancestry_is_an_error_instead_of_a_root_fallback() {
        let store = crate::state::Store::open_memory().unwrap();
        store
            .upsert_channel("child", "child", "", "missing-parent", 1)
            .unwrap();
        let resolver = WorkspacePathResolver::new(&store);

        let error = resolver.root_for_channel("child").unwrap_err();
        assert!(
            error.to_string().contains("incomplete ancestry"),
            "error = {error:#}"
        );
    }

    #[test]
    fn binding_enumeration_stays_behind_the_resolver() {
        let store = crate::state::Store::open_memory().unwrap();
        let resolver = WorkspacePathResolver::new(&store);
        resolver
            .bind_root_path("zeta", std::path::Path::new("/work/zeta"), 1)
            .unwrap();
        resolver
            .bind_root_path("alpha", std::path::Path::new("/work/alpha"), 2)
            .unwrap();

        let bindings = resolver.bindings().unwrap();
        assert_eq!(
            bindings
                .iter()
                .map(|binding| binding.channel_h.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "zeta"]
        );
    }
}
