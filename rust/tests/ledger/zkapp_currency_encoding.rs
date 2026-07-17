//! The two V2 networks write currency differently, and the block does not say
//! which: both declare `protocol_version` transaction 3. The hardfork mainnet
//! node writes `balance_change.magnitude` as integer nanomina; the newer node
//! devnet and mesa run writes it as decimal MINA. `"150"` is 150 nanomina on
//! one and 150 MINA on the other -- the same text, a billion apart -- so only
//! the network the block came from can settle it.
//!
//! Reading devnet's whole-MINA amounts as nanomina made every one of them a
//! billionth of its real size, which is issue #78: the account below is short
//! its whole 120 MINA, and the summed balance change no longer clears the
//! account creation fee, so the account looks like it was never funded.

use mina_indexer::{
    block::precomputed::{CurrencyEncoding, PcbVersion, PrecomputedBlock},
    ledger::diff::{
        account::PaymentDiff,
        LedgerDiff,
    },
};
use std::path::PathBuf;

/// Every balance change the diff carries, whatever kind of diff it came from.
fn balance_changes(diff: &LedgerDiff) -> Vec<PaymentDiff> {
    diff.account_diffs
        .iter()
        .flatten()
        .cloned()
        .flat_map(PaymentDiff::from_account_diff)
        .collect()
}

/// A devnet block whose zkApp command moves a whole number of MINA: the sender
/// goes 4937.9 -> 4817.8 MINA across it, which is the "120" magnitude plus the
/// "0.1" fee. The protocol's own arithmetic says what "120" means.
const DEVNET_BLOCK: &str =
    "./tests/data/devnet/devnet-531098-3NKLWSTmDsiWtzt79PKVzdro155uYRzC2FsDitqsUV7K4qiY6Ggb.json";

/// A hardfork mainnet block, whose magnitudes are nanomina. `2000000000` here
/// is 2 MINA; read as decimal MINA it would be two billion MINA, twice the
/// entire supply.
const MAINNET_BLOCK: &str =
    "./tests/data/hardfork/mainnet-359606-3NK7T1MeiFA4ALVxqZLuGrWr1PeufYQAm9i1TfMnN9Cu6U5crhot.json";

const PK: &str = "B62qicABScHaLZ4LB4fH8oKqvMXwcjnfSwsdckMS3odQNCJoD2eaz1J";

/// 120 MINA, as the live devnet node and the block's own `accounts_accessed`
/// both report for this account.
const CREDIT_NANOMINA: u64 = 120_000_000_000;

#[test]
fn devnet_whole_mina_balance_change_is_not_read_as_nanomina() -> anyhow::Result<()> {
    let block = PrecomputedBlock::parse_file(
        &PathBuf::from(DEVNET_BLOCK),
        PcbVersion::V2(CurrencyEncoding::DecimalMina),
    )?;
    let diff = LedgerDiff::from_precomputed(&block);

    let credited: Vec<u64> = balance_changes(&diff)
        .iter()
        .filter(|payment| payment.public_key.0 == PK)
        .filter_map(|payment| match payment.balance_change() {
            change if change > 0 => Some(change as u64),
            _ => None,
        })
        .collect();

    assert!(
        credited.contains(&CREDIT_NANOMINA),
        "the block's 120 MINA credit did not survive parsing: {credited:?}",
    );

    // and the block states the resulting balance outright -- the diff must be
    // able to reach it
    let stated = diff
        .accounts_accessed
        .iter()
        .find(|accessed| accessed.account.public_key.0 == PK)
        .map(|accessed| accessed.account.balance.0);

    assert_eq!(
        stated,
        Some(CREDIT_NANOMINA),
        "the block states this account's balance, and it is the credit above",
    );

    Ok(())
}

/// The same whole-number magnitude on a nanomina network must keep meaning
/// nanomina -- this is the reading the indexer has always had, and the one the
/// text alone cannot distinguish from the case above.
#[test]
fn mainnet_magnitudes_stay_nanomina() -> anyhow::Result<()> {
    let path = PathBuf::from(MAINNET_BLOCK);
    if !path.exists() {
        eprintln!("fixture missing, skipping: {}", path.display());
        return Ok(());
    }

    let block = PrecomputedBlock::parse_file(&path, PcbVersion::V2(CurrencyEncoding::Nanomina))?;
    let diff = LedgerDiff::from_precomputed(&block);

    // nothing on a mainnet block may be scaled: every balance change has to stay
    // within the total supply, which decimal-MINA scaling would blow past
    const TOTAL_SUPPLY_NANOMINA: i64 = 1_500_000_000 * 1_000_000_000;

    for payment in balance_changes(&diff).iter() {
        let change = payment.balance_change().abs();

        assert!(
            change < TOTAL_SUPPLY_NANOMINA,
            "mainnet magnitude was scaled as if it were decimal MINA: {change}",
        );
    }

    Ok(())
}
