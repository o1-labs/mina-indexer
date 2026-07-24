use crate::{
    base::{amount::Amount, state_hash::StateHash},
    block::{precomputed::PrecomputedBlock, store::BlockStore},
    chain::{store::ChainStore, ChainId},
    command::{internal::store::InternalCommandStore, store::UserCommandStore},
    constants::{epoch_slot, VERSION},
    ledger::store::best::BestLedgerStore,
    snark_work::store::SnarkStore,
    store::{
        version::{IndexerStoreVersion, VersionStore},
        IndexerStore,
    },
    utility::functions::nanomina_to_mina,
    web::{common::unique_block_producers_last_n_blocks, rest::locked_balances::LockedBalances},
};
use actix_web::{get, http::header::ContentType, web::Data, HttpResponse};
use anyhow::Context;
use chrono::DateTime;
use log::{error, trace};
use serde::Serialize;
use std::sync::Arc;

/// Returns blockchain summary information about the current chain
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BlockchainSummary {
    chain_id: String,
    genesis_state_hash: String,

    blockchain_length: u32,
    date_time: String,
    epoch: u32,
    slot: u32,
    global_slot: u32,

    min_window_density: u32,

    // ledger hashes
    next_epoch_ledger_hash: String,
    snarked_ledger_hash: String,
    staged_ledger_hash: String,
    staking_epoch_ledger_hash: String,

    // self & parent hash
    state_hash: String,
    previous_state_hash: String,

    // currency
    total_currency: String,
    locked_supply: String,
    circulating_supply: String,

    // accounts
    total_num_accounts: u32,
    total_num_mina_accounts: u32,
    total_num_zkapp_accounts: u32,
    total_num_mina_zkapp_accounts: u32,

    // blocks
    epoch_num_blocks: u32,
    total_num_blocks: u32,

    epoch_num_canonical_blocks: u32,

    num_unique_block_producers: Option<u32>,

    // SNARKs
    epoch_num_snarks: u32,
    total_num_snarks: u32,
    // epoch_num_canonical_snarks: u32,
    total_num_canonical_snarks: u32,

    // all user commands
    epoch_num_user_commands: u32,
    total_num_user_commands: u32,

    // applied user commands
    // epoch_num_applied_user_commands: u32,
    total_num_applied_user_commands: u32,

    // failed user commands
    // epoch_num_failed_user_commands: u32,
    total_num_failed_user_commands: u32,

    // canonical user commands
    // epoch_num_canonical_user_commands: u32,
    total_num_canonical_user_commands: u32,

    // applied canonical user commands
    // epoch_num_applied_canonical_user_commands: u32,
    total_num_applied_canonical_user_commands: u32,

    // failed canonical user commands
    // epoch_num_failed_canonical_user_commands: u32,
    total_num_failed_canonical_user_commands: u32,

    // zkapp user commands
    epoch_num_zkapp_commands: u32,
    total_num_zkapp_commands: u32,

    // applied zkapp commands
    // epoch_num_applied_zkapp_commands: u32,
    total_num_applied_zkapp_commands: u32,

    // failed zkapp commands
    // epoch_num_failed_zkapp_commands: u32,
    total_num_failed_zkapp_commands: u32,

    // canonical zkapp commands
    // epoch_num_canonical_zkapp_commands: u32,
    total_num_canonical_zkapp_commands: u32,

    // applied canonical zkapp commands
    // epoch_num_applied_canonical_zkapp_commands: u32,
    total_num_applied_canonical_zkapp_commands: u32,

    // failed canonical zkapp commands
    // epoch_num_failed_canonical_zkapp_commands: u32,
    total_num_failed_canonical_zkapp_commands: u32,

    // internal commands
    epoch_num_internal_commands: u32,
    total_num_internal_commands: u32,
    // epoch_num_canonical_internal_commands: u32,
    total_num_canonical_internal_commands: u32,

    // version
    db_version: String,
    indexer_version: String,
}

fn millis_to_date_string(millis: i64) -> String {
    let date_time = DateTime::from_timestamp_millis(millis).unwrap();
    // RFC 2822 date format
    date_time.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

struct SummaryInput {
    chain_id: ChainId,
    genesis_state_hash: StateHash,

    best_tip: PrecomputedBlock,
    locked_balance: Option<Amount>,

    // accounts
    total_num_accounts: u32,
    total_num_mina_accounts: u32,
    total_num_zkapp_accounts: u32,
    total_num_mina_zkapp_accounts: u32,

    // blocks
    epoch_num_blocks: u32,
    total_num_blocks: u32,

    epoch_num_canonical_blocks: u32,

    /// Unique block producer count in last n blocks
    num_unique_block_producers: Option<u32>,

    // SNARKs
    epoch_num_snarks: u32,
    total_num_snarks: u32,
    // epoch_num_canonical_snarks: u32,
    total_num_canonical_snarks: u32,

    // all user commands
    epoch_num_user_commands: u32,
    total_num_user_commands: u32,
    // epoch_num_applied_user_commands: u32,
    total_num_applied_user_commands: u32,
    // epoch_num_failed_user_commands: u32,
    total_num_failed_user_commands: u32,
    // epoch_num_canonical_user_commands: u32,
    total_num_canonical_user_commands: u32,
    // epoch_num_applied_canonical_user_commands: u32,
    total_num_applied_canonical_user_commands: u32,
    // epoch_num_failed_canonical_user_commands: u32,
    total_num_failed_canonical_user_commands: u32,

    // zkapp commands
    epoch_num_zkapp_commands: u32,
    total_num_zkapp_commands: u32,
    // epoch_num_applied_zkapp_commands: u32,
    total_num_applied_zkapp_commands: u32,
    // epoch_num_failed_zkapp_commands: u32,
    total_num_failed_zkapp_commands: u32,
    // epoch_num_canonical_zkapp_commands: u32,
    total_num_canonical_zkapp_commands: u32,
    // epoch_num_applied_canonical_zkapp_commands: u32,
    total_num_applied_canonical_zkapp_commands: u32,
    // epoch_num_failed_canonical_zkapp_commands: u32,
    total_num_failed_canonical_zkapp_commands: u32,

    // internal commands
    epoch_num_internal_commands: u32,
    total_num_internal_commands: u32,
    // epoch_num_canonical_internal_commands: u32,
    total_num_canonical_internal_commands: u32,

    // version
    db_version: IndexerStoreVersion,
    indexer_version: String,
}

impl BlockchainSummary {
    fn calculate_summary(input: SummaryInput) -> Option<Self> {
        let SummaryInput {
            chain_id,
            genesis_state_hash,

            best_tip,
            locked_balance,

            total_num_accounts,
            total_num_mina_accounts,
            total_num_zkapp_accounts,
            total_num_mina_zkapp_accounts,

            epoch_num_blocks,
            total_num_blocks,
            epoch_num_canonical_blocks,
            num_unique_block_producers,

            epoch_num_snarks,
            total_num_snarks,
            total_num_canonical_snarks,

            epoch_num_user_commands,
            total_num_user_commands,
            total_num_canonical_user_commands,

            total_num_applied_user_commands,
            total_num_applied_canonical_user_commands,
            total_num_failed_user_commands,
            total_num_failed_canonical_user_commands,

            epoch_num_zkapp_commands,
            total_num_zkapp_commands,
            total_num_canonical_zkapp_commands,

            total_num_applied_zkapp_commands,
            total_num_applied_canonical_zkapp_commands,
            total_num_failed_zkapp_commands,
            total_num_failed_canonical_zkapp_commands,

            epoch_num_internal_commands,
            total_num_internal_commands,
            total_num_canonical_internal_commands,

            db_version,
            indexer_version,
        } = input;

        let chain_id = chain_id.to_string();
        let genesis_state_hash = genesis_state_hash.to_string();
        let blockchain_length = best_tip.blockchain_length();
        let date_time = millis_to_date_string(best_tip.timestamp() as i64);
        let epoch = best_tip.epoch_count();
        let global_slot = best_tip.global_slot_since_genesis();
        let min_window_density = best_tip.min_window_density();
        let next_epoch_ledger_hash = best_tip.next_epoch_ledger_hash().0;
        let previous_state_hash = best_tip.previous_state_hash().0;
        let slot = epoch_slot(global_slot);
        let snarked_ledger_hash = best_tip.snarked_ledger_hash().0;
        let staged_ledger_hash = best_tip.staged_ledger_hash().0;
        let staking_epoch_ledger_hash = best_tip.staking_epoch_ledger_hash().0;
        let state_hash = best_tip.state_hash().0;
        let total_currency_u64 = best_tip.total_currency();
        let locked_currency_u64 = locked_balance.map(|a| a.0).unwrap_or_default();
        let total_currency = nanomina_to_mina(total_currency_u64);
        let circulating_supply = nanomina_to_mina(total_currency_u64 - locked_currency_u64);
        let locked_supply = nanomina_to_mina(locked_currency_u64);
        let db_version = db_version.to_string();

        Some(Self {
            chain_id,
            genesis_state_hash,

            date_time,
            epoch,
            blockchain_length,
            slot,
            global_slot,
            min_window_density,

            next_epoch_ledger_hash,
            snarked_ledger_hash,
            staged_ledger_hash,
            staking_epoch_ledger_hash,

            state_hash,
            previous_state_hash,

            total_currency,
            locked_supply,
            circulating_supply,

            total_num_accounts,
            total_num_mina_accounts,
            total_num_zkapp_accounts,
            total_num_mina_zkapp_accounts,

            epoch_num_blocks,
            total_num_blocks,
            epoch_num_canonical_blocks,
            num_unique_block_producers,

            epoch_num_snarks,
            total_num_snarks,
            total_num_canonical_snarks,

            epoch_num_user_commands,
            total_num_user_commands,

            total_num_applied_user_commands,
            total_num_failed_user_commands,
            total_num_canonical_user_commands,
            total_num_applied_canonical_user_commands,
            total_num_failed_canonical_user_commands,

            epoch_num_zkapp_commands,
            total_num_zkapp_commands,

            total_num_applied_zkapp_commands,
            total_num_failed_zkapp_commands,
            total_num_canonical_zkapp_commands,
            total_num_applied_canonical_zkapp_commands,
            total_num_failed_canonical_zkapp_commands,

            epoch_num_internal_commands,
            total_num_internal_commands,
            total_num_canonical_internal_commands,

            db_version,
            indexer_version,
        })
    }
}

/// Liveness/readiness probe. Returns the best-tip height and how far behind
/// wall-clock it is, plus a `synced` flag (tip within ~2 block slots of now).
/// 200 once a best tip exists; 503 while still initializing.
#[get("/health")]
pub async fn get_health(store: Data<Arc<IndexerStore>>) -> HttpResponse {
    use std::time::{SystemTime, UNIX_EPOCH};

    match store.as_ref().get_best_block() {
        Ok(Some(best_tip)) => {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or_default();
            let tip_ms = best_tip.timestamp();
            let tip_age_seconds = now_ms.saturating_sub(tip_ms) / 1000;
            let block_slot_secs = crate::constants::MAINNET_BLOCK_SLOT_TIME_MILLIS / 1000;
            let synced = tip_age_seconds < 2 * block_slot_secs;

            HttpResponse::Ok().json(serde_json::json!({
                "status": "ok",
                "synced": synced,
                "tip_height": best_tip.blockchain_length(),
                "tip_timestamp_ms": tip_ms,
                "tip_age_seconds": tip_age_seconds,
            }))
        }
        _ => HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "initializing",
            "synced": false,
        })),
    }
}

/// Readiness lag budget (seconds): `/readyz` reports ready only while the best
/// tip is at most this old. Override with `MINA_READY_MAX_LAG_SECS`; the
/// default (600s = 10 min) tolerates a few slots of normal lag / a brief reorg
/// without flapping, while still catching a genuinely-behind or stalled
/// indexer.
static READY_MAX_LAG_SECS: std::sync::LazyLock<u64> = std::sync::LazyLock::new(|| {
    std::env::var("MINA_READY_MAX_LAG_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600)
});

/// Kubernetes **liveness** probe: 200 while the process is up and the store
/// answers. Deliberately independent of sync state -- a catching-up indexer is
/// alive and must not be restarted.
#[get("/healthz")]
pub async fn get_healthz(store: Data<Arc<IndexerStore>>) -> HttpResponse {
    match store.as_ref().get_best_block_height() {
        Ok(_) => HttpResponse::Ok().json(serde_json::json!({ "status": "ok" })),
        Err(_) => HttpResponse::ServiceUnavailable()
            .json(serde_json::json!({ "status": "store_unavailable" })),
    }
}

/// Kubernetes **readiness** probe: 200 only when the best tip is fresh (within
/// `MINA_READY_MAX_LAG_SECS` of now), else 503 -- so a bootstrapping or
/// behind-the-tip indexer is pulled from the Service/load balancer and does not
/// serve stale data. Clients can gate on this before trusting query results.
#[get("/readyz")]
pub async fn get_readyz(store: Data<Arc<IndexerStore>>) -> HttpResponse {
    use std::time::{SystemTime, UNIX_EPOCH};

    match store.as_ref().get_best_block() {
        Ok(Some(best_tip)) => {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or_default();
            let tip_age_seconds = now_ms.saturating_sub(best_tip.timestamp()) / 1000;
            let max_lag = *READY_MAX_LAG_SECS;
            let ready = tip_age_seconds <= max_lag;

            let body = serde_json::json!({
                "status": if ready { "ready" } else { "catching_up" },
                "ready": ready,
                "tip_height": best_tip.blockchain_length(),
                "tip_age_seconds": tip_age_seconds,
                "max_lag_seconds": max_lag,
            });
            if ready {
                HttpResponse::Ok().json(body)
            } else {
                HttpResponse::ServiceUnavailable().json(body)
            }
        }
        // No best block yet: still bootstrapping the database.
        Ok(None) => HttpResponse::ServiceUnavailable()
            .json(serde_json::json!({ "status": "bootstrapping", "ready": false })),
        Err(_) => HttpResponse::ServiceUnavailable()
            .json(serde_json::json!({ "status": "store_unavailable", "ready": false })),
    }
}

#[cfg(test)]
mod health_tests {
    use super::{get_healthz, get_readyz};
    use crate::store::IndexerStore;
    use actix_web::{test, web::Data, App};
    use std::sync::Arc;
    use tempfile::TempDir;

    // On a fresh/empty store: liveness is 200 (the process is up), but readiness
    // is 503 (no best block yet == still bootstrapping) -- so k8s keeps the pod
    // running while pulling it from the Service until it has data.
    #[actix_web::test]
    async fn healthz_live_readyz_not_ready_while_bootstrapping() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(IndexerStore::new(dir.path(), true).unwrap());
        let app = test::init_service(
            App::new()
                .app_data(Data::new(store))
                .service(get_healthz)
                .service(get_readyz),
        )
        .await;

        let live =
            test::call_service(&app, test::TestRequest::get().uri("/healthz").to_request()).await;
        assert_eq!(live.status().as_u16(), 200, "liveness must be up");

        let ready =
            test::call_service(&app, test::TestRequest::get().uri("/readyz").to_request()).await;
        assert_eq!(
            ready.status().as_u16(),
            503,
            "readiness must be 503 with no best block"
        );
    }
}

#[get("/summary")]
pub async fn get_blockchain_summary(
    store: Data<Arc<IndexerStore>>,
    locked_balances: Data<Arc<LockedBalances>>,
) -> HttpResponse {
    match try_blockchain_summary(&store, &locked_balances) {
        Ok(Some(body)) => HttpResponse::Ok()
            .content_type(ContentType::json())
            .body(body),
        Ok(None) => HttpResponse::NotFound().finish(),
        Err(e) => {
            // A store read failed mid-summary: return 500 instead of panicking.
            error!("GET /summary failed: {e:#}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

/// Builds the blockchain-summary JSON, or `None` when there's no best tip / the
/// summary can't be computed yet. Any store error propagates as `Err` so the
/// handler can answer 500 rather than unwinding.
fn try_blockchain_summary(
    store: &Data<Arc<IndexerStore>>,
    locked_balances: &Data<Arc<LockedBalances>>,
) -> anyhow::Result<Option<String>> {
    let db = store.as_ref();
    let Some(best_tip) = db.get_best_block()? else {
        return Ok(None);
    };
    trace!("Found best tip: {}", best_tip.summary());

    // accounts
    let total_num_accounts = store
        .get_num_accounts()
        .context("num accounts")?
        .unwrap_or_default();
    let total_num_mina_accounts = store
        .get_num_mina_accounts()
        .context("num mina accounts")?
        .unwrap_or_default();
    let total_num_zkapp_accounts = store
        .get_num_zkapp_accounts()
        .context("num zkapp accounts")?
        .unwrap_or_default();
    let total_num_mina_zkapp_accounts = store
        .get_num_mina_zkapp_accounts()
        .context("num mina zkapp accounts")?
        .unwrap_or_default();

    // aggregated on-chain & off-chain time-locked tokens
    let chain_id = store.get_chain_id().context("chain id")?;
    let genesis_state_hash = store
        .get_block_genesis_state_hash(&best_tip.state_hash())
        .context("genesis state hash lookup")?
        .context("genesis state hash")?;

    let global_slot = best_tip.global_slot_since_genesis();
    let locked_balance = locked_balances.get_locked_amount(global_slot);

    // version info
    let db_version = store.get_db_version().context("store version")?;
    let indexer_version = VERSION.to_string();

    // epoch & total data counts
    let epoch_num_blocks = store
        .get_block_production_epoch_count(None, None)
        .context("epoch blocks count")?;
    let total_num_blocks = store
        .get_block_production_total_count()
        .context("total blocks count")?;

    let epoch_num_canonical_blocks = store
        .get_block_production_canonical_epoch_count(None, None)
        .context("epoch canonical blocks count")?;
    let total_num_canonical_blocks = store
        .get_block_production_canonical_total_count()
        .context("total canonical blocks count")?;

    let num_unique_block_producers =
        unique_block_producers_last_n_blocks(db, total_num_canonical_blocks)
            .context("unique block producers")?;

    let epoch_num_snarks = store
        .get_snarks_epoch_count(None, None)
        .context("epoch snarks count")?;
    let total_num_snarks = store
        .get_snarks_total_count()
        .context("total snarks count")?;
    let total_num_canonical_snarks = store
        .get_snarks_total_canonical_count()
        .context("total canonical snarks count")?;

    // user commands
    let epoch_num_user_commands = store
        .get_user_commands_epoch_count(None, None)
        .context("epoch user commands count")?;
    let total_num_user_commands = store
        .get_user_commands_total_count()
        .context("total user commands count")?;

    // applied user commands
    let total_num_applied_user_commands = store
        .get_applied_user_commands_count()
        .context("total applied user commands count")?;
    let total_num_canonical_user_commands = store
        .get_canonical_user_commands_count()
        .context("total canonical user commands count")?;

    // applied/failed canonical user commands
    let total_num_applied_canonical_user_commands = store
        .get_applied_canonical_user_commands_count()
        .context("total applied canonical user commands count")?;
    let total_num_failed_canonical_user_commands = store
        .get_failed_canonical_user_commands_count()
        .context("total failed canonical user commands count")?;

    // total failed user commands
    let total_num_failed_user_commands = store
        .get_failed_user_commands_count()
        .context("total failed user commands count")?;

    // zkapp commands
    let epoch_num_zkapp_commands = store
        .get_zkapp_commands_epoch_count(None, None)
        .context("epoch zkapp commands count")?;
    let total_num_zkapp_commands = store
        .get_zkapp_commands_total_count()
        .context("total zkapp commands count")?;

    // applied zkapp commands
    let total_num_applied_zkapp_commands = store
        .get_applied_zkapp_commands_count()
        .context("total applied zkapp commands count")?;
    let total_num_canonical_zkapp_commands = store
        .get_canonical_zkapp_commands_count()
        .context("total canonical zkapp commands count")?;

    // applied/failed canonical zkapp commands
    let total_num_applied_canonical_zkapp_commands = store
        .get_applied_canonical_zkapp_commands_count()
        .context("total applied canonical zkapp commands count")?;
    let total_num_failed_canonical_zkapp_commands = store
        .get_failed_canonical_zkapp_commands_count()
        .context("total failed canonical zkapp commands count")?;

    // total failed zkapp commands
    let total_num_failed_zkapp_commands = store
        .get_failed_zkapp_commands_count()
        .context("total failed zkapp commands count")?;

    // internal commands
    let epoch_num_internal_commands = store
        .get_internal_commands_epoch_count(None, None)
        .context("epoch internal commands count")?;
    let total_num_internal_commands = store
        .get_internal_commands_total_count()
        .context("total internal commands count")?;

    // canonical internal commands
    let total_num_canonical_internal_commands = store
        .get_canonical_internal_commands_count()
        .context("total number of canonical internal commands")?;

    let Some(summary) = BlockchainSummary::calculate_summary(SummaryInput {
        chain_id,
        genesis_state_hash,

        best_tip,
        locked_balance,
        db_version,
        indexer_version,

        total_num_accounts,
        total_num_mina_accounts,
        total_num_zkapp_accounts,
        total_num_mina_zkapp_accounts,

        epoch_num_blocks,
        total_num_blocks,

        epoch_num_canonical_blocks,

        num_unique_block_producers,

        epoch_num_snarks,
        total_num_snarks,
        total_num_canonical_snarks,

        epoch_num_user_commands,
        total_num_user_commands,
        total_num_canonical_user_commands,

        total_num_applied_user_commands,
        total_num_applied_canonical_user_commands,

        total_num_failed_user_commands,
        total_num_failed_canonical_user_commands,

        epoch_num_zkapp_commands,
        total_num_zkapp_commands,
        total_num_canonical_zkapp_commands,

        total_num_applied_zkapp_commands,
        total_num_applied_canonical_zkapp_commands,

        total_num_failed_zkapp_commands,
        total_num_failed_canonical_zkapp_commands,

        epoch_num_internal_commands,
        total_num_internal_commands,
        total_num_canonical_internal_commands,
    }) else {
        return Ok(None);
    };

    trace!("Blockchain summary: {summary:?}");
    Ok(Some(serde_json::to_string_pretty(&summary)?))
}
