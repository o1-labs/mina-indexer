use super::precomputed::PcbVersion;
use crate::{
    base::{blockchain_length::BlockchainLength, state_hash::StateHash},
    block::precomputed::PrecomputedBlock,
    chain::Network,
    constants::{
        HARDFORK_GENESIS_BLOCKCHAIN_LENGTH, HARDFORK_GENESIS_HASH, MAINNET_GENESIS_HASH,
        MESA_GENESIS_BLOCKCHAIN_LENGTH, MESA_GENESIS_HASH,
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
    "../../data/genesis_blocks/mesa-297734-3NLp6dKNhYtsqUj49QYV5GtDaeocSJBAa2y2ER2QQLqLukE3wuZT.json"
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

        Ok(Self(
            PrecomputedBlock::new(
                network,
                blockchain_length,
                state_hash,
                contents,
                PcbVersion::V2,
            )?,
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

        Ok(Self(
            PrecomputedBlock::new(
                network,
                blockchain_length,
                state_hash,
                contents,
                PcbVersion::V2,
            )?,
            size,
        ))
    }
}

impl GenesisBlock {
    pub fn to_precomputed(self) -> PrecomputedBlock {
        self.0
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
}
