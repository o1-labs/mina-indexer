//! The staged ledger keeps every account in two column families: one keyed
//! `{state_hash}{token}{pk}`, and one keyed `{state_hash}{token}{balance}{pk}`
//! for balance-sorted iteration. Only the first is a true overwrite -- the
//! balance is *in* the sort key, so re-writing an account at a new balance
//! leaves the old entry behind under its old key.
//!
//! `build_staged_ledger` reads only the sort CF, descending, and each
//! `insert_account` overwrites the last. Two entries for one account therefore
//! resolve to the *lowest* balance ever written, whatever the account's actual
//! final balance is.
//!
//! A block's accounts get re-written at a state hash whenever it is applied
//! more than once -- which is what a reorg does. The two reads then disagree:
//! `get_staged_account` (balance-free key) serves the truth while every ledger
//! reconstructed through `build_staged_ledger` serves a stale balance.

use crate::helpers::store::*;
use mina_indexer::{
    base::{public_key::PublicKey, state_hash::StateHash},
    ledger::{
        account::Account,
        store::staged::StagedLedgerStore,
        token::TokenAddress,
    },
    store::IndexerStore,
};

const PK: &str = "B62qicABScHaLZ4LB4fH8oKqvMXwcjnfSwsdckMS3odQNCJoD2eaz1J";
const STATE_HASH: &str = "3NKKHof1TFyfKBkzuXdoXzzuKP7qQyokWtbCZmUMKEyjd2bghGHP";

const HEIGHT: u32 = 531_098;
const FINAL_BALANCE: u64 = 120_000_000_000;

fn account_with_balance(pk: &PublicKey, token: &TokenAddress, balance: u64) -> Account {
    let mut account = Account::empty(pk.clone(), token.clone(), false);
    account.balance = balance.into();

    account
}

/// Re-writing an account at a *higher* balance must not leave the earlier,
/// lower-balance sort key to win the read back.
#[test]
fn rewritten_staged_account_does_not_resurrect_stale_balance() -> anyhow::Result<()> {
    let store_dir = setup_new_db_dir("staged-ledger-account-rewrite-db")?;
    let store = IndexerStore::new(store_dir.as_ref(), true)?;

    let pk = PublicKey::from(PK);
    let token = TokenAddress::default();
    let state_hash = StateHash::from(STATE_HASH);

    // the account is first written at one balance...
    store.set_staged_account(
        &pk,
        &token,
        &state_hash,
        HEIGHT,
        &account_with_balance(&pk, &token, 0),
    )?;

    // ...then re-written at its real balance, as a re-apply of the same block does
    store.set_staged_account(
        &pk,
        &token,
        &state_hash,
        HEIGHT,
        &account_with_balance(&pk, &token, FINAL_BALANCE),
    )?;

    // the balance-free read has always been right
    let staged_account = store
        .get_staged_account(&pk, &token, &state_hash)?
        .expect("staged account");

    assert_eq!(
        staged_account.balance.0, FINAL_BALANCE,
        "staged account read lost the account's final balance",
    );

    // the reconstructed ledger must agree with it
    let ledger = store.build_staged_ledger(&state_hash)?.expect("ledger");
    let ledger_account = ledger.get_account(&pk, &token).expect("ledger account");

    assert_eq!(
        ledger_account.balance.0, FINAL_BALANCE,
        "build_staged_ledger served a stale balance from an orphaned sort key",
    );

    Ok(())
}

/// A removed staged account must not be left behind in the sort CF, whatever
/// balance the caller computes for it.
#[test]
fn removed_staged_account_leaves_no_orphan_sort_key() -> anyhow::Result<()> {
    let store_dir = setup_new_db_dir("staged-ledger-account-remove-db")?;
    let store = IndexerStore::new(store_dir.as_ref(), true)?;

    let pk = PublicKey::from(PK);
    let token = TokenAddress::default();
    let state_hash = StateHash::from(STATE_HASH);

    store.set_staged_account(
        &pk,
        &token,
        &state_hash,
        HEIGHT,
        &account_with_balance(&pk, &token, FINAL_BALANCE),
    )?;

    // an unapply rolls the account back before removing it, so the balance it
    // has on hand is not the one the account was stored under
    store.remove_staged_account(&pk, &token, &state_hash, HEIGHT)?;

    let ledger = store.build_staged_ledger(&state_hash)?.expect("ledger");

    assert!(
        ledger.get_account(&pk, &token).is_none(),
        "removed staged account survived in the balance-sorted ledger",
    );

    Ok(())
}
