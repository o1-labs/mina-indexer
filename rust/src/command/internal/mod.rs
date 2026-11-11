pub mod store;

use crate::{
    base::{public_key::PublicKey, state_hash::StateHash},
    block::precomputed::PrecomputedBlock,
    command::{UserCommandWithStatus, UserCommandWithStatusT},
    ledger::coinbase::{Coinbase, CoinbaseFeeTransfer, CoinbaseKind},
    snark_work::SnarkWorkSummary,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum InternalCommandKind {
    Coinbase,

    #[serde(rename = "Fee_transfer")]
    FeeTransfer,

    #[serde(rename = "Fee_transfer_via_coinbase")]
    FeeTransferViaCoinbase,
}

/// This type is our internal representation of internal commands.
/// We use these internal commands for ledger calculations.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum InternalCommand {
    Coinbase {
        receiver: PublicKey,
        amount: u64,
    },
    FeeTransfer {
        sender: PublicKey,
        receiver: PublicKey,
        amount: u64,
    },
    FeeTransferViaCoinbase {
        sender: PublicKey,
        receiver: PublicKey,
        amount: u64,
    },
}

impl InternalCommand {
    /// Compute the internal commands for the given precomputed block, as the
    /// protocol does (`Staged_ledger.Pre_diff_info.get_transaction_data`):
    /// the coinbase first, then for each staged ledger pre-diff its fee
    /// transfers via coinbase and its fee transfers.
    ///
    /// The fee transfers are net amounts, not the gross fee flows the ledger
    /// diffs use: the coinbase receiver gets the user command fees minus the
    /// SNARK work fees it pays from them, and each prover gets its work fees
    /// minus the part paid via coinbase. Both give the same balances.
    pub fn from_precomputed(block: &PrecomputedBlock) -> Vec<Self> {
        let mut internal_cmds = vec![];

        let coinbase = Coinbase::from_precomputed(block);
        if coinbase.is_applied() {
            internal_cmds.push(coinbase.as_internal_cmd());
        }

        let receiver = block.coinbase_receiver();
        internal_cmds.append(&mut Self::from_pre_diff(
            &receiver,
            &block.pre_diff_coinbase(),
            &block.commands_pre_diff(),
            &block.completed_works_pre_diff(),
        ));

        if let Some(post_diff_coinbase) = block.post_diff_coinbase() {
            internal_cmds.append(&mut Self::from_pre_diff(
                &receiver,
                &post_diff_coinbase,
                &block.commands_post_diff(),
                &block.completed_works_post_diff().unwrap_or_default(),
            ));
        }

        internal_cmds
    }

    /// Fee transfers via coinbase and fee transfers of one pre-diff
    fn from_pre_diff(
        receiver: &PublicKey,
        coinbase: &CoinbaseKind,
        commands: &[UserCommandWithStatus],
        completed_works: &[SnarkWorkSummary],
    ) -> Vec<Self> {
        // a coinbase fee transfer to the coinbase receiver is dropped
        // (`Coinbase.create`)
        let coinbase_fts: Vec<&CoinbaseFeeTransfer> = match coinbase {
            CoinbaseKind::Zero => vec![],
            CoinbaseKind::One(ft) => ft.iter().collect(),
            CoinbaseKind::Two(ft1, ft2) => ft1.iter().chain(ft2.iter()).collect(),
        }
        .into_iter()
        .filter(|ft| &ft.receiver_pk != receiver)
        .collect();
        let coinbase_work_fees: u64 = coinbase_fts.iter().map(|ft| ft.fee).sum();

        // the receiver's own work is not paid
        let works_others: Vec<_> = completed_works
            .iter()
            .filter(|work| &work.prover != receiver)
            .collect();
        let work_fees: u64 = works_others.iter().map(|work| work.fee.0).sum();
        let budget: u64 = commands.iter().map(|cmd| cmd.fee()).sum();
        let remainder = budget.saturating_sub(work_fees.saturating_sub(coinbase_work_fees));

        let mut fees = BTreeMap::<PublicKey, u64>::new();
        if remainder > 0 {
            fees.insert(receiver.clone(), remainder);
        }
        for work in works_others.into_iter().filter(|work| work.fee.0 > 0) {
            *fees.entry(work.prover.clone()).or_default() += work.fee.0;
        }

        // the work paid via coinbase is already part of the coinbase
        for ft in &coinbase_fts {
            if let Some(fee) = fees.get_mut(&ft.receiver_pk) {
                *fee = fee.saturating_sub(ft.fee);
                if *fee == 0 {
                    fees.remove(&ft.receiver_pk);
                }
            }
        }

        let via_coinbase = coinbase_fts
            .into_iter()
            .map(|ft| Self::FeeTransferViaCoinbase {
                sender: receiver.clone(),
                receiver: ft.receiver_pk.clone(),
                amount: ft.fee,
            });
        let fee_transfers = fees.into_iter().map(|(pk, amount)| Self::FeeTransfer {
            sender: receiver.clone(),
            receiver: pk,
            amount,
        });

        via_coinbase.chain(fee_transfers).collect()
    }
}

/// This type is used to store internal commands in the store.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(untagged)]
pub enum DbInternalCommand {
    Coinbase { amount: u64, receiver: PublicKey },
    FeeTransfer { amount: u64, receiver: PublicKey },
    FeeTransferViaCoinbase { amount: u64, receiver: PublicKey },
}

/// This type is used to get internal commands with metadata from the store.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum DbInternalCommandWithData {
    // Fee_transfer or Fee_transfer_via_coinbase
    FeeTransfer {
        receiver: PublicKey,
        amount: u64,
        state_hash: StateHash,
        kind: InternalCommandKind,
        date_time: i64,
        block_height: u32,
    },
    Coinbase {
        receiver: PublicKey,
        amount: u64,
        state_hash: StateHash,
        kind: InternalCommandKind,
        date_time: i64,
        block_height: u32,
    },
}

impl DbInternalCommandWithData {
    pub fn from_internal_cmd(
        cmd: DbInternalCommand,
        state_hash: StateHash,
        block_height: u32,
        date_time: i64,
    ) -> Self {
        match cmd {
            DbInternalCommand::Coinbase { receiver, amount } => Self::Coinbase {
                amount,
                receiver,
                date_time,
                state_hash,
                block_height,
                kind: InternalCommandKind::Coinbase,
            },
            DbInternalCommand::FeeTransfer { receiver, amount } => Self::FeeTransfer {
                amount,
                receiver,
                date_time,
                state_hash,
                block_height,
                kind: InternalCommandKind::FeeTransfer,
            },
            DbInternalCommand::FeeTransferViaCoinbase { receiver, amount } => Self::FeeTransfer {
                amount,
                receiver,
                date_time,
                state_hash,
                block_height,
                kind: InternalCommandKind::FeeTransferViaCoinbase,
            },
        }
    }

    pub fn from_precomputed(block: &PrecomputedBlock) -> Vec<Self> {
        DbInternalCommand::from_precomputed(block)
            .into_iter()
            .map(|cmd| {
                Self::from_internal_cmd(
                    cmd,
                    block.state_hash(),
                    block.blockchain_length(),
                    block.timestamp() as i64,
                )
            })
            .collect()
    }

    pub fn public_keys(&self) -> PublicKey {
        match self {
            Self::Coinbase { receiver, .. } | Self::FeeTransfer { receiver, .. } => {
                receiver.clone()
            }
        }
    }

    pub fn contains_pk(&self, pk: &PublicKey) -> bool {
        match self {
            Self::Coinbase { receiver, .. } | Self::FeeTransfer { receiver, .. } => pk == receiver,
        }
    }

    pub fn recipient(&self) -> PublicKey {
        match self {
            DbInternalCommandWithData::Coinbase { receiver, .. }
            | DbInternalCommandWithData::FeeTransfer { receiver, .. } => receiver.clone(),
        }
    }

    pub fn kind(&self) -> u8 {
        match self {
            DbInternalCommandWithData::FeeTransfer { .. } => 0,
            DbInternalCommandWithData::Coinbase { .. } => 1,
        }
    }
}

impl DbInternalCommand {
    pub fn from_precomputed(block: &PrecomputedBlock) -> Vec<Self> {
        InternalCommand::from_precomputed(block)
            .into_iter()
            .map(Into::into)
            .collect()
    }
}

/////////////////
// conversions //
/////////////////

impl From<InternalCommand> for DbInternalCommand {
    fn from(value: InternalCommand) -> Self {
        match value {
            InternalCommand::Coinbase { receiver, amount } => Self::Coinbase { receiver, amount },
            InternalCommand::FeeTransfer {
                receiver,
                amount,
                sender: _,
            } => Self::FeeTransfer { receiver, amount },
            InternalCommand::FeeTransferViaCoinbase {
                receiver,
                amount,
                sender: _,
            } => Self::FeeTransferViaCoinbase { receiver, amount },
        }
    }
}

/////////////
// dispaly //
/////////////

impl std::fmt::Display for InternalCommandKind {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            InternalCommandKind::Coinbase => write!(f, "Coinbase"),
            InternalCommandKind::FeeTransfer => write!(f, "Fee_transfer"),
            InternalCommandKind::FeeTransferViaCoinbase => write!(f, "Fee_transfer_via_coinbase"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::precomputed::{CurrencyEncoding, PcbVersion, PrecomputedBlock};

    #[test]
    fn from_precomputed_v1() -> anyhow::Result<()> {
        let path = std::path::PathBuf::from("./tests/data/canonical_chain_discovery/contiguous/mainnet-11-3NLMeYAFXxsmhSFtLHFxdtjGcfHTVFmBmBF8uTJvP4Ve5yEmxYeA.json");
        let block = PrecomputedBlock::parse_file(&path, PcbVersion::V1)?;

        assert_eq!(
            InternalCommand::from_precomputed(&block),
            vec![
                InternalCommand::Coinbase {
                    receiver: "B62qs2YyNuo1LbNo5sbhPByDDAB7NZiejFM6H1ctND5ui7wH4PWa7qm".into(),
                    amount: 720000000000
                },
                InternalCommand::FeeTransfer {
                    sender: "B62qs2YyNuo1LbNo5sbhPByDDAB7NZiejFM6H1ctND5ui7wH4PWa7qm".into(),
                    receiver: "B62qs2YyNuo1LbNo5sbhPByDDAB7NZiejFM6H1ctND5ui7wH4PWa7qm".into(),
                    amount: 20000000
                }
            ]
        );

        let internal_cmds = DbInternalCommand::from_precomputed(&block);
        assert_eq!(
            internal_cmds,
            vec![
                DbInternalCommand::Coinbase {
                    receiver: "B62qs2YyNuo1LbNo5sbhPByDDAB7NZiejFM6H1ctND5ui7wH4PWa7qm".into(),
                    amount: 720000000000
                },
                DbInternalCommand::FeeTransfer {
                    receiver: "B62qs2YyNuo1LbNo5sbhPByDDAB7NZiejFM6H1ctND5ui7wH4PWa7qm".into(),
                    amount: 20000000
                }
            ]
        );

        let cmds: Vec<DbInternalCommandWithData> = internal_cmds
            .into_iter()
            .map(|cmd| {
                DbInternalCommandWithData::from_internal_cmd(
                    cmd,
                    block.state_hash(),
                    block.blockchain_length(),
                    block.timestamp() as i64,
                )
            })
            .collect();
        assert_eq!(
            cmds,
            vec![
                DbInternalCommandWithData::Coinbase {
                    receiver: "B62qs2YyNuo1LbNo5sbhPByDDAB7NZiejFM6H1ctND5ui7wH4PWa7qm".into(),
                    amount: 720000000000,
                    state_hash: "3NLMeYAFXxsmhSFtLHFxdtjGcfHTVFmBmBF8uTJvP4Ve5yEmxYeA".into(),
                    kind: InternalCommandKind::Coinbase,
                    block_height: block.blockchain_length(),
                    date_time: block.timestamp() as i64,
                },
                DbInternalCommandWithData::FeeTransfer {
                    receiver: "B62qs2YyNuo1LbNo5sbhPByDDAB7NZiejFM6H1ctND5ui7wH4PWa7qm".into(),
                    amount: 20000000,
                    state_hash: "3NLMeYAFXxsmhSFtLHFxdtjGcfHTVFmBmBF8uTJvP4Ve5yEmxYeA".into(),
                    kind: InternalCommandKind::FeeTransfer,
                    block_height: block.blockchain_length(),
                    date_time: block.timestamp() as i64,
                }
            ]
        );

        Ok(())
    }

    /// Block with SNARK work: 3 works by one prover at 0.004 MINA, one paid
    /// via coinbase, and 0.01 MINA of user command fees. The values are the
    /// archive node's internal commands for this block.
    #[test]
    fn net_fee_transfers_with_snark_work() -> anyhow::Result<()> {
        let path = std::path::PathBuf::from("./tests/data/devnet/devnet-587491-3NLeYAKahDjfbtnJetWiEnQXfGSukVt87ER8tbkXPJJwGkYDnieG.json");
        let block = PrecomputedBlock::parse_file(&path, PcbVersion::V2(CurrencyEncoding::DecimalMina))?;

        let producer: PublicKey = "B62qnGoNQ1gJ4h97wMZeKr6Nk3PY6qQPH5kAGnkuuqbRqKPpDkG6pYu".into();
        let prover: PublicKey = "B62qoXucNXRjjG1SmWvogDQTeC17GRNsdhvhsBUEcsMy6CAXhWSgmTV".into();
        assert_eq!(block.coinbase_receiver(), producer);
        assert_eq!(block.tx_fees(), 10_000_000);
        assert_eq!(block.snark_fees(), 12_000_000);

        let mut fee_transfers = DbInternalCommand::from_precomputed(&block);
        assert!(matches!(
            fee_transfers.remove(0),
            DbInternalCommand::Coinbase { receiver, .. } if receiver == producer
        ));
        fee_transfers.sort();
        assert_eq!(
            fee_transfers,
            vec![
                DbInternalCommand::FeeTransfer {
                    receiver: producer,
                    amount: 2_000_000
                },
                DbInternalCommand::FeeTransfer {
                    receiver: prover.clone(),
                    amount: 8_000_000
                },
                DbInternalCommand::FeeTransferViaCoinbase {
                    receiver: prover,
                    amount: 4_000_000
                },
            ]
        );

        Ok(())
    }

    #[test]
    fn from_genesis_v2() -> anyhow::Result<()> {
        let path = std::path::PathBuf::from("./data/genesis_blocks/mainnet-359605-3NK4BpDSekaqsG6tx8Nse2zJchRft2JpnbvMiog55WCr5xJZaKeP.json");
        let block = PrecomputedBlock::parse_file(&path, PcbVersion::V2(CurrencyEncoding::Nanomina))?;

        // empty internal commands
        assert_eq!(InternalCommand::from_precomputed(&block), vec![]);

        // empty db internal commands
        assert_eq!(DbInternalCommand::from_precomputed(&block), vec![]);

        // empty db internal commands with metadata
        assert_eq!(DbInternalCommandWithData::from_precomputed(&block), vec![]);

        Ok(())
    }
}
