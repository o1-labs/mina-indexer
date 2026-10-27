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

        // devnet chain data (also PcbVersion::V2), rooted at its fork genesis,
        // which every devnet block carries in `genesis_state_hash`.
        let devnet_genesis_state_hash: StateHash = DEVNET_GENESIS_HASH.into();

        // The hardfork mainnet node writes currency as nanomina; the newer node
        // devnet runs writes it as decimal MINA. The block does not say
        // which -- both declare protocol_version transaction 3 -- so the chain
        // it belongs to is the only thing that does. See [CurrencyEncoding].
        Self(HashMap::from([
            (v1_genesis_state_hash, (PcbVersion::V1, v1_chain_id)),
            (
                v2_genesis_state_hash,
                (PcbVersion::V2(CurrencyEncoding::Nanomina), v2_chain_id),
            ),
            (
                devnet_genesis_state_hash,
                (
                    PcbVersion::V2(CurrencyEncoding::DecimalMina),
                    ChainId::devnet(),
                ),
            ),
        ]))
    }
}
