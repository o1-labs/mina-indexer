//! Zkapp store trait implementation

use super::{
    zkapp::{VerificationKeyChange, ZkappStore},
    IndexerStore, Result,
};
use crate::{
    base::{public_key::PublicKey, state_hash::StateHash},
    ledger::{
        account::{Permissions, Timing},
        token::{TokenAddress, TokenSymbol},
    },
    mina_blocks::v2::{VerificationKey, ZkappState, ZkappUri},
    store::column_families::ColumnFamilyHelpers,
    utility::store::{
        common::{from_be_bytes, state_hash_suffix, U32_LEN},
        zkapp::{
            zkapp_permissions_key, zkapp_permissions_num_key, zkapp_state_key, zkapp_state_num_key,
            zkapp_timing_key, zkapp_timing_num_key, zkapp_token_symbol_key,
            zkapp_token_symbol_num_key, zkapp_uri_key, zkapp_uri_num_key,
            zkapp_verification_key_key, zkapp_verification_key_num_key, zkapp_vk_history_key,
        },
    },
};
use log::trace;
use speedb::{DBIterator, Direction, IteratorMode};

pub mod action_store_impl;
pub mod event_store_impl;
pub mod token_store_impl;

impl ZkappStore for IndexerStore {
    ///////////////
    // app state //
    ///////////////

    fn get_zkapp_state_num(&self, token: &TokenAddress, pk: &PublicKey) -> Result<Option<u32>> {
        trace!("Getting zkapp state count for token {} pk {}", token, pk);

        Ok(self
            .database
            .get_cf(self.zkapp_state_num_cf(), zkapp_state_num_key(token, pk))?
            .map(from_be_bytes))
    }

    fn get_zkapp_state(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        index: u32,
    ) -> Result<Option<ZkappState>> {
        trace!(
            "Getting zkapp state for token {} pk {} index {}",
            token,
            pk,
            index
        );

        Ok(self
            .database
            .get_cf(self.zkapp_state_cf(), zkapp_state_key(token, pk, index))?
            .map(|bytes| serde_json::from_slice(&bytes).expect("zkapp state")))
    }

    fn add_zkapp_state(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        app_state: &ZkappState,
    ) -> Result<()> {
        trace!(
            "Adding zkapp state for token {} pk {}: {:?}",
            token,
            pk,
            app_state
        );

        // get index & update count
        let index = self.get_zkapp_state_num(token, pk)?.unwrap_or_default();
        self.database.put_cf(
            self.zkapp_state_num_cf(),
            zkapp_state_num_key(token, pk),
            (index + 1).to_be_bytes(),
        )?;

        // write entry
        Ok(self.database.put_cf(
            self.zkapp_state_cf(),
            zkapp_state_key(token, pk, index),
            serde_json::to_vec(app_state)?,
        )?)
    }

    fn remove_last_zkapp_state(&self, token: &TokenAddress, pk: &PublicKey) -> Result<ZkappState> {
        trace!("Removing last zkapp state for token {} pk {}", token, pk);

        let count = self
            .get_zkapp_state_num(token, pk)?
            .expect("zkapp state count");
        assert_ne!(count, 0);

        let index = count - 1;
        let zkapp_state = self
            .get_zkapp_state(token, pk, index)?
            .expect("last zkapp state");

        // delete entry
        self.database
            .delete_cf(self.zkapp_state_cf(), zkapp_state_key(token, pk, index))?;

        // update count
        self.database.put_cf(
            self.zkapp_state_num_cf(),
            zkapp_state_num_key(token, pk),
            index.to_be_bytes(),
        )?;

        Ok(zkapp_state)
    }

    /////////////////
    // permissions //
    /////////////////

    fn get_zkapp_permissions_num(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
    ) -> Result<Option<u32>> {
        trace!(
            "Getting zkapp permissions count for token {} pk {}",
            token,
            pk
        );

        Ok(self
            .database
            .get_cf(
                self.zkapp_permissions_num_cf(),
                zkapp_permissions_num_key(token, pk),
            )?
            .map(from_be_bytes))
    }

    fn get_zkapp_permissions(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        index: u32,
    ) -> Result<Option<Permissions>> {
        trace!(
            "Getting zkapp permissions for token {} pk {} index {}",
            token,
            pk,
            index
        );

        Ok(self
            .database
            .get_cf(
                self.zkapp_permissions_cf(),
                zkapp_permissions_key(token, pk, index),
            )?
            .map(|bytes| serde_json::from_slice(&bytes).expect("zkapp permissions")))
    }

    fn add_zkapp_permissions(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        permissions: &Permissions,
    ) -> Result<()> {
        trace!(
            "Adding zkapp permissions for token {} pk {}: {:?}",
            token,
            pk,
            permissions
        );

        // get index & update count
        let index = self
            .get_zkapp_permissions_num(token, pk)?
            .unwrap_or_default();
        self.database.put_cf(
            self.zkapp_permissions_num_cf(),
            zkapp_permissions_num_key(token, pk),
            (index + 1).to_be_bytes(),
        )?;

        // write entry
        Ok(self.database.put_cf(
            self.zkapp_permissions_cf(),
            zkapp_permissions_key(token, pk, index),
            serde_json::to_vec(permissions)?,
        )?)
    }

    fn remove_last_zkapp_permissions(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
    ) -> Result<Permissions> {
        trace!(
            "Removing last zkapp permissions for token {} pk {}",
            token,
            pk
        );

        let count = self
            .get_zkapp_permissions_num(token, pk)?
            .expect("zkapp permissions count");
        assert_ne!(count, 0);

        let index = count - 1;
        let permissions = self
            .get_zkapp_permissions(token, pk, index)?
            .expect("last zkapp permissions");

        // delete entry
        self.database.delete_cf(
            self.zkapp_permissions_cf(),
            zkapp_permissions_key(token, pk, index),
        )?;

        // update count
        self.database.put_cf(
            self.zkapp_permissions_num_cf(),
            zkapp_permissions_num_key(token, pk),
            index.to_be_bytes(),
        )?;

        Ok(permissions)
    }

    //////////////////////
    // verification key //
    //////////////////////

    fn get_zkapp_verification_key_num(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
    ) -> Result<Option<u32>> {
        trace!(
            "Getting zkapp verification key count for token {} pk {}",
            token,
            pk
        );

        Ok(self
            .database
            .get_cf(
                self.zkapp_verification_key_num_cf(),
                zkapp_verification_key_num_key(token, pk),
            )?
            .map(from_be_bytes))
    }

    fn get_zkapp_verification_key(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        index: u32,
    ) -> Result<Option<VerificationKey>> {
        trace!(
            "Getting zkapp verification key for token {} pk {} index {}",
            token,
            pk,
            index
        );

        Ok(self
            .database
            .get_cf(
                self.zkapp_verification_key_cf(),
                zkapp_verification_key_key(token, pk, index),
            )?
            .map(|bytes| serde_json::from_slice(&bytes).expect("zkapp permissions")))
    }

    fn add_zkapp_verification_key(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        verification_key: &VerificationKey,
    ) -> Result<()> {
        trace!(
            "Adding zkapp verification key for token {} pk {}: {:?}",
            token,
            pk,
            verification_key
        );

        // get index & update count
        let index = self
            .get_zkapp_verification_key_num(token, pk)?
            .unwrap_or_default();
        self.database.put_cf(
            self.zkapp_verification_key_num_cf(),
            zkapp_verification_key_num_key(token, pk),
            (index + 1).to_be_bytes(),
        )?;

        // write entry
        Ok(self.database.put_cf(
            self.zkapp_verification_key_cf(),
            zkapp_verification_key_key(token, pk, index),
            serde_json::to_vec(verification_key)?,
        )?)
    }

    fn remove_last_zkapp_verification_key(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
    ) -> Result<VerificationKey> {
        trace!(
            "Removing last zkapp verification key for token {} pk {}",
            token,
            pk
        );

        let count = self
            .get_zkapp_verification_key_num(token, pk)?
            .unwrap_or_default();
        assert_ne!(count, 0);

        let index = count - 1;
        let verification_key = self
            .get_zkapp_verification_key(token, pk, index)?
            .expect("last zkapp verification key");

        // delete entry
        self.database.delete_cf(
            self.zkapp_verification_key_cf(),
            zkapp_verification_key_key(token, pk, index),
        )?;

        // update count
        self.database.put_cf(
            self.zkapp_verification_key_num_cf(),
            zkapp_verification_key_num_key(token, pk),
            index.to_be_bytes(),
        )?;

        Ok(verification_key)
    }

    fn add_zkapp_verification_key_change(
        &self,
        pk: &PublicKey,
        block_height: u32,
        state_hash: &StateHash,
        change: &VerificationKeyChange,
    ) -> Result<()> {
        trace!(
            "Adding zkapp verification key change for pk {} at height {} block {}",
            pk,
            block_height,
            state_hash
        );

        Ok(self.database.put_cf(
            self.zkapp_verification_key_history_cf(),
            zkapp_vk_history_key(pk, block_height, state_hash),
            serde_json::to_vec(change)?,
        )?)
    }

    fn remove_zkapp_verification_key_change(
        &self,
        pk: &PublicKey,
        block_height: u32,
        state_hash: &StateHash,
    ) -> Result<()> {
        trace!(
            "Removing zkapp verification key change for pk {} at height {} block {}",
            pk,
            block_height,
            state_hash
        );

        Ok(self.database.delete_cf(
            self.zkapp_verification_key_history_cf(),
            zkapp_vk_history_key(pk, block_height, state_hash),
        )?)
    }

    fn get_last_zkapp_verification_key_change(
        &self,
        pk: &PublicKey,
    ) -> Result<Option<(u32, StateHash, VerificationKeyChange)>> {
        // Reverse-scan this pk's prefix; the first row is the highest block
        // height (== the most recent change).
        let next = self
            .zkapp_verification_key_history_iterator(pk, Direction::Reverse)
            .flatten()
            .next();
        if let Some((key, value)) = next {
            // guard against landing on another pk's row (empty history)
            if key.len() >= PublicKey::LEN && key[..PublicKey::LEN] == *pk.0.as_bytes() {
                let block_height = from_be_bytes(key[PublicKey::LEN..][..U32_LEN].to_vec());
                let state_hash = state_hash_suffix(&key)?;
                let change: VerificationKeyChange = serde_json::from_slice(&value)?;
                return Ok(Some((block_height, state_hash, change)));
            }
        }
        Ok(None)
    }

    fn zkapp_verification_key_history_iterator(
        &self,
        pk: &PublicKey,
        direction: Direction,
    ) -> DBIterator<'_> {
        // Seek to this pk's rows. Reverse starts just past the pk's last possible
        // key (max height + max state hash) so iteration yields newest-first.
        let mut start = [0u8; PublicKey::LEN + U32_LEN + StateHash::LEN];
        start[..PublicKey::LEN].copy_from_slice(pk.0.as_bytes());
        if let Direction::Reverse = direction {
            start[PublicKey::LEN..][..U32_LEN].copy_from_slice(&u32::MAX.to_be_bytes());
            start[PublicKey::LEN..][U32_LEN..].fill(u8::MAX);
        }

        self.database.iterator_cf(
            self.zkapp_verification_key_history_cf(),
            IteratorMode::From(&start, direction),
        )
    }

    ///////////////
    // zkapp uri //
    ///////////////

    fn get_zkapp_uri_num(&self, token: &TokenAddress, pk: &PublicKey) -> Result<Option<u32>> {
        trace!("Getting zkapp uri count for token {} pk {}", token, pk);

        Ok(self
            .database
            .get_cf(self.zkapp_uri_num_cf(), zkapp_uri_num_key(token, pk))?
            .map(from_be_bytes))
    }

    fn get_zkapp_uri(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        index: u32,
    ) -> Result<Option<ZkappUri>> {
        trace!(
            "Getting zkapp uri for token {} pk {} index {}",
            token,
            pk,
            index
        );

        Ok(self
            .database
            .get_cf(self.zkapp_uri_cf(), zkapp_uri_key(token, pk, index))?
            .map(|bytes| String::from_utf8(bytes).expect("zkapp uri").into()))
    }

    fn add_zkapp_uri(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        zkapp_uri: &ZkappUri,
    ) -> Result<()> {
        trace!(
            "Adding zkapp uri for token {} pk {}: {:?}",
            token,
            pk,
            zkapp_uri
        );

        // get index & update count
        let index = self.get_zkapp_uri_num(token, pk)?.unwrap_or_default();
        self.database.put_cf(
            self.zkapp_uri_num_cf(),
            zkapp_uri_num_key(token, pk),
            (index + 1).to_be_bytes(),
        )?;

        // write entry
        Ok(self.database.put_cf(
            self.zkapp_uri_cf(),
            zkapp_uri_key(token, pk, index),
            zkapp_uri.0.as_bytes(),
        )?)
    }

    fn remove_last_zkapp_uri(&self, token: &TokenAddress, pk: &PublicKey) -> Result<ZkappUri> {
        trace!("Removing last zkapp uri for token {} pk {}", token, pk);

        let count = self.get_zkapp_uri_num(token, pk)?.expect("zkapp uri count");
        assert_ne!(count, 0);

        let index = count - 1;
        let zkapp_uri = self
            .get_zkapp_uri(token, pk, index)?
            .expect("last zkapp uri");

        // delete entry
        self.database
            .delete_cf(self.zkapp_uri_cf(), zkapp_uri_key(token, pk, index))?;

        // update count
        self.database.put_cf(
            self.zkapp_uri_num_cf(),
            zkapp_uri_num_key(token, pk),
            index.to_be_bytes(),
        )?;

        Ok(zkapp_uri)
    }

    //////////////////
    // token symbol //
    //////////////////

    fn get_zkapp_token_symbol_num(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
    ) -> Result<Option<u32>> {
        trace!(
            "Getting zkapp token symbol count for token {} pk {}",
            token,
            pk
        );

        Ok(self
            .database
            .get_cf(
                self.zkapp_token_symbol_num_cf(),
                zkapp_token_symbol_num_key(token, pk),
            )?
            .map(from_be_bytes))
    }

    fn get_zkapp_token_symbol(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        index: u32,
    ) -> Result<Option<TokenSymbol>> {
        trace!(
            "Getting zkapp token symbol for token {} pk {} index {}",
            token,
            pk,
            index
        );

        Ok(self
            .database
            .get_cf(
                self.zkapp_token_symbol_cf(),
                zkapp_token_symbol_key(token, pk, index),
            )?
            .map(|bytes| String::from_utf8(bytes).expect("zkapp token symbol").into()))
    }

    fn add_zkapp_token_symbol(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        token_symbol: &TokenSymbol,
    ) -> Result<()> {
        trace!(
            "Adding zkapp token symbol for token {} pk {}: {:?}",
            token,
            pk,
            token_symbol
        );

        // get index & update count
        let index = self
            .get_zkapp_token_symbol_num(token, pk)?
            .unwrap_or_default();
        self.database.put_cf(
            self.zkapp_token_symbol_num_cf(),
            zkapp_token_symbol_num_key(token, pk),
            (index + 1).to_be_bytes(),
        )?;

        // write entry
        Ok(self.database.put_cf(
            self.zkapp_token_symbol_cf(),
            zkapp_token_symbol_key(token, pk, index),
            token_symbol.0.as_bytes(),
        )?)
    }

    fn remove_last_zkapp_token_symbol(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
    ) -> Result<TokenSymbol> {
        trace!(
            "Removing last zkapp token symbol for token {} pk {}",
            token,
            pk
        );

        let count = self
            .get_zkapp_token_symbol_num(token, pk)?
            .expect("zkapp token symbol count");
        assert_ne!(count, 0);

        let index = count - 1;
        let token_symbol = self
            .get_zkapp_token_symbol(token, pk, index)?
            .expect("last zkapp token symbol");

        // delete entry
        self.database.delete_cf(
            self.zkapp_token_symbol_cf(),
            zkapp_token_symbol_key(token, pk, index),
        )?;

        // update count
        self.database.put_cf(
            self.zkapp_token_symbol_num_cf(),
            zkapp_token_symbol_num_key(token, pk),
            index.to_be_bytes(),
        )?;

        Ok(token_symbol)
    }

    ////////////
    // timing //
    ////////////

    fn get_zkapp_timing_num(&self, token: &TokenAddress, pk: &PublicKey) -> Result<Option<u32>> {
        trace!("Getting zkapp timing count for token {} pk {}", token, pk);

        Ok(self
            .database
            .get_cf(self.zkapp_timing_num_cf(), zkapp_timing_num_key(token, pk))?
            .map(from_be_bytes))
    }

    fn get_zkapp_timing(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        index: u32,
    ) -> Result<Option<Timing>> {
        trace!(
            "Getting zkapp timing for token {} pk {} index {}",
            token,
            pk,
            index
        );

        Ok(self
            .database
            .get_cf(self.zkapp_timing_cf(), zkapp_timing_key(token, pk, index))?
            .map(|bytes| serde_json::from_slice(&bytes).expect("zkapp timing")))
    }

    fn add_zkapp_timing(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        timing: &Timing,
    ) -> Result<()> {
        trace!(
            "Adding zkapp timing for token {} pk {}: {:?}",
            token,
            pk,
            timing
        );

        // get index & update count
        let index = self.get_zkapp_timing_num(token, pk)?.unwrap_or_default();
        self.database.put_cf(
            self.zkapp_timing_num_cf(),
            zkapp_timing_num_key(token, pk),
            (index + 1).to_be_bytes(),
        )?;

        // write entry
        Ok(self.database.put_cf(
            self.zkapp_timing_cf(),
            zkapp_timing_key(token, pk, index),
            serde_json::to_vec(timing)?,
        )?)
    }

    fn remove_last_zkapp_timing(&self, token: &TokenAddress, pk: &PublicKey) -> Result<Timing> {
        trace!("Removing last zkapp timing for token {} pk {}", token, pk);

        let count = self
            .get_zkapp_timing_num(token, pk)?
            .expect("zkapp timing count");
        assert_ne!(count, 0);

        let index = count - 1;
        let timing = self
            .get_zkapp_timing(token, pk, index)?
            .expect("last zkapp timing");

        // delete entry
        self.database
            .delete_cf(self.zkapp_timing_cf(), zkapp_timing_key(token, pk, index))?;

        // update count
        self.database.put_cf(
            self.zkapp_timing_num_cf(),
            zkapp_timing_num_key(token, pk),
            index.to_be_bytes(),
        )?;

        Ok(timing)
    }
}

#[cfg(test)]
mod tests {
    use super::ZkappStore;
    use crate::{
        base::{public_key::PublicKey, state_hash::StateHash},
        command::TxnHash,
        ledger::{
            account::{Permissions, Timing},
            token::{TokenAddress, TokenSymbol},
        },
        mina_blocks::v2::{
            zkapp::{
                app_state::ZkappState,
                verification_key::{VerificationKey, VerificationKeyData, VerificationKeyHash},
            },
            ZkappUri,
        },
        store::{zkapp::VerificationKeyChange, IndexerStore},
    };
    use quickcheck::{Arbitrary, Gen};
    use speedb::Direction;
    use tempfile::TempDir;

    const GEN_SIZE: usize = 1000;

    fn create_indexer_store() -> anyhow::Result<IndexerStore> {
        let temp_dir = TempDir::with_prefix(std::env::current_dir()?)?;
        IndexerStore::new(temp_dir.path(), true)
    }

    #[test]
    fn zkapp_verification_key_history() -> anyhow::Result<()> {
        let g = &mut Gen::new(GEN_SIZE);
        let store = create_indexer_store()?;

        let pk = PublicKey::arbitrary(g);
        let token = TokenAddress::arbitrary(g);

        let vk = |h: &str| VerificationKey {
            data: VerificationKeyData(format!("data-{h}")),
            hash: VerificationKeyHash(h.to_string()),
        };
        let sh1 = StateHash::new("3NK2tkzqqK5spR2sZ7tujjqPksL45M3UUrcA4WhCkeiPtnugyE2x")?;
        let sh2 = StateHash::new("3NK4BpDSekaqsG6tx8Nse2zJchRft2JpnbvMiog55WCr5xJZaKeP")?;
        let txn = TxnHash::new("5JuJ1eRNWdE8jSMmCDoHnAdBGhLyBnCk2gkcvkfCZ7WvrKtGuWHB")?;

        // no history yet
        assert!(store.get_last_zkapp_verification_key_change(&pk)?.is_none());

        // first key set (creation, old = None) at height 10
        let create = VerificationKeyChange {
            token: token.clone(),
            txn_hash: txn.clone(),
            old_vk_hash: None,
            verification_key: vk("0xAAA"),
        };
        store.add_zkapp_verification_key_change(&pk, 10, &sh1, &create)?;

        // an actual change A -> B at height 20
        let change = VerificationKeyChange {
            token: token.clone(),
            txn_hash: txn.clone(),
            old_vk_hash: Some(VerificationKeyHash("0xAAA".to_string())),
            verification_key: vk("0xBBB"),
        };
        store.add_zkapp_verification_key_change(&pk, 20, &sh2, &change)?;

        // last change is the highest height (20), newest-first
        let (h, sh, last) = store
            .get_last_zkapp_verification_key_change(&pk)?
            .expect("last change");
        assert_eq!(h, 20);
        assert_eq!(sh, sh2);
        assert_eq!(last, change);

        // reverse iterator yields both, newest first
        let heights: Vec<_> = store
            .zkapp_verification_key_history_iterator(&pk, Direction::Reverse)
            .flatten()
            .take_while(|(key, _)| key[..PublicKey::LEN] == *pk.0.as_bytes())
            .map(|(key, _)| u32::from_be_bytes(key[PublicKey::LEN..][..4].try_into().unwrap()))
            .collect();
        assert_eq!(heights, vec![20, 10]);

        // rollback (unapply) removes the height-20 record; last falls back to 10
        store.remove_zkapp_verification_key_change(&pk, 20, &sh2)?;
        let (h, sh, last) = store
            .get_last_zkapp_verification_key_change(&pk)?
            .expect("change after rollback");
        assert_eq!(h, 10);
        assert_eq!(sh, sh1);
        assert_eq!(last, create);

        // remove is idempotent (no record -> no error)
        store.remove_zkapp_verification_key_change(&pk, 20, &sh2)?;

        Ok(())
    }

    #[test]
    fn zkapp_state() -> anyhow::Result<()> {
        let g = &mut Gen::new(GEN_SIZE);
        let store = create_indexer_store()?;

        let pk = PublicKey::arbitrary(g);
        let token = TokenAddress::arbitrary(g);

        assert!(store.get_zkapp_state_num(&token, &pk)?.is_none());

        // add zkapp state
        let zkapp_state0 = ZkappState::arbitrary(g);
        store.add_zkapp_state(&token, &pk, &zkapp_state0)?;

        assert_eq!(store.get_zkapp_state_num(&token, &pk)?.unwrap(), 1);

        // add another zkapp state
        let zkapp_state1 = ZkappState::arbitrary(g);
        store.add_zkapp_state(&token, &pk, &zkapp_state1)?;

        assert_eq!(store.get_zkapp_state_num(&token, &pk)?.unwrap(), 2);

        // get zkapp states
        assert_eq!(
            store.get_zkapp_state(&token, &pk, 0)?.unwrap(),
            zkapp_state0
        );
        assert_eq!(
            store.get_zkapp_state(&token, &pk, 1)?.unwrap(),
            zkapp_state1
        );

        // remove last zkapp states
        store.remove_last_zkapp_state(&token, &pk)?;

        assert_eq!(store.get_zkapp_state_num(&token, &pk)?.unwrap(), 1);
        assert_eq!(
            store.get_zkapp_state(&token, &pk, 0)?.unwrap(),
            zkapp_state0
        );

        store.remove_last_zkapp_state(&token, &pk)?;
        assert_eq!(store.get_zkapp_state_num(&token, &pk)?.unwrap(), 0);

        Ok(())
    }

    #[test]
    fn zkapp_permissions() -> anyhow::Result<()> {
        let g = &mut Gen::new(GEN_SIZE);
        let store = create_indexer_store()?;

        let pk = PublicKey::arbitrary(g);
        let token = TokenAddress::arbitrary(g);

        assert!(store.get_zkapp_permissions_num(&token, &pk)?.is_none());

        // add zkapp permissions
        let permissions0 = Permissions::arbitrary(g);
        store.add_zkapp_permissions(&token, &pk, &permissions0)?;

        assert_eq!(store.get_zkapp_permissions_num(&token, &pk)?.unwrap(), 1);

        // add another zkapp permissions
        let permissions1 = Permissions::arbitrary(g);
        store.add_zkapp_permissions(&token, &pk, &permissions1)?;

        assert_eq!(store.get_zkapp_permissions_num(&token, &pk)?.unwrap(), 2);

        // get zkapp permissions
        assert_eq!(
            store.get_zkapp_permissions(&token, &pk, 0)?.unwrap(),
            permissions0
        );
        assert_eq!(
            store.get_zkapp_permissions(&token, &pk, 1)?.unwrap(),
            permissions1
        );

        // remove last zkapp permissions
        store.remove_last_zkapp_permissions(&token, &pk)?;

        assert_eq!(store.get_zkapp_permissions_num(&token, &pk)?.unwrap(), 1);
        assert_eq!(
            store.get_zkapp_permissions(&token, &pk, 0)?.unwrap(),
            permissions0
        );

        store.remove_last_zkapp_permissions(&token, &pk)?;
        assert_eq!(store.get_zkapp_permissions_num(&token, &pk)?.unwrap(), 0);

        Ok(())
    }

    #[test]
    fn zkapp_verification_key() -> anyhow::Result<()> {
        let g = &mut Gen::new(GEN_SIZE);
        let store = create_indexer_store()?;

        let pk = PublicKey::arbitrary(g);
        let token = TokenAddress::arbitrary(g);

        assert!(store.get_zkapp_verification_key_num(&token, &pk)?.is_none());

        // add zkapp verification key
        let verification_key0 = VerificationKey::arbitrary(g);
        store.add_zkapp_verification_key(&token, &pk, &verification_key0)?;

        assert_eq!(
            store.get_zkapp_verification_key_num(&token, &pk)?.unwrap(),
            1
        );

        // add another zkapp verification key
        let verification_key1 = VerificationKey::arbitrary(g);
        store.add_zkapp_verification_key(&token, &pk, &verification_key1)?;

        assert_eq!(
            store.get_zkapp_verification_key_num(&token, &pk)?.unwrap(),
            2
        );

        // get zkapp verification keys
        assert_eq!(
            store.get_zkapp_verification_key(&token, &pk, 0)?.unwrap(),
            verification_key0
        );
        assert_eq!(
            store.get_zkapp_verification_key(&token, &pk, 1)?.unwrap(),
            verification_key1
        );

        // remove last zkapp verification keys
        store.remove_last_zkapp_verification_key(&token, &pk)?;

        assert_eq!(
            store.get_zkapp_verification_key_num(&token, &pk)?.unwrap(),
            1
        );
        assert_eq!(
            store.get_zkapp_verification_key(&token, &pk, 0)?.unwrap(),
            verification_key0
        );

        store.remove_last_zkapp_verification_key(&token, &pk)?;
        assert_eq!(
            store.get_zkapp_verification_key_num(&token, &pk)?.unwrap(),
            0
        );

        Ok(())
    }

    #[test]
    fn zkapp_uri() -> anyhow::Result<()> {
        let g = &mut Gen::new(GEN_SIZE);
        let store = create_indexer_store()?;

        let pk = PublicKey::arbitrary(g);
        let token = TokenAddress::arbitrary(g);

        assert!(store.get_zkapp_uri_num(&token, &pk)?.is_none());

        // add zkapp uri
        let zkapp_uri0 = ZkappUri::arbitrary(g);
        store.add_zkapp_uri(&token, &pk, &zkapp_uri0)?;

        assert_eq!(store.get_zkapp_uri_num(&token, &pk)?.unwrap(), 1);

        // add another zkapp uri
        let zkapp_uri1 = ZkappUri::arbitrary(g);
        store.add_zkapp_uri(&token, &pk, &zkapp_uri1)?;

        assert_eq!(store.get_zkapp_uri_num(&token, &pk)?.unwrap(), 2);

        // get zkapp uris
        assert_eq!(store.get_zkapp_uri(&token, &pk, 0)?.unwrap(), zkapp_uri0);
        assert_eq!(store.get_zkapp_uri(&token, &pk, 1)?.unwrap(), zkapp_uri1);

        // remove last zkapp uris
        store.remove_last_zkapp_uri(&token, &pk)?;

        assert_eq!(store.get_zkapp_uri_num(&token, &pk)?.unwrap(), 1);
        assert_eq!(store.get_zkapp_uri(&token, &pk, 0)?.unwrap(), zkapp_uri0);

        store.remove_last_zkapp_uri(&token, &pk)?;
        assert_eq!(store.get_zkapp_uri_num(&token, &pk)?.unwrap(), 0);

        Ok(())
    }

    #[test]
    fn zkapp_token_symbol() -> anyhow::Result<()> {
        let g = &mut Gen::new(GEN_SIZE);
        let store = create_indexer_store()?;

        let pk = PublicKey::arbitrary(g);
        let token = TokenAddress::arbitrary(g);

        assert!(store.get_zkapp_token_symbol_num(&token, &pk)?.is_none());

        // add zkapp token symbol
        let token_symbol0 = TokenSymbol::arbitrary(g);
        store.add_zkapp_token_symbol(&token, &pk, &token_symbol0)?;

        assert_eq!(store.get_zkapp_token_symbol_num(&token, &pk)?.unwrap(), 1);

        // add another zkapp token symbol
        let token_symbol1 = TokenSymbol::arbitrary(g);
        store.add_zkapp_token_symbol(&token, &pk, &token_symbol1)?;

        assert_eq!(store.get_zkapp_token_symbol_num(&token, &pk)?.unwrap(), 2);

        // get zkapp token symbols
        assert_eq!(
            store.get_zkapp_token_symbol(&token, &pk, 0)?.unwrap(),
            token_symbol0
        );
        assert_eq!(
            store.get_zkapp_token_symbol(&token, &pk, 1)?.unwrap(),
            token_symbol1
        );

        // remove last zkapp token symbols
        store.remove_last_zkapp_token_symbol(&token, &pk)?;

        assert_eq!(store.get_zkapp_token_symbol_num(&token, &pk)?.unwrap(), 1);
        assert_eq!(
            store.get_zkapp_token_symbol(&token, &pk, 0)?.unwrap(),
            token_symbol0
        );

        store.remove_last_zkapp_token_symbol(&token, &pk)?;
        assert_eq!(store.get_zkapp_token_symbol_num(&token, &pk)?.unwrap(), 0);

        Ok(())
    }

    #[test]
    fn zkapp_timing() -> anyhow::Result<()> {
        let g = &mut Gen::new(GEN_SIZE);
        let store = create_indexer_store()?;

        let pk = PublicKey::arbitrary(g);
        let token = TokenAddress::arbitrary(g);

        assert!(store.get_zkapp_timing_num(&token, &pk)?.is_none());

        // add zkapp timing
        let timing0 = Timing::arbitrary(g);
        store.add_zkapp_timing(&token, &pk, &timing0)?;

        assert_eq!(store.get_zkapp_timing_num(&token, &pk)?.unwrap(), 1);

        // add another zkapp timing
        let timing1 = Timing::arbitrary(g);
        store.add_zkapp_timing(&token, &pk, &timing1)?;

        assert_eq!(store.get_zkapp_timing_num(&token, &pk)?.unwrap(), 2);

        // get zkapp timings
        assert_eq!(store.get_zkapp_timing(&token, &pk, 0)?.unwrap(), timing0);
        assert_eq!(store.get_zkapp_timing(&token, &pk, 1)?.unwrap(), timing1);

        // remove last zkapp timings
        store.remove_last_zkapp_timing(&token, &pk)?;

        assert_eq!(store.get_zkapp_timing_num(&token, &pk)?.unwrap(), 1);
        assert_eq!(store.get_zkapp_timing(&token, &pk, 0)?.unwrap(), timing0);

        store.remove_last_zkapp_timing(&token, &pk)?;
        assert_eq!(store.get_zkapp_timing_num(&token, &pk)?.unwrap(), 0);

        Ok(())
    }
}
