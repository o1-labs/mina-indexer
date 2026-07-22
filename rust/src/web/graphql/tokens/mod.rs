//! GraphQL `tokens` & `tokenHolders` endpoints

use super::{
    accounts::{self, AccountWithMeta},
    db,
    pk::PK,
};
use crate::{
    base::public_key::PublicKey,
    constants::MINA_TOKEN_ADDRESS,
    ledger::{self, account, store::best::BestLedgerStore, token::TokenAddress},
    store::{zkapp::tokens::ZkappTokenStore, IndexerStore},
    utility::store::common::U64_LEN,
};
use async_graphql::{Context, Enum, InputObject, Object, Result, SimpleObject};
use speedb::Direction;
use std::sync::Arc;

#[derive(InputObject, Default)]
pub struct TokensQueryInput {
    /// Input token address
    token: Option<String>,

    /// Input token owner
    owner: Option<String>,

    /// Input token symbol
    symbol: Option<String>,

    /// Input token supply
    supply: Option<u64>,
}

#[derive(SimpleObject)]
pub struct Token {
    /// Value token
    #[graphql(flatten)]
    token: TokenSimple,

    /// Value count of token holders
    #[graphql(name = "num_holders")]
    num_holders: u32,

    /// Value total count of token transactions
    #[graphql(name = "total_num_txns")]
    total_num_txns: u32,

    /// Value total count of locked tokens
    #[graphql(name = "total_num_locked")]
    total_num_locked: u64,

    /// Value total count of tokens
    #[graphql(name = "total_num_tokens")]
    total_num_tokens: u32,
}

#[derive(Enum, Default, Copy, Clone, Eq, PartialEq)]
pub enum TokensSortByInput {
    #[default]
    #[graphql(name = "SUPPLY_DESC")]
    SupplyDesc,

    #[graphql(name = "SUPPLY_ASC")]
    SupplyAsc,

    #[graphql(name = "NUM_HOLDERS_DESC")]
    NumHoldersDesc,

    #[graphql(name = "NUM_HOLDERS_ASC")]
    NumHoldersAsc,

    #[graphql(name = "TOTAL_NUM_TXNS_DESC")]
    NumTxnsDesc,

    #[graphql(name = "TOTAL_NUM_TXNS_ASC")]
    NumTxnsAsc,
}

#[derive(SimpleObject)]
pub struct TokenSimple {
    /// Value token address
    token: String,

    /// Value token supply (nano)
    supply: u64,

    /// Value token owner
    #[graphql(deprecation = "Use owner_account instead")]
    owner: Option<String>,

    /// Value token owner account
    #[graphql(name = "owner_account")]
    owner_account: Option<PK>,

    /// Value token symbol
    symbol: Option<String>,
}

#[derive(InputObject)]
pub struct TokenHoldersQueryInput {
    /// Input token address of holder account
    token: Option<String>,

    /// Input public key of holder account
    holder: Option<String>,
}

#[derive(SimpleObject)]
pub struct TokenHolder {
    /// Value token address
    token: String,

    /// Value token supply (nano)
    supply: u64,

    /// Value token owner public key
    #[graphql(deprecation = "Use owner_account instead")]
    owner: Option<String>,

    /// Value token owner account
    #[graphql(name = "owner_account")]
    owner_account: Option<PK>,

    /// Value token symbol
    symbol: Option<String>,

    /// Value token holder account
    account: accounts::Account,
}

#[derive(Default, Enum, Copy, Clone, Eq, PartialEq)]
pub enum TokenHoldersSortByInput {
    #[graphql(name = "BALANCE_ASC")]
    BalanceAsc,

    #[default]
    #[graphql(name = "BALANCE_DESC")]
    BalanceDesc,
}

struct TokenWithMeta {
    token: ledger::token::Token,
    num_holders: u32,
    total_num_txns: u32,
    total_num_locked: u64,
    total_num_tokens: u32,
}

struct TokenAccount {
    account: accounts::AccountWithMeta,
    token: ledger::token::Token,
}

#[derive(Default)]
pub struct TokensQueryRoot;

#[Object]
impl TokensQueryRoot {
    #[graphql(cache_control(max_age = 3600))]
    async fn tokens(
        &self,
        ctx: &Context<'_>,
        query: Option<TokensQueryInput>,
        sort_by: Option<TokensSortByInput>,
        #[graphql(default = 100)] limit: usize,
        // `offset`: tokens to skip before `limit` — pages the full token set
        // (this endpoint has no keyset filter).
        #[graphql(default = 0)] offset: usize,
    ) -> Result<Vec<Token>> {
        let limit = limit.min(crate::constants::GRAPHQL_MAX_PAGE_SIZE);
        let db = db(ctx);

        // specific token query
        if query
            .as_ref()
            .map(|q| q.token.is_some())
            .unwrap_or_default()
        {
            if let Some(token) = query
                .as_ref()
                .and_then(|q| q.token.as_ref().and_then(TokenAddress::new))
            {
                if let Some(token) = db.get_token(&token)? {
                    let token = TokenWithMeta::new(db, token);
                    return Ok(vec![Token::new(db, token)]);
                }
            } else {
                return Err(async_graphql::Error::new(format!(
                    "Invalid token address: {}",
                    query.as_ref().unwrap().token.as_ref().unwrap()
                )));
            }
        }

        // default query
        let mut tokens = Vec::with_capacity(limit);
        let mut skipped = 0usize;
        for (_, value) in db.token_iterator().flatten() {
            if tokens.len() >= limit {
                // gone beyond limit
                break;
            }

            let token = serde_json::from_slice(&value)?;
            if TokensQueryInput::matches(query.as_ref(), &token) {
                if skipped < offset {
                    skipped += 1;
                    continue;
                }
                let token = TokenWithMeta::new(db, token);
                tokens.push(Token::new(db, token));
            }
        }

        // sort tokens
        use TokensSortByInput::*;
        match sort_by.unwrap_or_default() {
            SupplyDesc => tokens.sort_by(supply_desc),
            SupplyAsc => tokens.sort_by(supply_asc),
            NumHoldersDesc => tokens.sort_by(num_holders_desc),
            NumHoldersAsc => tokens.sort_by(num_holders_asc),
            NumTxnsDesc => tokens.sort_by(total_num_txns_desc),
            NumTxnsAsc => tokens.sort_by(total_num_txns_asc),
        }

        Ok(tokens)
    }

    /// Total tokens matching `query` -- the count companion to `tokens`, for a
    /// gateway to compute total pages. Applies the same `matches` filter over
    /// the same iterator without building each `Token`, so it stays cheap.
    #[graphql(cache_control(max_age = 3600))]
    async fn tokens_count(
        &self,
        ctx: &Context<'_>,
        query: Option<TokensQueryInput>,
    ) -> Result<u32> {
        let db = db(ctx);

        // specific-token query is 0 or 1
        if let Some(addr) = query.as_ref().and_then(|q| q.token.as_ref()) {
            let addr = TokenAddress::new(addr).ok_or_else(|| {
                async_graphql::Error::new(format!("Invalid token address: {addr}"))
            })?;
            return Ok(db.get_token(&addr)?.is_some() as u32);
        }

        let mut count = 0u32;
        for (_, value) in db.token_iterator().flatten() {
            let token = serde_json::from_slice(&value)?;
            if TokensQueryInput::matches(query.as_ref(), &token) {
                count += 1;
            }
        }
        Ok(count)
    }

    #[graphql(cache_control(max_age = 3600))]
    async fn token_holders(
        &self,
        ctx: &Context<'_>,
        query: Option<TokenHoldersQueryInput>,
        sort_by: Option<TokenHoldersSortByInput>,
        #[graphql(default = 100)] limit: usize,
        // `offset`: holders to skip before `limit` — pages the full holder set
        // (this endpoint has no keyset filter).
        #[graphql(default = 0)] offset: usize,
    ) -> Result<Vec<TokenHolder>> {
        let limit = limit.min(crate::constants::GRAPHQL_MAX_PAGE_SIZE);
        let db = db(ctx);
        let mut holders = Vec::with_capacity(limit);
        let mut skipped = 0usize;

        // specific token's holder accounts
        if let Some(token) = query.as_ref().and_then(|q| q.token.as_ref()) {
            // validate token address
            if let Some(token) = TokenAddress::new(token) {
                let direction = match sort_by {
                    Some(TokenHoldersSortByInput::BalanceDesc) | None => speedb::Direction::Reverse,
                    Some(TokenHoldersSortByInput::BalanceAsc) => speedb::Direction::Forward,
                };

                let mut start = [0u8; TokenAddress::LEN + U64_LEN + PublicKey::LEN];
                start[..TokenAddress::LEN].copy_from_slice(token.0.as_bytes());

                if let Direction::Reverse = direction {
                    start[TokenAddress::LEN..][..U64_LEN].copy_from_slice(&u64::MAX.to_be_bytes());
                    start[TokenAddress::LEN..][U64_LEN..]
                        .copy_from_slice(PublicKey::upper_bound().0.as_bytes());
                };

                let mode = speedb::IteratorMode::From(&start, direction);
                for (key, value) in db.best_ledger_account_balance_iterator(mode).flatten() {
                    if key[..TokenAddress::LEN] != *token.0.as_bytes() || holders.len() >= limit {
                        // beyond token or limit
                        break;
                    }

                    let account = serde_json::from_slice::<account::Account>(&value)?
                        .deduct_mina_account_creation_fee();

                    if TokenHoldersQueryInput::matches(query.as_ref(), &account) {
                        if skipped < offset {
                            skipped += 1;
                            continue;
                        }
                        let token = db.get_token(&token)?.unwrap_or_default();
                        let account = TokenAccount {
                            token,
                            account: AccountWithMeta::new(db, account),
                        };

                        holders.push(TokenHolder::new(db, account));
                    }
                }
            } else {
                return Err(async_graphql::Error::new(format!(
                    "Invalid token address: {}",
                    token
                )));
            }

            return Ok(holders);
        }

        // specific holder's token accounts
        if let Some(holder) = query.as_ref().and_then(|q| q.holder.as_ref()) {
            // validate holder pk
            let holder = match PublicKey::new(holder) {
                Ok(holder) => holder,
                Err(_) => {
                    return Err(async_graphql::Error::new(format!(
                        "Invalid holder public key: {}",
                        holder
                    )))
                }
            };

            for (key, value) in db.tokens_pk_iterator(&holder).flatten() {
                if key[..PublicKey::LEN] != *holder.0.as_bytes() || holders.len() >= limit {
                    // beyond public key or limit
                    break;
                }

                let account: account::Account = serde_json::from_slice(&value)?;
                let token = account.token.to_owned().unwrap_or_default();

                if TokenHoldersQueryInput::matches(query.as_ref(), &account) {
                    if skipped < offset {
                        skipped += 1;
                        continue;
                    }
                    let account = TokenAccount {
                        token: db.get_token(&token)?.expect("token"),
                        account: AccountWithMeta::new(db, account),
                    };

                    holders.push(TokenHolder::new(db, account));
                }
            }
        }

        Ok(holders)
    }

    /// Total token holders matching `query` -- the count companion to
    /// `tokenHolders`. Applies the same `matches` filter over the same iterator
    /// (by token, or by holder) without building each `TokenHolder`, so it
    /// stays cheap.
    #[graphql(cache_control(max_age = 3600))]
    async fn token_holders_count(
        &self,
        ctx: &Context<'_>,
        query: Option<TokenHoldersQueryInput>,
    ) -> Result<u32> {
        let db = db(ctx);
        let mut count = 0u32;

        // holders of a specific token
        if let Some(token) = query.as_ref().and_then(|q| q.token.as_ref()) {
            let token = TokenAddress::new(token).ok_or_else(|| {
                async_graphql::Error::new(format!("Invalid token address: {token}"))
            })?;

            let mut start = [0u8; TokenAddress::LEN + U64_LEN + PublicKey::LEN];
            start[..TokenAddress::LEN].copy_from_slice(token.0.as_bytes());
            let mode = speedb::IteratorMode::From(&start, speedb::Direction::Forward);

            for (key, value) in db.best_ledger_account_balance_iterator(mode).flatten() {
                if key[..TokenAddress::LEN] != *token.0.as_bytes() {
                    break;
                }
                let account = serde_json::from_slice::<account::Account>(&value)?
                    .deduct_mina_account_creation_fee();
                if TokenHoldersQueryInput::matches(query.as_ref(), &account) {
                    count += 1;
                }
            }
            return Ok(count);
        }

        // a specific holder's token accounts
        if let Some(holder) = query.as_ref().and_then(|q| q.holder.as_ref()) {
            let holder = PublicKey::new(holder).map_err(|_| {
                async_graphql::Error::new(format!("Invalid holder public key: {holder}"))
            })?;

            for (key, value) in db.tokens_pk_iterator(&holder).flatten() {
                if key[..PublicKey::LEN] != *holder.0.as_bytes() {
                    break;
                }
                let account: account::Account = serde_json::from_slice(&value)?;
                if TokenHoldersQueryInput::matches(query.as_ref(), &account) {
                    count += 1;
                }
            }
        }

        Ok(count)
    }
}

///////////
// impls //
///////////

impl TokensQueryInput {
    fn matches(query: Option<&Self>, token: &ledger::token::Token) -> bool {
        if let Some(query) = query {
            let Self {
                token: q_token,
                owner,
                symbol,
                supply,
            } = query;

            // token
            if let Some(q_token) = q_token {
                if *q_token != token.token.0 {
                    return false;
                }
            }

            // owner
            if let Some(q_owner) = owner {
                if let Some(owner) = token.owner.as_ref() {
                    if *q_owner != owner.0 {
                        return false;
                    }
                } else {
                    return false;
                }
            }

            // symbol
            if let Some(q_symbol) = symbol {
                if *q_symbol != token.symbol.0 {
                    return false;
                }
            }

            // supply
            if let Some(q_supply) = supply {
                if *q_supply != token.supply.0 {
                    return false;
                }
            }
        }

        true
    }
}

impl TokenHoldersQueryInput {
    fn matches(query: Option<&Self>, account: &ledger::account::Account) -> bool {
        if let Some(query) = query {
            let Self {
                token: q_token,
                holder,
            } = query;

            // token
            if let Some(q_token) = q_token {
                if *q_token != account.token.to_owned().unwrap_or_default().0 {
                    return false;
                }
            }

            // holder
            if let Some(holder) = holder {
                if *holder != account.public_key.0 {
                    return false;
                }
            }
        }

        true
    }
}

impl TokenWithMeta {
    fn new(db: &Arc<IndexerStore>, token: ledger::token::Token) -> Self {
        Self {
            num_holders: if token.token.0 == MINA_TOKEN_ADDRESS {
                db.get_num_mina_accounts()
            } else {
                db.get_token_holders_num(&token.token)
            }
            .expect("num token holders")
            .unwrap_or_default(),
            total_num_txns: db
                .get_token_txns_num(&token.token)
                .expect("num token txns")
                .unwrap_or_default(),
            total_num_tokens: db.get_num_tokens().expect("num tokens"),
            total_num_locked: 0,
            token,
        }
    }
}

/////////////////
// conversions //
/////////////////

impl Token {
    fn new(db: &Arc<IndexerStore>, token: TokenWithMeta) -> Self {
        Self {
            token: TokenSimple::new(db, token.token),
            num_holders: token.num_holders,
            total_num_txns: token.total_num_txns,
            total_num_locked: token.total_num_locked,
            total_num_tokens: token.total_num_tokens,
        }
    }
}

impl TokenSimple {
    fn new(db: &Arc<IndexerStore>, token: ledger::token::Token) -> Self {
        Self {
            token: token.token.0,
            supply: token.supply.0,
            owner: token.owner.as_ref().map(ToString::to_string),
            owner_account: token.owner.map(|pk| PK::new(db, pk)),
            symbol: Some(token.symbol.0),
        }
    }
}

impl TokenHolder {
    fn new(db: &Arc<IndexerStore>, account: TokenAccount) -> Self {
        Self {
            account: account.account.account,
            token: account.token.token.0,
            supply: account.token.supply.0,
            owner: account.token.owner.as_ref().map(ToString::to_string),
            owner_account: account.token.owner.map(|pk| PK::new(db, pk)),
            symbol: Some(account.token.symbol.0),
        }
    }
}

/////////////
// helpers //
/////////////

fn supply_asc(x: &Token, y: &Token) -> std::cmp::Ordering {
    x.token.supply.cmp(&y.token.supply)
}

fn supply_desc(x: &Token, y: &Token) -> std::cmp::Ordering {
    y.token.supply.cmp(&x.token.supply)
}

fn num_holders_asc(x: &Token, y: &Token) -> std::cmp::Ordering {
    x.num_holders.cmp(&y.num_holders)
}

fn num_holders_desc(x: &Token, y: &Token) -> std::cmp::Ordering {
    y.num_holders.cmp(&x.num_holders)
}

fn total_num_txns_asc(x: &Token, y: &Token) -> std::cmp::Ordering {
    x.total_num_txns.cmp(&y.total_num_txns)
}

fn total_num_txns_desc(x: &Token, y: &Token) -> std::cmp::Ordering {
    y.total_num_txns.cmp(&x.total_num_txns)
}

#[cfg(test)]
mod tests {
    use super::TokensQueryInput;
    use crate::{
        base::public_key::PublicKey,
        ledger::token::{Token, TokenAddress, TokenSymbol},
        store::{zkapp::tokens::ZkappTokenStore, IndexerStore},
        web::graphql::build_schema,
    };
    use std::{collections::HashSet, sync::Arc};
    use tempfile::TempDir;

    /// Pagination regression for #95: page a >1000-row result set to completion
    /// with no gaps or duplicates, and confirm `tokensCount` agrees with the
    /// list (unfiltered and filtered). `limit` is capped at
    /// GRAPHQL_MAX_PAGE_SIZE (1000), so the only way to read a larger set
    /// is by paging with `offset`.
    #[tokio::test]
    async fn tokens_offset_pages_past_the_page_cap_and_count_agrees() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(IndexerStore::new(dir.path(), true).unwrap());

        // 1050 distinct tokens; the first 50 share a known owner so we can filter.
        let n = 1050usize;
        let owned = 50usize;
        let owner =
            PublicKey::new("B62qkPg6P2We1SZhCq84ZvDKknrWy8P3Moi99Baz8KFpYsMoFJKHHqF").unwrap();
        for i in 0..n {
            // a valid 50-char token address, distinct per i
            let addr = TokenAddress::new(format!("t{:049}", i)).unwrap();
            let token = Token {
                token: addr,
                owner: (i < owned).then(|| owner.clone()),
                symbol: TokenSymbol::new(""),
                supply: (i as u64).into(),
            };
            store.set_token(&token).unwrap();
        }

        let schema = build_schema(store, 0, 0, 0, false);

        let count = |q: &str| {
            let schema = &schema;
            let q = format!("{{ tokensCount{q} }}");
            async move {
                let res = schema.execute(q).await;
                assert!(
                    res.errors.is_empty(),
                    "tokensCount errored: {:?}",
                    res.errors
                );
                res.data.into_json().unwrap()["tokensCount"]
                    .as_u64()
                    .unwrap() as usize
            }
        };

        // count agrees with the number seeded, unfiltered and filtered by owner
        assert_eq!(count("").await, n);
        assert_eq!(
            count(&format!("(query: {{ owner: \"{owner}\" }})")).await,
            owned
        );

        // page the whole set in chunks of 400 and collect the token addresses
        let page_size = 400usize;
        let mut seen = HashSet::new();
        let mut offset = 0usize;
        loop {
            let q = format!("{{ tokens(limit: {page_size}, offset: {offset}) {{ token }} }}");
            let res = schema.execute(q).await;
            assert!(res.errors.is_empty(), "tokens errored: {:?}", res.errors);
            let page: Vec<String> = res.data.into_json().unwrap()["tokens"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["token"].as_str().unwrap().to_string())
                .collect();
            if page.is_empty() {
                break;
            }
            for addr in page {
                assert!(seen.insert(addr), "duplicate token across pages");
            }
            offset += page_size;
            assert!(offset < n + page_size, "paging did not terminate");
        }

        // every token seen exactly once -> no gaps, no duplicates, past the cap
        assert_eq!(seen.len(), n, "pages should cover every token once");
    }

    #[test]
    fn matches() {
        let query = TokensQueryInput {
            symbol: Some("MINU".to_string()),
            ..Default::default()
        };

        // does not match MINA token
        let mina = Token::mina_with_supply(100000000000000);
        assert!(!TokensQueryInput::matches(Some(&query), &mina));

        // matches MINU token
        let minu = Token {
            token: TokenAddress::new("wfG3GivPMttpt6nQnPuX9eDPnoyA5RJZY23LTc4kkNkCRH2gUd").unwrap(),
            owner: PublicKey::new("B62qkPg6P2We1SZhCq84ZvDKknrWy8P3Moi99Baz8KFpYsMoFJKHHqF").ok(),
            symbol: TokenSymbol::new("MINU"),
            supply: 100000000000000.into(),
        };
        assert!(TokensQueryInput::matches(Some(&query), &minu));
    }
}
