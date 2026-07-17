//! A block's diffs for one account do not arrive in the order the protocol
//! applied them: a zkApp's account updates are a second pass over the ledger,
//! after the block's signed commands, so a zkApp debit can sit ahead of the
//! payment that funds it.
//!
//! Balances are unsigned and saturate at zero, so applying them in that order
//! silently drops the overshoot, and the account is too high by exactly that
//! much from then on.
//!
//! Devnet block 531095 is the real case. The account holds 39.9 MINA and the
//! block moves four amounts through it -- a 5000 MINA payment in, a 1 MINA
//! zkApp fee, and zkApp debits of 1 and 100 -- ending at 4937.9. Applying the
//! 100 first saturates 39.9 to zero, losing 60.1, and the account reads 4998.0
//! forever after. The tip was wrong by exactly that 60.1 MINA.

use mina_indexer::{
    base::{amount::Amount, public_key::PublicKey, state_hash::StateHash},
    ledger::{
        account::Account,
        diff::account::{AccountDiff, PaymentDiff, UpdateType},
        token::TokenAddress,
        Ledger,
    },
};

const PK: &str = "B62qrVDERNv1cj1KiPe82ErRkbcS6Cc3sUobVU6KdfbMeXQFyHPGUq8";
const STATE_HASH: &str = "3NKMZamiGpv1xWk5umLWx9t7NpVd2ccDX1fy4ap9yJ6mfuyTahk9";

const START: u64 = 39_900_000_000; // 39.9 MINA
const PAYMENT: u64 = 5_000_000_000_000; // 5000 MINA in
const BIG_DEBIT: u64 = 100_000_000_000; // 100 MINA out
const SMALL_DEBIT: u64 = 1_000_000_000; // 1 MINA out, twice

/// 39.9 + 5000 - 100 - 1 - 1
const EXPECTED: u64 = 4_937_900_000_000;

fn payment(update_type: UpdateType, amount: u64) -> AccountDiff {
    AccountDiff::Payment(PaymentDiff {
        public_key: PublicKey::from(PK),
        amount: Amount(amount),
        update_type,
        txn_hash: None,
        token: Some(TokenAddress::default()),
    })
}

/// The debits lead, as they do coming out of a block whose zkApp updates are
/// ordered ahead of its payments.
#[test]
fn debit_ahead_of_the_credit_that_funds_it_keeps_the_balance() -> anyhow::Result<()> {
    let token = TokenAddress::default();
    let pk = PublicKey::from(PK);

    let mut ledger = Ledger::new();
    let mut account = Account::empty(pk.clone(), token.clone(), false);
    account.balance = START.into();
    ledger.insert_account(account, &token);

    let diff = mina_indexer::ledger::diff::LedgerDiff {
        state_hash: StateHash::from(STATE_HASH),
        account_diffs: vec![vec![
            payment(UpdateType::Debit(None), BIG_DEBIT),
            payment(UpdateType::Debit(None), SMALL_DEBIT),
            payment(UpdateType::Credit, PAYMENT),
            payment(UpdateType::Debit(None), SMALL_DEBIT),
        ]],
        ..Default::default()
    };

    ledger._apply_diff(&diff)?;

    let balance = ledger.get_account(&pk, &token).expect("account").balance.0;

    assert_eq!(
        balance, EXPECTED,
        "a debit ahead of its funding credit saturated the balance at zero and lost the overshoot",
    );

    Ok(())
}

/// The same amounts in the order the protocol would apply them. This passed
/// before the fix too -- it is here so the pair pins the invariant that the
/// result cannot depend on the order.
#[test]
fn credit_first_is_unchanged() -> anyhow::Result<()> {
    let token = TokenAddress::default();
    let pk = PublicKey::from(PK);

    let mut ledger = Ledger::new();
    let mut account = Account::empty(pk.clone(), token.clone(), false);
    account.balance = START.into();
    ledger.insert_account(account, &token);

    let diff = mina_indexer::ledger::diff::LedgerDiff {
        state_hash: StateHash::from(STATE_HASH),
        account_diffs: vec![vec![
            payment(UpdateType::Credit, PAYMENT),
            payment(UpdateType::Debit(None), BIG_DEBIT),
            payment(UpdateType::Debit(None), SMALL_DEBIT),
            payment(UpdateType::Debit(None), SMALL_DEBIT),
        ]],
        ..Default::default()
    };

    ledger._apply_diff(&diff)?;

    let balance = ledger.get_account(&pk, &token).expect("account").balance.0;
    assert_eq!(balance, EXPECTED);

    Ok(())
}
