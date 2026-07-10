use crate::{
    base::public_key::PublicKey,
    block::store::BlockStore,
    command::{internal::store::InternalCommandStore, store::UserCommandStore},
    ledger::{account, store::best::BestLedgerStore, token::TokenAddress},
    snark_work::store::SnarkStore,
    store::{username::UsernameStore, IndexerStore},
};
use actix_web::{
    get,
    http::header::ContentType,
    web::{self, Data},
    HttpResponse,
};
use anyhow::Context;
use log::{debug, error};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Account {
    #[serde(flatten)]
    account: account::Account,

    // accounts
    total_num_accounts: u32,
    total_num_zkapp_accounts: u32,

    // blocks
    epoch_num_blocks: u32,
    total_num_blocks: u32,

    // SNARKs
    epoch_num_snarks: u32,
    total_num_snarks: u32,

    // all user commands
    epoch_num_user_commands: u32,
    total_num_user_commands: u32,

    // zkapp user commands
    epoch_num_zkapp_commands: u32,
    total_num_zkapp_commands: u32,

    // internal commands
    epoch_num_internal_commands: u32,
    total_num_internal_commands: u32,
}

#[get("/accounts/{public_key}")]
pub async fn get_account(
    store: Data<Arc<IndexerStore>>,
    public_key: web::Path<String>,
) -> HttpResponse {
    match try_get_account(&store, &public_key) {
        Ok(Some(body)) => HttpResponse::Ok()
            .content_type(ContentType::json())
            .body(body),
        Ok(None) => HttpResponse::NotFound().finish(),
        Err(e) => {
            error!("GET /accounts/{public_key} failed: {e:#}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

/// Builds the account JSON, or `None` when the account isn't in the ledger. Any
/// store error propagates as `Err` so the handler answers 500 rather than
/// panicking.
fn try_get_account(
    store: &Data<Arc<IndexerStore>>,
    public_key: &web::Path<String>,
) -> anyhow::Result<Option<String>> {
    let db = store.as_ref();

    // Reject a malformed public key here: querying the ledger with one panics
    // inside the store decoder. Treated as "not found" (404).
    if !PublicKey::is_valid(public_key.as_str()) {
        return Ok(None);
    }
    let pk: PublicKey = public_key.as_str().into();

    let Some(account) = db.get_best_account(&pk, &TokenAddress::default())? else {
        return Ok(None);
    };
    debug!("Found account in ledger: {account}");

    let account = Account {
        account: account::Account {
            username: db.get_username(&pk).unwrap_or_default(),
            ..account
        },

        // accounts
        total_num_accounts: db
            .get_num_accounts()
            .context("num accounts")?
            .unwrap_or_default(),
        total_num_zkapp_accounts: db
            .get_num_zkapp_accounts()
            .context("num zkapp accounts")?
            .unwrap_or_default(),

        // blocks
        epoch_num_blocks: db
            .get_block_production_pk_epoch_count(&pk, None, None)
            .unwrap_or_default(),
        total_num_blocks: db
            .get_block_production_pk_total_count(&pk)
            .unwrap_or_default(),

        // SNARKs
        epoch_num_snarks: db
            .get_snarks_pk_epoch_count(&pk, None, None)
            .unwrap_or_default(),
        total_num_snarks: db.get_snarks_pk_total_count(&pk).unwrap_or_default(),

        // all user commands
        epoch_num_user_commands: db
            .get_user_commands_pk_epoch_count(&pk, None, None)
            .unwrap_or_default(),
        total_num_user_commands: db.get_user_commands_pk_total_count(&pk).unwrap_or_default(),

        // zkapp user commands
        epoch_num_zkapp_commands: db
            .get_zkapp_commands_pk_epoch_count(&pk, None, None)
            .unwrap_or_default(),
        total_num_zkapp_commands: db
            .get_zkapp_commands_pk_total_count(&pk)
            .unwrap_or_default(),

        // internal commands
        epoch_num_internal_commands: db
            .get_internal_commands_pk_epoch_count(&pk, None, None)
            .unwrap_or_default(),
        total_num_internal_commands: db
            .get_internal_commands_pk_total_count(&pk)
            .unwrap_or_default(),
    };

    Ok(Some(
        serde_json::to_string_pretty(&Account {
            account: account.account.clone().deduct_mina_account_creation_fee(),
            ..account
        })
        .context("serialize account")?,
    ))
}
