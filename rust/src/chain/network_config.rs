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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    const DESCRIPTOR: &str = r#"{
        "network": "testnet",
        "pcb_version": "V2",
        "chain_id": "6d6573612d6d75740000000000000000000000000000000000000000000000aa",
        "genesis": {
            "state_hash": "3NKQttwm8QRdvSZL62Lid8YAPCXBuAucZPDT8mJriHmUZFA8Ybns",
            "prev_state_hash": "3NLp6dKNhYtsqUj49QYV5GtDaeocSJBAa2y2ER2QQLqLukE3wuZT",
            "blockchain_length": 100,
            "global_slot": 200,
            "ledger_hash": "jxicjVogngTDjJh5EEsTUrvBxa3R4fhepqrAeexiRVMogJGqHdT",
            "last_vrf_output": "8oxYNPIKw0xNLJJrhcXRICHIS34t4z-8fsvfTfSbIAA="
        },
        "genesis_ledger": "ledger.json",
        "genesis_block": "blocks/genesis.json"
    }"#;

    fn write_descriptor(dir: &Path, contents: &str) -> anyhow::Result<PathBuf> {
        let path = dir.join("network.json");
        let mut file = std::fs::File::create(&path)?;
        file.write_all(contents.as_bytes())?;
        Ok(path)
    }

    #[test]
    fn parses_all_fields() -> anyhow::Result<()> {
        let dir = TempDir::new()?;
        let path = write_descriptor(dir.path(), DESCRIPTOR)?;
        let config = NetworkConfig::parse_file(&path)?;

        assert_eq!(config.network, "testnet");
        assert_eq!(config.pcb_version, PcbVersion::V2);
        assert_eq!(config.network(), Network::from("testnet"));
        assert_eq!(config.genesis.blockchain_length, 100);
        assert_eq!(config.genesis.global_slot, 200);
        assert_eq!(
            config.genesis.last_vrf_output.as_deref(),
            Some("8oxYNPIKw0xNLJJrhcXRICHIS34t4z-8fsvfTfSbIAA=")
        );
        Ok(())
    }

    #[test]
    fn resolves_relative_paths_against_descriptor_dir() -> anyhow::Result<()> {
        let dir = TempDir::new()?;
        let path = write_descriptor(dir.path(), DESCRIPTOR)?;
        let config = NetworkConfig::parse_file(&path)?;

        assert_eq!(config.genesis_block, dir.path().join("blocks/genesis.json"));
        assert_eq!(
            config.genesis_ledger,
            Some(dir.path().join("ledger.json"))
        );
        Ok(())
    }

    #[test]
    fn leaves_absolute_paths_untouched() -> anyhow::Result<()> {
        let dir = TempDir::new()?;
        let descriptor = DESCRIPTOR
            .replace("\"blocks/genesis.json\"", "\"/abs/genesis.json\"")
            .replace("\"ledger.json\"", "\"/abs/ledger.json\"");
        let path = write_descriptor(dir.path(), &descriptor)?;
        let config = NetworkConfig::parse_file(&path)?;

        assert_eq!(config.genesis_block, PathBuf::from("/abs/genesis.json"));
        assert_eq!(config.genesis_ledger, Some(PathBuf::from("/abs/ledger.json")));
        Ok(())
    }

    #[test]
    fn optional_fields_default_when_omitted() -> anyhow::Result<()> {
        let dir = TempDir::new()?;
        // drop `genesis_ledger` and `last_vrf_output`
        let descriptor = r#"{
            "network": "testnet",
            "pcb_version": "V1",
            "chain_id": "6d6573612d6d75740000000000000000000000000000000000000000000000aa",
            "genesis": {
                "state_hash": "3NKQttwm8QRdvSZL62Lid8YAPCXBuAucZPDT8mJriHmUZFA8Ybns",
                "prev_state_hash": "3NLp6dKNhYtsqUj49QYV5GtDaeocSJBAa2y2ER2QQLqLukE3wuZT",
                "blockchain_length": 1,
                "global_slot": 0,
                "ledger_hash": "jxicjVogngTDjJh5EEsTUrvBxa3R4fhepqrAeexiRVMogJGqHdT"
            },
            "genesis_block": "genesis.json"
        }"#;
        let path = write_descriptor(dir.path(), descriptor)?;
        let config = NetworkConfig::parse_file(&path)?;

        assert_eq!(config.pcb_version, PcbVersion::V1);
        assert!(config.genesis_ledger.is_none());
        assert!(config.genesis.last_vrf_output.is_none());
        Ok(())
    }
}
