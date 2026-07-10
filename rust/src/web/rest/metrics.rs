//! Prometheus `/metrics` endpoint.

use crate::{block::store::BlockStore, metrics, store::IndexerStore};
use actix_web::{get, http::header::ContentType, web::Data, HttpResponse};
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

#[get("/metrics")]
pub async fn get_metrics(store: Data<Arc<IndexerStore>>) -> HttpResponse {
    // Refresh scrape-time gauges from the store (same source as /health) so a
    // scrape reflects current tip state even between block-ingest events.
    if let Ok(Some(best_tip)) = store.as_ref().get_best_block() {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or_default();
        let tip_age_seconds = now_ms.saturating_sub(best_tip.timestamp()) / 1000;
        let block_slot_secs = crate::constants::MAINNET_BLOCK_SLOT_TIME_MILLIS / 1000;

        metrics::BEST_TIP_HEIGHT.set(best_tip.blockchain_length() as i64);
        metrics::TIP_AGE_SECONDS.set(tip_age_seconds as i64);
        metrics::SYNCED.set((tip_age_seconds < 2 * block_slot_secs) as i64);
    }

    // Store size gauges (disk-exhaustion visibility). These are cheap speedb
    // property reads, refreshed on each scrape.
    let db = store.as_ref();
    metrics::DB_ESTIMATED_LIVE_DATA_BYTES.set(db.estimate_live_data_size() as i64);
    metrics::DB_SST_FILES_BYTES.set(db.total_sst_files_size() as i64);
    metrics::DB_ESTIMATED_NUM_KEYS.set(db.estimate_num_keys() as i64);

    HttpResponse::Ok()
        .content_type(ContentType::plaintext())
        .body(metrics::gather())
}
