//! Version store impl

use super::{
    fixed_keys::FixedKeys,
    version::{IndexerStoreVersion, VersionStore},
    IndexerStore,
};
use log::trace;

impl VersionStore for IndexerStore {
    fn set_db_version_with_git_commit(
        &self,
        major: u32,
        minor: u32,
        patch: u32,
    ) -> anyhow::Result<()> {
        let version = IndexerStoreVersion {
            major,
            minor,
            patch,
            ..Default::default()
        };
        trace!("Setting database version: {version:#?}");

        if self
            .database
            .get(Self::INDEXER_STORE_VERSION_KEY)?
            .is_none()
        {
            self.database.put(
                Self::INDEXER_STORE_VERSION_KEY,
                serde_json::to_vec(&version)?,
            )?;
        }

        Ok(())
    }

    fn get_db_version(&self) -> anyhow::Result<IndexerStoreVersion> {
        trace!("Getting database version");
        self.database
            .get(Self::INDEXER_STORE_VERSION_KEY)?
            .map(|bytes| serde_json::from_slice(&bytes))
            .transpose()?
            .ok_or_else(|| anyhow::anyhow!("database has no store version"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn open_refuses_another_store_version() -> anyhow::Result<()> {
        let dir = TempDir::with_prefix(env!("CARGO_PKG_NAME"))?;
        {
            let store = IndexerStore::new(dir.path(), true)?;
            let old = IndexerStoreVersion {
                minor: IndexerStoreVersion::MINOR - 1,
                ..Default::default()
            };
            store.database.put(
                IndexerStore::INDEXER_STORE_VERSION_KEY,
                serde_json::to_vec(&old)?,
            )?;
        }

        let Err(err) = IndexerStore::new(dir.path(), false) else {
            panic!("an older store version must not open");
        };
        assert!(err.to_string().contains("has store version"), "{err}");
        Ok(())
    }

    #[test]
    fn open_accepts_own_store_version() -> anyhow::Result<()> {
        let dir = TempDir::with_prefix(env!("CARGO_PKG_NAME"))?;
        drop(IndexerStore::new(dir.path(), true)?);
        IndexerStore::new(dir.path(), false)?;
        Ok(())
    }
}
