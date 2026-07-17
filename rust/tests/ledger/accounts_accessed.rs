//! A block states the post-block state of every account it touched, in
//! `accounts_accessed`. That is the only source for the account fields a ledger
//! diff cannot derive -- `receipt_chain_hash` (a Poseidon chain the indexer has
//! no hasher for), `voting_for` and `permissions`.
//!
//! These fields used to be dropped on the floor: the block -> `Account`
//! conversion ended in `..Default::default()`, so they stayed `None` and the
//! served value was whatever the genesis ledger said -- forever, for every
//! account, silently.

use mina_indexer::{
    block::precomputed::{CurrencyEncoding, PcbVersion, PrecomputedBlock},
    ledger::{diff::LedgerDiff, token::TokenAddress},
};
use std::path::PathBuf;

/// A V2 block with one accessed account that has actually transacted (nonce
/// 175), so its receipt chain hash is *not* the default -- a stale or dropped
/// value shows up.
const BLOCK: &str = "./tests/data/hardfork/mainnet-359606-3NK7T1MeiFA4ALVxqZLuGrWr1PeufYQAm9i1TfMnN9Cu6U5crhot.json";

const PK: &str = "B62qn5dLFmJntm3mR1EUpVqgZaNht2TUEpM9NPYQ4K9gk98hsAvPLnz";
const RECEIPT_CHAIN_HASH: &str = "2n2GQqy9156Bm3cGYV9Uq8wxgQGCMgkqsYXXAmevFNnhpjVtbnEs";
const VOTING_FOR: &str = "3NK2tkzqqK5spR2sZ7tujjqPksL45M3UUrcA4WhCkeiPtnugyE2x";

#[test]
fn ledger_diff_carries_block_stated_account_fields() -> anyhow::Result<()> {
    let block = PrecomputedBlock::parse_file(&PathBuf::from(BLOCK), PcbVersion::V2(CurrencyEncoding::Nanomina))?;
    let diff = LedgerDiff::from_precomputed(&block);

    // the diff must carry the block's own account records
    assert!(
        !diff.accounts_accessed.is_empty(),
        "V2 ledger diff dropped the block's accounts_accessed"
    );

    let stated = diff
        .accounts_accessed
        .iter()
        .map(|accessed| &accessed.account)
        .find(|account| {
            account.public_key.0 == PK
                && account.token.as_ref().unwrap_or(&TokenAddress::default())
                    == &TokenAddress::default()
        })
        .expect("block states this account");

    // the three fields no diff can derive - each was silently dropped before
    assert_eq!(
        stated.receipt_chain_hash.as_ref().map(|r| r.0.as_str()),
        Some(RECEIPT_CHAIN_HASH),
        "receipt_chain_hash must come from the block, not the genesis ledger"
    );
    assert_eq!(
        stated.voting_for.as_ref().map(|v| v.0.as_str()),
        Some(VOTING_FOR),
        "voting_for must come from the block"
    );
    assert!(
        stated.permissions.is_some(),
        "permissions must come from the block"
    );

    Ok(())
}

#[test]
fn applying_a_diff_takes_receipt_chain_hash_from_the_block() -> anyhow::Result<()> {
    let block = PrecomputedBlock::parse_file(&PathBuf::from(BLOCK), PcbVersion::V2(CurrencyEncoding::Nanomina))?;
    let diff = LedgerDiff::from_precomputed(&block);

    // apply onto an empty ledger: the account is created by its diffs, then the
    // block-stated fields are laid over the top
    let ledger = mina_indexer::ledger::Ledger::new().apply_diff(&diff)?;

    let account = ledger
        .get_account(&PK.into(), &TokenAddress::default())
        .expect("account is in the ledger after applying the diff");

    assert_eq!(
        account.receipt_chain_hash.as_ref().map(|r| r.0.as_str()),
        Some(RECEIPT_CHAIN_HASH),
        "applying the diff must set receipt_chain_hash from the block"
    );
    assert_eq!(
        account.voting_for.as_ref().map(|v| v.0.as_str()),
        Some(VOTING_FOR),
        "applying the diff must set voting_for from the block"
    );
    assert!(
        account.permissions.is_some(),
        "applying the diff must set permissions from the block"
    );

    Ok(())
}
