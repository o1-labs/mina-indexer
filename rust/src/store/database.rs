//! The store's speedb handle, with atomic write scopes
//!
//! [Database] derefs to the speedb [DB] and overrides the point reads and
//! writes, so that [Database::atomically] can hold back this thread's writes
//! and commit them in one batch. Code inside a scope reads its own writes, as
//! a multi-step update (e.g. a reorg: canonicity, best ledger, counters, best
//! tip) depends on them, while other threads keep reading the committed state.

use log::error;
use speedb::{AsColumnFamilyRef, DBIterator, IteratorMode, WriteBatch, DB};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ops::Deref,
};

#[derive(Debug)]
pub struct Database(DB);

/// This thread's writes inside [Database::atomically], not yet committed
struct Pending {
    database: usize,
    batch: WriteBatch,

    /// (column family, key) -> written value, or `None` if deleted
    values: HashMap<(usize, Vec<u8>), Option<Vec<u8>>>,

    /// column families with pending writes
    column_families: HashSet<usize>,
}

thread_local! {
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
}

// Test only: makes the next reorg stop half-way, as a crash would
#[cfg(test)]
thread_local! {
    pub(crate) static STOP_MID_REORG: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Column family id of the default column family (`put`/`get` without cf)
const DEFAULT_CF: usize = 0;

/// Discards the scope's pending writes when the scope ends without a commit,
/// including by a panic (block ingestion catches panics and goes on, on the
/// same thread)
struct Scope;

impl Drop for Scope {
    fn drop(&mut self) {
        PENDING.with(|pending| pending.borrow_mut().take());
    }
}

fn cf_id(cf: &impl AsColumnFamilyRef) -> usize {
    cf.inner() as usize
}

impl Database {
    pub fn new(database: DB) -> Self {
        Self(database)
    }

    fn id(&self) -> usize {
        &self.0 as *const DB as usize
    }

    /// Runs `f` with this thread's writes to the database held back, then
    /// commits them in one batch if `f` succeeds, or discards them if it
    /// fails. Reads inside `f` see its writes. A nested call joins the outer
    /// scope.
    pub fn atomically<T>(&self, f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
        let active = PENDING.with(|pending| pending.borrow().as_ref().map(|p| p.database));
        match active {
            Some(database) if database == self.id() => return f(),
            Some(_) => anyhow::bail!("an atomic scope of another database is active"),
            None => (),
        }

        PENDING.with(|pending| {
            *pending.borrow_mut() = Some(Pending {
                database: self.id(),
                batch: WriteBatch::default(),
                values: HashMap::new(),
                column_families: HashSet::new(),
            })
        });
        let scope = Scope;
        let value = f()?;

        let pending = PENDING
            .with(|pending| pending.borrow_mut().take())
            .expect("pending writes of the atomic scope");
        drop(scope);

        self.0.write(pending.batch)?;
        Ok(value)
    }

    /// Applies `f` to this thread's pending writes to this database, if an
    /// atomic scope is active
    fn pending<R>(&self, f: impl FnOnce(&mut Pending) -> R) -> Option<R> {
        PENDING.with(|pending| {
            pending
                .borrow_mut()
                .as_mut()
                .filter(|p| p.database == self.id())
                .map(f)
        })
    }

    /// The pending value of `key`: `Some(None)` if deleted, `None` if untouched
    fn pending_value(&self, cf: usize, key: &[u8]) -> Option<Option<Vec<u8>>> {
        self.pending(|p| p.values.get(&(cf, key.to_vec())).cloned())
            .flatten()
    }

    pub fn get<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Vec<u8>>, speedb::Error> {
        match self.pending_value(DEFAULT_CF, key.as_ref()) {
            Some(value) => Ok(value),
            None => self.0.get(key),
        }
    }

    pub fn get_cf<K: AsRef<[u8]>>(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: K,
    ) -> Result<Option<Vec<u8>>, speedb::Error> {
        match self.pending_value(cf_id(cf), key.as_ref()) {
            Some(value) => Ok(value),
            None => self.0.get_cf(cf, key),
        }
    }

    pub fn put<K, V>(&self, key: K, value: V) -> Result<(), speedb::Error>
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        let (key, value) = (key.as_ref(), value.as_ref());
        match self.pending(|p| {
            p.batch.put(key, value);
            p.values
                .insert((DEFAULT_CF, key.to_vec()), Some(value.to_vec()));
            p.column_families.insert(DEFAULT_CF);
        }) {
            Some(()) => Ok(()),
            None => self.0.put(key, value),
        }
    }

    pub fn put_cf<K, V>(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: K,
        value: V,
    ) -> Result<(), speedb::Error>
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        let (key, value) = (key.as_ref(), value.as_ref());
        match self.pending(|p| {
            p.batch.put_cf(cf, key, value);
            p.values
                .insert((cf_id(cf), key.to_vec()), Some(value.to_vec()));
            p.column_families.insert(cf_id(cf));
        }) {
            Some(()) => Ok(()),
            None => self.0.put_cf(cf, key, value),
        }
    }

    pub fn delete<K: AsRef<[u8]>>(&self, key: K) -> Result<(), speedb::Error> {
        let key = key.as_ref();
        match self.pending(|p| {
            p.batch.delete(key);
            p.values.insert((DEFAULT_CF, key.to_vec()), None);
            p.column_families.insert(DEFAULT_CF);
        }) {
            Some(()) => Ok(()),
            None => self.0.delete(key),
        }
    }

    pub fn delete_cf<K: AsRef<[u8]>>(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: K,
    ) -> Result<(), speedb::Error> {
        let key = key.as_ref();
        match self.pending(|p| {
            p.batch.delete_cf(cf, key);
            p.values.insert((cf_id(cf), key.to_vec()), None);
            p.column_families.insert(cf_id(cf));
        }) {
            Some(()) => Ok(()),
            None => self.0.delete_cf(cf, key),
        }
    }

    /// Writes `batch`. A batch cannot join an atomic scope (its entries cannot
    /// be read back), so it is refused inside one.
    pub fn write(&self, batch: WriteBatch) -> anyhow::Result<()> {
        if self.pending(|_| ()).is_some() {
            anyhow::bail!("a write batch cannot be written inside an atomic scope");
        }
        Ok(self.0.write(batch)?)
    }

    /// An iterator does not see the pending writes of an atomic scope, so
    /// iterating a column family with pending writes inside one reads stale
    /// data
    pub fn iterator_cf<'a>(
        &'a self,
        cf: &impl AsColumnFamilyRef,
        mode: IteratorMode,
    ) -> DBIterator<'a> {
        if self
            .pending(|p| p.column_families.contains(&cf_id(cf)))
            .unwrap_or(false)
        {
            error!("Iterating a column family with pending writes inside an atomic scope");
            debug_assert!(
                false,
                "iterating a column family with pending writes inside an atomic scope"
            );
        }
        self.0.iterator_cf(cf, mode)
    }
}

impl Deref for Database {
    type Target = DB;

    fn deref(&self) -> &DB {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        block::{parser::BlockParser, precomputed::PrecomputedBlock, store::BlockStore},
        constants::{MAINNET_CANONICAL_THRESHOLD, MAINNET_TRANSITION_FRONTIER_K},
        state::IndexerState,
        store::IndexerStore,
    };
    use speedb::IteratorMode;
    use std::{fs, path::Path, sync::Arc};
    use tempfile::TempDir;

    const KEY: &[u8] = b"atomic-test-key";

    /// (column family, key, value)
    type Entry = (String, Box<[u8]>, Box<[u8]>);

    /// Every key and value of every column family
    fn dump(store: &IndexerStore) -> Vec<Entry> {
        let mut entries: Vec<_> = store
            .database
            .iterator(IteratorMode::Start)
            .flatten()
            .map(|(key, value)| ("default".to_string(), key, value))
            .collect();
        for name in IndexerStore::COLUMN_FAMILIES {
            let cf = store.database.cf_handle(name).expect("column family");
            entries.extend(
                store
                    .database
                    .iterator_cf(cf, IteratorMode::Start)
                    .flatten()
                    .map(|(key, value)| (name.to_string(), key, value)),
            );
        }
        entries
    }

    #[test]
    fn scope_reads_its_writes_and_commits_them_together() -> anyhow::Result<()> {
        let dir = TempDir::new()?;
        let store = Arc::new(IndexerStore::new(dir.path(), true)?);

        store.database.atomically(|| {
            store.database.put(KEY, b"1")?;
            assert_eq!(store.database.get(KEY)?.as_deref(), Some(&b"1"[..]));

            // another thread reads the committed state only
            let other = store.clone();
            let seen = std::thread::spawn(move || other.database.get(KEY).unwrap())
                .join()
                .unwrap();
            assert_eq!(seen, None);

            store.database.delete(KEY)?;
            assert_eq!(store.database.get(KEY)?, None);
            store.database.put(KEY, b"2")?;
            Ok(())
        })?;

        assert_eq!(store.database.get(KEY)?.as_deref(), Some(&b"2"[..]));
        Ok(())
    }

    #[test]
    fn failed_or_panicked_scope_writes_nothing() -> anyhow::Result<()> {
        let dir = TempDir::new()?;
        let store = IndexerStore::new(dir.path(), true)?;

        let failed: anyhow::Result<()> = store.database.atomically(|| {
            store.database.put(KEY, b"1")?;
            anyhow::bail!("stop")
        });
        assert!(failed.is_err());
        assert_eq!(store.database.get(KEY)?, None);

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.database.atomically(|| -> anyhow::Result<()> {
                store.database.put(KEY, b"1")?;
                panic!("stop")
            })
        }));
        assert!(panicked.is_err());
        assert_eq!(store.database.get(KEY)?, None);

        // the scope is gone: writes go straight to the database again
        store.database.put(KEY, b"3")?;
        assert_eq!(store.database.get(KEY)?.as_deref(), Some(&b"3"[..]));
        Ok(())
    }

    /// A best tip update that stops after its writes leaves the store as it
    /// was (#149), and one that does not applies
    #[tokio::test]
    async fn stopped_best_block_update_leaves_store_unchanged() -> anyhow::Result<()> {
        let source = Path::new("./tests/data/canonical_chain_discovery/contiguous");
        let tip_file = "mainnet-21-3NKZ6DTHiMtuaeP3tJq2xe4uujVRnGT9FX1rBiZY521uNToSppUZ.json";

        // ingest every block but the tip
        let blocks_dir = TempDir::new()?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            if entry.file_name() != tip_file {
                fs::copy(entry.path(), blocks_dir.path().join(entry.file_name()))?;
            }
        }
        let store_dir = TempDir::new()?;
        let store = Arc::new(IndexerStore::new(store_dir.path(), true)?);
        let mut state = IndexerState::new_v1(
            store.clone(),
            MAINNET_CANONICAL_THRESHOLD,
            MAINNET_TRANSITION_FRONTIER_K,
            false,
            false,
        )?;
        state
            .add_blocks(&mut BlockParser::new_testing(blocks_dir.path())?)
            .await?;

        // the tip is stored, not yet best
        let path = source.join(tip_file);
        let tip = PrecomputedBlock::parse_file(&path, crate::block::precomputed::PcbVersion::V1)?;
        store.add_block(&tip, path.metadata()?.len())?;
        let old_best = store.get_best_block_hash()?;
        assert_ne!(old_best, Some(tip.state_hash()));

        // stop half-way through the update, after the canonicity and best
        // ledger writes
        let before = dump(&store);
        super::STOP_MID_REORG.with(|stop| stop.set(true));
        let stopped = store.set_best_block(&tip.state_hash());
        super::STOP_MID_REORG.with(|stop| stop.set(false));
        assert!(stopped.is_err());
        assert!(before == dump(&store), "a stopped update changed the store");
        assert_eq!(store.get_best_block_hash()?, old_best);

        store.set_best_block(&tip.state_hash())?;
        assert_eq!(store.get_best_block_hash()?, Some(tip.state_hash()));
        assert!(before != dump(&store));
        Ok(())
    }
}
