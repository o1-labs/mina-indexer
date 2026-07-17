//! Chain data

pub mod store;

mod id;
mod network;

use crate::{
    base::state_hash::StateHash,
    block::precomputed::{CurrencyEncoding, PcbVersion},
    constants::*,
};
use std::collections::HashMap;

// re-export types
pub type ChainId = id::ChainId;
pub type Network = network::Network;

#[derive(Debug)]
pub struct ChainData(pub HashMap<StateHash, (PcbVersion, ChainId)>);

/////////////
// default //
/////////////

impl std::default::Default for ChainData {
    fn default() -> Self {
        // v1 chain data
        let v1_genesis_state_hash: StateHash = MAINNET_GENESIS_HASH.into();
        let v1_chain_id = ChainId::v1();

        // v2 chain data
        let v2_genesis_state_hash: StateHash = HARDFORK_GENESIS_HASH.into();
        let v2_chain_id = ChainId::v2();

        // mesa-mut chain data (also PcbVersion::V2). Both the fork genesis hash
        // and the pre-fork original genesis hash (which mesa blocks carry in
        // their `genesis_state_hash` field) map to the mesa V2 chain.
        let mesa_genesis_state_hash: StateHash = MESA_GENESIS_HASH.into();
        let mesa_original_genesis_state_hash: StateHash = MESA_ORIGINAL_GENESIS_HASH.into();

        // devnet chain data (also PcbVersion::V2). Both the checkpoint-root genesis
        // hash and the devnet chain's genesis hash (which devnet blocks carry in
        // `genesis_state_hash`) map to the devnet V2 chain.
        let devnet_genesis_state_hash: StateHash = DEVNET_GENESIS_HASH.into();
        let devnet_original_genesis_state_hash: StateHash = DEVNET_ORIGINAL_GENESIS_HASH.into();

        // The hardfork mainnet node writes currency as nanomina; the newer node
        // devnet and mesa run writes it as decimal MINA. The block does not say
        // which -- both declare protocol_version transaction 3 -- so the chain
        // it belongs to is the only thing that does. See [CurrencyEncoding].
        Self(HashMap::from([
            (v1_genesis_state_hash, (PcbVersion::V1, v1_chain_id)),
            (
                v2_genesis_state_hash,
                (PcbVersion::V2(CurrencyEncoding::Nanomina), v2_chain_id),
            ),
            (
                mesa_genesis_state_hash,
                (PcbVersion::V2(CurrencyEncoding::DecimalMina), ChainId::mesa()),
            ),
            (
                mesa_original_genesis_state_hash,
                (PcbVersion::V2(CurrencyEncoding::DecimalMina), ChainId::mesa()),
            ),
            (
                devnet_genesis_state_hash,
                (
                    PcbVersion::V2(CurrencyEncoding::DecimalMina),
                    ChainId::devnet(),
                ),
            ),
            (
                devnet_original_genesis_state_hash,
                (
                    PcbVersion::V2(CurrencyEncoding::DecimalMina),
                    ChainId::devnet(),
                ),
            ),
        ]))
    }
}
