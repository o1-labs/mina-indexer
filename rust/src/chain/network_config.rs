//! Custom-network descriptor (the input to `--network-config`).
//!
//! When the indexer is pointed at an arbitrary Mina network (e.g. a lightnet
//! whose genesis is regenerated each boot), the hardcoded genesis-hash dispatch
//! in `bin/mina-indexer.rs` cannot resolve it. This descriptor supplies the same
//! `(network, pcb_version, chain_id, genesis)` quantities at runtime.
//!
//! The rule everywhere is: known genesis hash -> embedded constant; otherwise ->
//! this descriptor. When `--network-config` is absent, none of this is used and
//! behavior is byte-for-byte unchanged.

use crate::{block::precomputed::PcbVersion, chain::Network};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Top-level descriptor parsed from the `--network-config <file.json>` file.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NetworkConfig {
    /// Network name (e.g. "testnet", "mainnet"). Parsed with the same
    /// `Network::from(&str)` mapping used by the `--network` CLI flag, so e.g.
    /// "testnet" becomes `Network::Custom("testnet")`.
    pub network: String,

    /// Precomputed-block format version ("V1" | "V2").
    pub pcb_version: PcbVersion,

    /// 64-hex chain id.
    pub chain_id: String,

    /// Runtime genesis quantities.
    pub genesis: NetworkConfigGenesis,

    /// Path to the genesis ledger (JSON). Optional: when omitted, the existing
    /// `--genesis-ledger` flag supplies the ledger.
    #[serde(default)]
    pub genesis_ledger: Option<PathBuf>,

    /// Path to a precomputed-block JSON file used as the genesis block.
    pub genesis_block: PathBuf,
}

/// Runtime genesis quantities (mirror of the per-network `*_GENESIS_*`
/// constants) — read by the store impls instead of those constants when a
/// custom network is configured.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NetworkConfigGenesis {
    pub state_hash: String,
    pub prev_state_hash: String,
    pub blockchain_length: u32,
    pub global_slot: u32,
    pub ledger_hash: String,

    /// Optional last vrf output (base64). Defaults when omitted.
    #[serde(default)]
    pub last_vrf_output: Option<String>,
}

impl NetworkConfig {
    /// The network as the indexer's `Network` type.
    pub fn network(&self) -> Network {
        Network::from(self.network.as_str())
    }

    /// Parses a descriptor file. Relative `genesis_ledger`/`genesis_block` paths
    /// are resolved against the descriptor file's parent directory.
    pub fn parse_file(path: &Path) -> anyhow::Result<Self> {
        let contents = std::fs::read(path)?;
        let mut config: Self = serde_json::from_slice(&contents)?;

        let base = path.parent();
        config.genesis_block = resolve(base, &config.genesis_block);
        config.genesis_ledger = config.genesis_ledger.map(|p| resolve(base, &p));

        Ok(config)
    }
}

fn resolve(base: Option<&Path>, p: &Path) -> PathBuf {
    match base {
        Some(base) if p.is_relative() => base.join(p),
        _ => p.to_path_buf(),
    }
}
