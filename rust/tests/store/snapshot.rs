use crate::helpers::store::*;
use mina_indexer::{
    block::{precomputed::PrecomputedBlock, store::BlockStore},
    store::{
        restore_snapshot, version::IndexerStoreVersion, IndexerStore, SnapshotManifest,
        SNAPSHOT_MANIFEST,
    },
};
use std::path::{Path, PathBuf};

const BLOCK: &str = "./tests/data/sequential_blocks/mainnet-105489-3NK4huLvUDiL4XuCUcyrWCKynmvhqfKsx5h2MfBXVVUq2Qwzi5uT.json";

/// A store whose best tip is [BLOCK], snapshotted to `<tmp>/snapshot.tar`
fn snapshot_with_best_block(tmp: &Path) -> anyhow::Result<(PathBuf, PrecomputedBlock)> {
    let db = IndexerStore::new(&tmp.join("db"), true)?;
    let block = PrecomputedBlock::from_path(Path::new(BLOCK))?;

    db.add_block(&block, 0)?;
    db.set_best_block(&block.state_hash())?;

    let snapshot = tmp.join("snapshot.tar");
    db.create_snapshot(&snapshot)?;
    Ok((snapshot, block))
}

#[test]
fn snapshot_manifest_records_best_tip() -> anyhow::Result<()> {
    let tmp = setup_new_db_dir("snapshot-manifest")?;
    let (snapshot, block) = snapshot_with_best_block(tmp.path())?;

    let restore_dir = tmp.path().join("restored");
    let genesis = block.genesis_state_hash().0;
    let manifest = restore_snapshot(&snapshot, &restore_dir, Some(&genesis))?;

    assert_eq!(manifest, SnapshotManifest::read(&restore_dir)?);
    assert_eq!(manifest.best_block_height, Some(105489));
    assert_eq!(manifest.best_block_hash, Some(block.state_hash().0));
    assert_eq!(manifest.genesis_state_hash, Some(genesis));
    assert!(manifest.store_version.matches_binary());

    // the restored directory is an openable store with the same best tip
    let restored = IndexerStore::new(&restore_dir, false)?;
    assert_eq!(restored.get_best_block_hash()?, Some(block.state_hash()));
    Ok(())
}

#[test]
fn restore_rejects_another_chain() -> anyhow::Result<()> {
    let tmp = setup_new_db_dir("snapshot-wrong-genesis")?;
    let (snapshot, _) = snapshot_with_best_block(tmp.path())?;

    let restore_dir = tmp.path().join("restored");
    let err = restore_snapshot(
        &snapshot,
        &restore_dir,
        Some("3NLT7n4LiVo6U4LXr9BjCEp9712fP61uXkRC6hnRnB682A8f4HrJ"),
    )
    .unwrap_err();

    assert!(format!("{err:#}").contains("genesis hash"), "{err:#}");
    assert!(!restore_dir.exists(), "an unusable restore must be removed");
    Ok(())
}

#[test]
fn restore_rejects_empty_store_when_genesis_expected() -> anyhow::Result<()> {
    let tmp = setup_new_db_dir("snapshot-empty")?;
    let db = IndexerStore::new(&tmp.path().join("db"), true)?;
    let snapshot = tmp.path().join("snapshot.tar");
    db.create_snapshot(&snapshot)?;

    // without an expected genesis an empty store restores ...
    let manifest = restore_snapshot(&snapshot, &tmp.path().join("any"), None)?;
    assert_eq!(manifest.best_block_height, None);

    // ... but it cannot be vouched for as a given chain
    let restore_dir = tmp.path().join("restored");
    assert!(restore_snapshot(&snapshot, &restore_dir, Some("3NK...")).is_err());
    assert!(!restore_dir.exists());
    Ok(())
}

#[test]
fn restore_rejects_other_store_version() -> anyhow::Result<()> {
    let tmp = setup_new_db_dir("snapshot-old-version")?;
    let (snapshot, _) = snapshot_with_best_block(tmp.path())?;

    // rewrite the snapshot with a manifest from an older store
    let unpacked = tmp.path().join("unpacked");
    restore_snapshot(&snapshot, &unpacked, None)?;
    let mut manifest = SnapshotManifest::read(&unpacked)?;
    manifest.store_version = IndexerStoreVersion {
        minor: IndexerStoreVersion::MINOR - 1,
        ..manifest.store_version
    };
    std::fs::write(
        unpacked.join(SNAPSHOT_MANIFEST),
        serde_json::to_vec(&manifest)?,
    )?;
    let old = tmp.path().join("old.tar");
    let mut builder = tar::Builder::new(std::fs::File::create(&old)?);
    for entry in std::fs::read_dir(&unpacked)?.flatten() {
        builder.append_path_with_name(entry.path(), entry.file_name())?;
    }
    builder.finish()?;

    let restore_dir = tmp.path().join("restored");
    let err = restore_snapshot(&old, &restore_dir, None).unwrap_err();
    assert!(format!("{err:#}").contains("store version"), "{err:#}");
    assert!(!restore_dir.exists());
    Ok(())
}

#[test]
fn restore_rejects_snapshot_without_manifest() -> anyhow::Result<()> {
    let tmp = setup_new_db_dir("snapshot-legacy")?;
    let (snapshot, _) = snapshot_with_best_block(tmp.path())?;

    // a snapshot made before manifests existed
    let unpacked = tmp.path().join("unpacked");
    restore_snapshot(&snapshot, &unpacked, None)?;
    std::fs::remove_file(unpacked.join(SNAPSHOT_MANIFEST))?;
    let legacy = tmp.path().join("legacy.tar");
    let mut builder = tar::Builder::new(std::fs::File::create(&legacy)?);
    for entry in std::fs::read_dir(&unpacked)?.flatten() {
        builder.append_path_with_name(entry.path(), entry.file_name())?;
    }
    builder.finish()?;

    let restore_dir = tmp.path().join("restored");
    assert!(restore_snapshot(&legacy, &restore_dir, None).is_err());
    assert!(!restore_dir.exists());
    Ok(())
}
