//! Chain store trait

use super::{ChainId, Network};
use crate::base::state_hash::StateHash;
use crate::ledger::LedgerHash;
use serde::{Deserialize, Serialize};

/// The runtime genesis quantities a `--network-config` (custom network) supplies
/// to the store impls in place of the hardcoded `*_GENESIS_*` constants.
///
/// Persisted under [`FixedKeys::CONFIG_GENESIS_KEY`]. Absent for the four
/// hardcoded networks, which keep reading their embedded constants — so when
/// `--network-config` is not used the store behavior is byte-for-byte unchanged.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ConfigGenesis {
    /// Genesis block state hash.
    pub state_hash: StateHash,

    /// Genesis block previous state hash (the synthetic "pre-genesis" hash the
    /// store keys the genesis ledger under).
    pub prev_state_hash: StateHash,

    /// Genesis ledger hash.
    pub ledger_hash: LedgerHash,

    /// Genesis blockchain length.
    pub blockchain_length: u32,
}

pub trait ChainStore {
    /// Persists a (chain id, network) pair
    ///
    /// Error propogates from db
    fn set_chain_id_for_network(&self, chain_id: &ChainId, network: &Network)
        -> anyhow::Result<()>;

    /// Gets the network for the given chain id
    ///
    /// Error if not present
    fn get_network(&self, chain_id: &ChainId) -> anyhow::Result<Network>;

    /// Gets the current network
    ///
    /// Error if not present
    fn get_current_network(&self) -> anyhow::Result<Network>;

    /// Gets the current chain id
    ///
    /// Error if not present
    fn get_chain_id(&self) -> anyhow::Result<ChainId>;

    /// Persists the runtime genesis from a `--network-config` descriptor.
    ///
    /// Only called for custom networks; the hardcoded networks never write this
    /// key and keep using their embedded constants.
    fn set_config_genesis(&self, genesis: &ConfigGenesis) -> anyhow::Result<()>;

    /// Gets the persisted runtime genesis, if a custom network was configured.
    ///
    /// `None` for the hardcoded networks.
    fn get_config_genesis(&self) -> anyhow::Result<Option<ConfigGenesis>>;
}
