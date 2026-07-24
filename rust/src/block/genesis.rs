use super::precomputed::{CurrencyEncoding, PcbVersion};
use crate::{
    base::{blockchain_length::BlockchainLength, state_hash::StateHash},
    block::precomputed::PrecomputedBlock,
    chain::Network,
    constants::{
        DEVNET_GENESIS_BLOCKCHAIN_LENGTH, DEVNET_GENESIS_HASH, HARDFORK_GENESIS_BLOCKCHAIN_LENGTH,
        HARDFORK_GENESIS_HASH, MAINNET_GENESIS_HASH, MESA_GENESIS_BLOCKCHAIN_LENGTH,
        MESA_GENESIS_HASH,
    },
};

#[derive(Debug)]
pub struct GenesisBlock(pub PrecomputedBlock, pub u64);

pub const GENESIS_MAINNET_BLOCK_CONTENTS: &str = include_str!(
    "../../data/genesis_blocks/mainnet-1-3NKeMoncuHab5ScarV5ViyF16cJPT4taWNSaTLS64Dp67wuXigPZ.json"
);

pub const GENESIS_HARDFORK_BLOCK_CONTENTS: &str = include_str!(
    "../../data/genesis_blocks/mainnet-359605-3NK4BpDSekaqsG6tx8Nse2zJchRft2JpnbvMiog55WCr5xJZaKeP.json"
);

// mesa-mut fork/genesis block (transactions emptied so it applies as a no-op
// onto the post-fork genesis ledger). Embedded as bytes: the original block
// contains raw bytes in proof fields that the V2 parser skips.
pub const GENESIS_MESA_BLOCK_CONTENTS: &[u8] = include_bytes!(
    "../../data/genesis_blocks/mesa-297735-3NKQttwm8QRdvSZL62Lid8YAPCXBuAucZPDT8mJriHmw2qk9cVcr.json"
);

// devnet checkpoint/genesis block (transactions emptied so it applies as a
// no-op onto the genesis ledger supplied at runtime).
pub const GENESIS_DEVNET_BLOCK_CONTENTS: &[u8] = include_bytes!(
    "../../data/genesis_blocks/devnet-527922-3NK4DL35iKQ6G8VPqPFLZ122M82dcRRPt8rHrpRW662kXWpH8fRa.json"
);

impl GenesisBlock {
    /// Creates the v1 (pre-hardfork) mainnet genesis block as a PCB
    pub fn new_v1() -> anyhow::Result<Self> {
        let contents = GENESIS_MAINNET_BLOCK_CONTENTS.as_bytes().to_vec();
        let size = contents.len() as u64;
        let network = Network::Mainnet;
        let blockchain_length: BlockchainLength = 1.into();
        let state_hash: StateHash = MAINNET_GENESIS_HASH.into();

        Ok(Self(
            PrecomputedBlock::new(
                network,
                blockchain_length,
                state_hash,
                contents,
                PcbVersion::V1,
            )?,
            size,
        ))
    }

    /// Creates the v2 (hardfork) mainnet genesis block as a PCB
    pub fn new_v2() -> anyhow::Result<Self> {
        let contents = GENESIS_HARDFORK_BLOCK_CONTENTS.as_bytes().to_vec();
        let size = contents.len() as u64;
        let network = Network::Mainnet;
        let blockchain_length: BlockchainLength = HARDFORK_GENESIS_BLOCKCHAIN_LENGTH.into();
        let state_hash: StateHash = HARDFORK_GENESIS_HASH.into();
        let version = PcbVersion::V2(CurrencyEncoding::for_network(&network));

        Ok(Self(
            PrecomputedBlock::new(network, blockchain_length, state_hash, contents, version)?,
            size,
        ))
    }

    /// Creates the mesa-mut fork genesis block as a PCB (transaction version 3)
    pub fn new_mesa() -> anyhow::Result<Self> {
        let contents = GENESIS_MESA_BLOCK_CONTENTS.to_vec();
        let size = contents.len() as u64;
        let network = Network::from("mesa");
        let blockchain_length: BlockchainLength = MESA_GENESIS_BLOCKCHAIN_LENGTH.into();
        let state_hash: StateHash = MESA_GENESIS_HASH.into();
        let version = PcbVersion::V2(CurrencyEncoding::for_network(&network));

        Ok(Self(
            PrecomputedBlock::new(network, blockchain_length, state_hash, contents, version)?,
            size,
        ))
    }

    /// Creates the devnet checkpoint/genesis block as a PCB
    pub fn new_devnet() -> anyhow::Result<Self> {
        let contents = GENESIS_DEVNET_BLOCK_CONTENTS.to_vec();
        let size = contents.len() as u64;
        let network = Network::Devnet;
        let blockchain_length: BlockchainLength = DEVNET_GENESIS_BLOCKCHAIN_LENGTH.into();
        let state_hash: StateHash = DEVNET_GENESIS_HASH.into();
        let version = PcbVersion::V2(CurrencyEncoding::for_network(&network));

        Ok(Self(
            PrecomputedBlock::new(network, blockchain_length, state_hash, contents, version)?,
            size,
        ))
    }
}

impl GenesisBlock {
    pub fn to_precomputed(self) -> PrecomputedBlock {
        self.0
    }

    /// Build a genesis block for a **custom** network (e.g. a minimina
    /// lightnet) from a supplied precomputed-block file named
    /// `<network>-<height>-<hash>.json`.
    ///
    /// Known networks embed their genesis block; a custom network must
    /// *provide* one, because the indexer has no hasher and cannot compute
    /// a genesis block from a ledger. The network / height / state hash are
    /// taken from the filename (the standard block-filename contract), and
    /// the block is parsed with the given `version` (V2 for any
    /// post-Berkeley network).
    pub fn from_file(path: &std::path::Path, version: PcbVersion) -> anyhow::Result<Self> {
        let (network, blockchain_length, state_hash) =
            crate::block::extract_network_height_hash(path);
        let contents = std::fs::read(path)?;
        let size = contents.len() as u64;

        Ok(Self(
            PrecomputedBlock::new(network, blockchain_length, state_hash, contents, version)?,
            size,
        ))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parse_genesis_block_v1() -> anyhow::Result<()> {
        let block = GenesisBlock::new_v1()?;
        assert_eq!(block.0.state_hash().0, MAINNET_GENESIS_HASH);
        Ok(())
    }

    #[test]
    fn parse_genesis_block_v2() -> anyhow::Result<()> {
        let block = GenesisBlock::new_v2()?;
        assert_eq!(block.0.state_hash().0, HARDFORK_GENESIS_HASH);
        Ok(())
    }

    // A custom genesis block is parsed from a supplied file with the standard
    // `<network>-<height>-<hash>.json` name. Using the hardfork genesis contents
    // (a real V2 genesis block) as the "custom" file, `from_file` must reconstruct
    // the same block `new_v2()` does.
    #[test]
    fn parse_genesis_block_from_file() -> anyhow::Result<()> {
        use crate::block::precomputed::CurrencyEncoding;
        let dir = tempfile::TempDir::new()?;
        let name = format!(
            "mainnet-{}-{}.json",
            HARDFORK_GENESIS_BLOCKCHAIN_LENGTH, HARDFORK_GENESIS_HASH
        );
        let path = dir.path().join(name);
        std::fs::write(&path, GENESIS_HARDFORK_BLOCK_CONTENTS)?;

        let from_file = GenesisBlock::from_file(&path, PcbVersion::V2(CurrencyEncoding::Nanomina))?;
        assert_eq!(from_file.0.state_hash().0, HARDFORK_GENESIS_HASH);
        assert_eq!(
            from_file.0.state_hash(),
            GenesisBlock::new_v2()?.0.state_hash()
        );
        Ok(())
    }
}
