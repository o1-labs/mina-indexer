//! GraphQL `accounts` endpoint

mod zkapp;

use super::{
    db,
    pk::{DelegatePK, PK},
};
use crate::{
    base::public_key::PublicKey,
    block::store::BlockStore,
    command::{internal::store::InternalCommandStore, store::UserCommandStore},
    constants::MINA_TOKEN_ADDRESS,
    ledger::{
        account::{self, Permission},
        store::best::BestLedgerStore,
        token::TokenAddress,
    },
    snark_work::store::SnarkStore,
    store::{username::UsernameStore, IndexerStore},
    utility::store::common::U64_LEN,
    web::graphql::timing::Timing,
};
use async_graphql::{Context, Enum, InputObject, Object, Result, SimpleObject};
use speedb::IteratorMode;
use zkapp::ZkappAccount;

#[derive(InputObject)]
pub struct AccountQueryInput {
    public_key: Option<String>,
    delegate: Option<String>,
    username: Option<String>,
    balance: Option<u64>,
    token: Option<String>,
    zkapp: Option<bool>,

    #[graphql(name = "balance_gt")]
    balance_gt: Option<u64>,

    #[graphql(name = "balance_gte")]
    balance_gte: Option<u64>,

    #[graphql(name = "balance_lt")]
    balance_lt: Option<u64>,

    #[graphql(name = "balance_lte")]
    balance_lte: Option<u64>,

    #[graphql(name = "balance_ne")]
    balance_ne: Option<u64>,
}

#[derive(SimpleObject)]
pub struct Account {
    /// Value public key
    #[graphql(flatten)]
    public_key: PK,

    /// Value delegate public key
    #[graphql(flatten)]
    delegate: DelegatePK,

    /// Value balance (nano)
    balance: u64,

    /// Value nonce
    nonce: u32,

    /// Value time locked
    time_locked: bool,

    /// Value account timing
    timing: Option<Timing>,

    /// Value account token address
    token: String,

    /// Value zkapp
    zkapp: Option<ZkappAccount>,

    /// Value receipt chain hash
    receipt_chain_hash: String,

    /// Value voting for
    voting_for: String,

    /// Value permissions
    permissions: Option<Permissions>,
}

#[derive(SimpleObject, Default, Debug, Clone, PartialEq, Eq)]
struct Permissions {
    #[graphql(name = "edit_state")]
    edit_state: String,

    #[graphql(name = "access")]
    access: String,

    #[graphql(name = "send")]
    send: String,

    #[graphql(name = "receive")]
    receive: String,

    #[graphql(name = "set_delegate")]
    set_delegate: String,

    #[graphql(name = "set_permissions")]
    set_permissions: String,

    #[graphql(name = "set_verification_key")]
    set_verification_key: PermissionVk,

    #[graphql(name = "set_zkapp_uri")]
    set_zkapp_uri: String,

    #[graphql(name = "edit_action_state")]
    edit_action_state: String,

    #[graphql(name = "set_token_symbol")]
    set_token_symbol: String,

    #[graphql(name = "increment_nonce")]
    increment_nonce: String,

    #[graphql(name = "set_voting_for")]
    set_voting_for: String,

    #[graphql(name = "set_timing")]
    set_timing: String,
}

#[derive(SimpleObject, Default, Debug, Clone, PartialEq, Eq)]
struct PermissionVk {
    permission: String,
    number: String,
}

#[derive(Enum, Copy, Clone, Default, Eq, PartialEq)]
pub enum AccountSortByInput {
    BalanceAsc,

    #[default]
    BalanceDesc,
}

#[derive(SimpleObject)]
pub struct AccountWithMeta {
    #[graphql(flatten)]
    pub account: Account,

    #[graphql(name = "is_genesis_account")]
    is_genesis_account: bool,

    #[graphql(name = "genesis_account")]
    genesis_account: Option<u64>,

    #[graphql(name = "pk_epoch_num_blocks")]
    pk_epoch_num_blocks: u32,

    #[graphql(name = "pk_total_num_blocks")]
    pk_total_num_blocks: u32,

    #[graphql(name = "pk_epoch_num_snarks")]
    pk_epoch_num_snarks: u32,

    #[graphql(name = "pk_total_num_snarks")]
    pk_total_num_snarks: u32,

    #[graphql(name = "pk_epoch_num_user_commands")]
    pk_epoch_num_user_commands: u32,

    #[graphql(name = "pk_total_num_user_commands")]
    pk_total_num_user_commands: u32,

    #[graphql(name = "pk_epoch_num_zkapp_commands")]
    pk_epoch_num_zkapp_commands: u32,

    #[graphql(name = "pk_total_num_zkapp_commands")]
    pk_total_num_zkapp_commands: u32,

    #[graphql(name = "pk_epoch_num_internal_commands")]
    pk_epoch_num_internal_commands: u32,

    #[graphql(name = "pk_total_num_internal_commands")]
    pk_total_num_internal_commands: u32,

    #[graphql(name = "block_height")]
    block_height: u32,

    // TODO deprecate
    username: String,
}

/// Deserialize one stored ledger account and test it against `query`, returning
/// the account when it matches (`None` otherwise). Single source of truth for the
/// account filter, shared by the `accounts` list resolver and the `accountsCount`
/// count resolver so a page and its total can never disagree. `query == None`
/// matches every account. Errors on a corrupt stored record (propagated, as the
/// list path did before).
fn account_matches(
    query: Option<&AccountQueryInput>,
    db: &std::sync::Arc<IndexerStore>,
    value: &[u8],
) -> Result<Option<account::Account>> {
    let account =
        serde_json::from_slice::<account::Account>(value)?.deduct_mina_account_creation_fee();
    let username = db.get_username(&account.public_key).ok().flatten().map(|u| u.0);
    let matches = query.is_none_or(|q| q.matches(&account, username.as_ref()));
    Ok(matches.then_some(account))
}

#[derive(Default)]
pub struct AccountQueryRoot;

#[Object]
impl AccountQueryRoot {
    #[graphql(cache_control(max_age = 3600))]
    async fn accounts(
        &self,
        ctx: &Context<'_>,
        query: Option<AccountQueryInput>,
        sort_by: Option<AccountSortByInput>,
        #[graphql(default = 100)] limit: usize,
        // `offset`: matching accounts to skip before `limit` -- pages the result
        // set. Pair with `accountsCount(query)` for total-count / page math.
        #[graphql(default = 0)] offset: usize,
    ) -> Result<Vec<AccountWithMeta>> {
        use AccountSortByInput::*;

        let limit = limit.min(crate::constants::GRAPHQL_MAX_PAGE_SIZE);
        let db = db(ctx);
        let sort_by = sort_by.unwrap_or_default();

        // query or default token
        let token = query
            .as_ref()
            .map_or(TokenAddress::default(), |q| match q.token.as_ref() {
                Some(token) => TokenAddress::new(token).expect("valid token address"),
                None => TokenAddress::default(),
            });

        // public key query handler
        if let Some(public_key) = query.as_ref().and_then(|q| q.public_key.as_ref()) {
            if let Ok(pk) = PublicKey::new(public_key) {
                return Ok(db
                    .get_best_account_display(&pk, &token)?
                    .into_iter()
                    .filter_map(|acct| {
                        let username = match db.get_username(&pk) {
                            Ok(None) | Err(_) => None,
                            Ok(Some(username)) => Some(username.0),
                        };

                        if query.as_ref().unwrap().matches(&acct, username.as_ref()) {
                            let account = AccountWithMeta::new(db, acct);
                            return Some(account);
                        }

                        None
                    })
                    .collect());
            } else {
                return Err(async_graphql::Error::new(format!(
                    "Invalid public key: {}",
                    public_key
                )));
            }
        }

        // token query handler
        if let Some(token) = query.as_ref().and_then(|q| q.token.as_ref()) {
            return query
                .as_ref()
                .unwrap()
                .token_query_handler(db, token as &str, sort_by, limit, offset);
        }

        let mode = match sort_by {
            BalanceAsc => IteratorMode::Start,
            BalanceDesc => IteratorMode::End,
        };

        // default query handler use balance-sorted accounts
        let iter = match query.as_ref().and_then(|q| q.zkapp) {
            None | Some(false) => db.best_ledger_account_balance_iterator(mode).flatten(),
            Some(true) => db
                .zkapp_best_ledger_account_balance_iterator(mode)
                .flatten(),
        };
        let mut accounts = Vec::with_capacity(limit);
        let mut skipped = 0;

        for (_, value) in iter {
            if accounts.len() >= limit {
                break;
            }

            // Same filter as `accountsCount` (shared `account_matches`), so the
            // page and the total can't drift.
            if let Some(account) = account_matches(query.as_ref(), db, &value)? {
                if skipped < offset {
                    skipped += 1;
                    continue;
                }
                accounts.push(AccountWithMeta::new(db, account));
            }
        }

        Ok(accounts)
    }

    /// Total number of accounts matching `query` -- the count companion to
    /// `accounts`, for a gateway to compute total pages. Uses the same filter as
    /// `accounts` (shared `account_matches`) so the two can never disagree.
    #[graphql(cache_control(max_age = 3600))]
    async fn accounts_count(
        &self,
        ctx: &Context<'_>,
        query: Option<AccountQueryInput>,
    ) -> Result<u32> {
        let db = db(ctx);

        // public-key query: exactly the account, if it exists and matches (0 or 1).
        if let Some(public_key) = query.as_ref().and_then(|q| q.public_key.as_ref()) {
            let pk = PublicKey::new(public_key).map_err(|_| {
                async_graphql::Error::new(format!("Invalid public key: {}", public_key))
            })?;
            let token = query.as_ref().and_then(|q| q.token.as_ref()).map_or_else(
                || Ok(TokenAddress::default()),
                |t| {
                    TokenAddress::new(t)
                        .ok_or_else(|| async_graphql::Error::new(format!("Invalid token: {t}")))
                },
            )?;
            let username = db.get_username(&pk).ok().flatten().map(|u| u.0);
            let count = db
                .get_best_account_display(&pk, &token)?
                .filter(|acct| {
                    query
                        .as_ref()
                        .is_none_or(|q| q.matches(acct, username.as_ref()))
                })
                .is_some();
            return Ok(count as u32);
        }

        // token query: count accounts on that token that match.
        if let Some(token) = query.as_ref().and_then(|q| q.token.as_ref()) {
            return query.as_ref().unwrap().token_count_handler(db, token);
        }

        // default: count all matching accounts across the ledger. Iteration order
        // is irrelevant for a count, so scan from the start.
        let iter = match query.as_ref().and_then(|q| q.zkapp) {
            None | Some(false) => db
                .best_ledger_account_balance_iterator(IteratorMode::Start)
                .flatten(),
            Some(true) => db
                .zkapp_best_ledger_account_balance_iterator(IteratorMode::Start)
                .flatten(),
        };

        let mut count = 0u32;
        for (_, value) in iter {
            if account_matches(query.as_ref(), db, &value)?.is_some() {
                count += 1;
            }
        }
        Ok(count)
    }
}

impl AccountQueryInput {
    fn matches(&self, account: &account::Account, username: Option<&String>) -> bool {
        let AccountQueryInput {
            public_key,
            delegate,
            username: query_username_prefix,
            balance,
            balance_gt,
            balance_gte,
            balance_lt,
            balance_lte,
            balance_ne,
            token,
            zkapp,
        } = self;

        if let Some(public_key) = public_key {
            if *public_key != account.public_key.0 {
                return false;
            }
        }

        if let Some(delegate) = delegate {
            if *delegate != account.delegate.0 .0 {
                return false;
            }
        }

        if let Some(username_prefix) = query_username_prefix {
            if username.is_none_or(|u| {
                !u.to_lowercase()
                    .starts_with(&username_prefix.to_lowercase())
            }) {
                return false;
            }
        }

        if let Some(balance) = balance {
            if account.balance.0 != *balance {
                return false;
            }
        }

        if let Some(balance_gt) = balance_gt {
            if account.balance.0 <= *balance_gt {
                return false;
            }
        }

        if let Some(balance_gte) = balance_gte {
            if account.balance.0 < *balance_gte {
                return false;
            }
        }

        if let Some(balance_lt) = balance_lt {
            if account.balance.0 >= *balance_lt {
                return false;
            }
        }

        if let Some(balance_lte) = balance_lte {
            if account.balance.0 > *balance_lte {
                return false;
            }
        }

        if let Some(balance_ne) = balance_ne {
            if account.balance.0 == *balance_ne {
                return false;
            }
        }

        if let Some(token) = token.as_ref() {
            if account
                .token
                .as_ref()
                .map_or(token != MINA_TOKEN_ADDRESS, |t| {
                    *t != TokenAddress::new(token).expect("valid token address")
                })
            {
                return false;
            }
        }

        if let Some(zkapp) = zkapp {
            if account.is_zkapp_account() != *zkapp {
                return false;
            }
        }

        true
    }

    fn token_query_handler(
        &self,
        db: &std::sync::Arc<IndexerStore>,
        token: &str,
        sort_by: AccountSortByInput,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<AccountWithMeta>> {
        // validate token
        if TokenAddress::new(token).is_none() {
            return Err(async_graphql::Error::new(format!(
                "Invalid token address: {}",
                token
            )));
        }

        // iterator mode
        let mut start = [0u8; TokenAddress::LEN + U64_LEN + 1];
        start[..TokenAddress::LEN].copy_from_slice(token.as_bytes());

        let mode = match sort_by {
            AccountSortByInput::BalanceAsc => {
                IteratorMode::From(&start, speedb::Direction::Forward)
            }
            AccountSortByInput::BalanceDesc => {
                // go beyond current token accounts
                start[TokenAddress::LEN..][..U64_LEN].copy_from_slice(&u64::MAX.to_be_bytes());
                start[TokenAddress::LEN..][U64_LEN..].copy_from_slice("Z".as_bytes());

                IteratorMode::From(&start, speedb::Direction::Reverse)
            }
        };

        // iterator
        let iter = match self.zkapp {
            None | Some(false) => db.best_ledger_account_balance_iterator(mode).flatten(),
            Some(true) => db
                .zkapp_best_ledger_account_balance_iterator(mode)
                .flatten(),
        };
        let mut accounts = Vec::with_capacity(limit);
        let mut skipped = 0;

        // iterate
        for (key, value) in iter {
            if key[..TokenAddress::LEN] != *token.as_bytes() || accounts.len() >= limit {
                // beyond desired token accounts or limit
                break;
            }

            if let Some(account) = account_matches(Some(self), db, &value)? {
                if skipped < offset {
                    skipped += 1;
                    continue;
                }
                accounts.push(AccountWithMeta::new(db, account));
            }
        }

        Ok(accounts)
    }

    /// Count of accounts on `token` matching this query -- the count companion to
    /// `token_query_handler`, sharing `account_matches`.
    fn token_count_handler(&self, db: &std::sync::Arc<IndexerStore>, token: &str) -> Result<u32> {
        if TokenAddress::new(token).is_none() {
            return Err(async_graphql::Error::new(format!(
                "Invalid token address: {}",
                token
            )));
        }

        let mut start = [0u8; TokenAddress::LEN + U64_LEN + 1];
        start[..TokenAddress::LEN].copy_from_slice(token.as_bytes());
        let mode = IteratorMode::From(&start, speedb::Direction::Forward);

        let iter = match self.zkapp {
            None | Some(false) => db.best_ledger_account_balance_iterator(mode).flatten(),
            Some(true) => db
                .zkapp_best_ledger_account_balance_iterator(mode)
                .flatten(),
        };

        let mut count = 0u32;
        for (key, value) in iter {
            if key[..TokenAddress::LEN] != *token.as_bytes() {
                break;
            }
            if account_matches(Some(self), db, &value)?.is_some() {
                count += 1;
            }
        }
        Ok(count)
    }
}

impl AccountWithMeta {
    /// Account creation fee must already be deducted
    pub fn new(db: &std::sync::Arc<IndexerStore>, account: account::Account) -> Self {
        let pk = &account.public_key;

        Self {
            is_genesis_account: account.genesis_account.is_some(),
            genesis_account: account.genesis_account.map(|amt| amt.0),
            pk_epoch_num_blocks: db
                .get_block_production_pk_epoch_count(pk, None, None)
                .expect("pk epoch block count"),
            pk_total_num_blocks: db
                .get_block_production_pk_total_count(pk)
                .expect("pk total block count"),
            pk_epoch_num_snarks: db
                .get_snarks_pk_epoch_count(pk, None, None)
                .expect("pk epoch snark count"),
            pk_total_num_snarks: db
                .get_snarks_pk_total_count(pk)
                .expect("pk total snark count"),
            pk_epoch_num_user_commands: db
                .get_user_commands_pk_epoch_count(pk, None, None)
                .expect("pk epoch user command count"),
            pk_total_num_user_commands: db
                .get_user_commands_pk_total_count(pk)
                .expect("pk total user command count"),
            pk_epoch_num_zkapp_commands: db
                .get_zkapp_commands_pk_epoch_count(pk, None, None)
                .expect("pk epoch zkapp command count"),
            pk_total_num_zkapp_commands: db
                .get_zkapp_commands_pk_total_count(pk)
                .expect("pk total zkapp command count"),
            pk_epoch_num_internal_commands: db
                .get_internal_commands_pk_epoch_count(pk, None, None)
                .expect("pk epoch internal command count"),
            pk_total_num_internal_commands: db
                .get_internal_commands_pk_total_count(pk)
                .expect("pk total internal command count"),
            block_height: db
                .get_best_block_height()
                .unwrap()
                .expect("best block height"),
            username: db.get_username(pk).expect("username").unwrap_or_default().0,
            account: Account::new(db, account),
        }
    }
}

impl Account {
    /// Creates a GQL account from a ledger account
    fn new(db: &std::sync::Arc<IndexerStore>, account: account::Account) -> Self {
        let permissions = if account.is_zkapp_account() {
            account.permissions.map(Into::into)
        } else {
            None
        };

        Self {
            public_key: PK::new(db, account.public_key),
            delegate: DelegatePK::new(db, account.delegate.0),
            nonce: account.nonce.map_or(0, |n| n.0),
            balance: account.balance.0,
            time_locked: account.timing.is_some(),
            timing: account.timing.map(Into::into),
            token: account
                .token
                .map_or(MINA_TOKEN_ADDRESS.to_string(), |t| t.0),
            zkapp: account.zkapp.map(Into::into),
            receipt_chain_hash: account.receipt_chain_hash.unwrap_or_default().0,
            voting_for: account.voting_for.unwrap_or_default().0,
            permissions,
        }
    }
}

/////////////////
// conversions //
/////////////////

impl From<account::Timing> for Timing {
    fn from(timing: account::Timing) -> Self {
        Self {
            initial_minimum_balance: Some(timing.initial_minimum_balance.0),
            cliff_time: Some(timing.cliff_time.0),
            cliff_amount: Some(timing.cliff_amount.0),
            vesting_period: Some(timing.vesting_period.0),
            vesting_increment: Some(timing.vesting_increment.0),
        }
    }
}

impl From<account::Permissions> for Permissions {
    fn from(value: account::Permissions) -> Self {
        Self {
            edit_state: value.edit_state.to_string(),
            access: value.access.to_string(),
            send: value.send.to_string(),
            receive: value.receive.to_string(),
            set_delegate: value.set_delegate.to_string(),
            set_permissions: value.set_permissions.to_string(),
            set_verification_key: value.set_verification_key.into(),
            set_zkapp_uri: value.set_zkapp_uri.to_string(),
            edit_action_state: value.edit_action_state.to_string(),
            set_token_symbol: value.set_token_symbol.to_string(),
            increment_nonce: value.increment_nonce.to_string(),
            set_voting_for: value.set_voting_for.to_string(),
            set_timing: value.set_timing.to_string(),
        }
    }
}

impl From<(Permission, String)> for PermissionVk {
    fn from(value: (Permission, String)) -> Self {
        Self {
            permission: value.0.to_string(),
            number: value.1,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        base::{amount::Amount, public_key::PublicKey},
        ledger::{account::Account, store::best::BestLedgerStore, token::TokenAddress},
        store::IndexerStore,
        web::graphql::build_schema,
    };
    use quickcheck::{Arbitrary, Gen};
    use std::{collections::HashSet, sync::Arc};
    use tempfile::TempDir;

    // Seed `n` best-ledger accounts with distinct balances (i * 1 MINA) and
    // return their public keys.
    fn seed_accounts(store: &Arc<IndexerStore>, n: u64) -> Vec<PublicKey> {
        let g = &mut Gen::new(1000);
        let token = TokenAddress::default();
        (1..=n)
            .map(|i| {
                let pk = PublicKey::arbitrary(g);
                let account = Account {
                    public_key: pk.clone(),
                    balance: Amount(i * 1_000_000_000),
                    token: Some(token.clone()),
                    // already paid, so the display balance == the seeded balance
                    // (no creation-fee deduction) and the filters are predictable
                    creation_fee_paid: true,
                    ..Default::default()
                };
                store
                    .update_best_account(&pk, &token, None, Some(account), true)
                    .unwrap();
                pk
            })
            .collect()
    }

    async fn count(
        schema: &async_graphql::Schema<
            crate::web::graphql::Root,
            async_graphql::EmptyMutation,
            async_graphql::EmptySubscription,
        >,
        query_arg: &str,
    ) -> u64 {
        let q = format!("{{ accountsCount{query_arg} }}");
        let res = schema.execute(q).await;
        assert!(res.errors.is_empty(), "accountsCount errored: {:?}", res.errors);
        res.data.into_json().unwrap()["accountsCount"]
            .as_u64()
            .unwrap()
    }

    // `accountsCount` is the total-count companion to `accounts`, sharing the same
    // `account_matches` filter so a page and its total can't drift. It doesn't
    // build `AccountWithMeta` (which needs a seeded genesis/best block), so it is
    // unit-testable directly; end-to-end offset paging of the `accounts` list is
    // covered against a real seeded chain by the e2e/hurl integration tests.
    #[tokio::test]
    async fn accounts_count_totals_and_filters() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(IndexerStore::new(dir.path(), true).unwrap());
        // balances 1..=5 MINA
        let pks = seed_accounts(&store, 5);
        assert_eq!(pks.iter().collect::<HashSet<_>>().len(), 5, "distinct pks");
        let schema = build_schema(store, 0, 0, 0, false);

        // no filter -> every account
        assert_eq!(count(&schema, "").await, 5);

        // balance filter narrows the count. balances are 1e9..=5e9; balance_gte 3e9
        // keeps {3,4,5} MINA = 3 accounts. Uses the same predicate the list uses.
        assert_eq!(count(&schema, "(query: { balance_gte: 3000000000 })").await, 3);
        assert_eq!(count(&schema, "(query: { balance_gte: 6000000000 })").await, 0);
        assert_eq!(count(&schema, "(query: { balance_lte: 2000000000 })").await, 2);

        // a valid but unseeded public key -> zero (present-or-not, count is 0/1)
        assert_eq!(
            count(
                &schema,
                "(query: { publicKey: \"B62qmK2RecMoNXcqvt6K9k7yKG81qhyMoXhCfZ15SXNa5ikJaJr3urk\" })"
            )
            .await,
            0
        );
    }
}
