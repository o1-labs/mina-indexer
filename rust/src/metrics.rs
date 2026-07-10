//! Prometheus metrics for the indexer.
//!
//! Each metric registers into the [`prometheus`] default registry on first use;
//! [`init`] forces them all to register at startup so they appear in `/metrics`
//! even before their first event. The HTTP `/metrics` endpoint
//! (see `web::rest::metrics`) sets the scrape-time gauges and encodes the
//! registry.

use prometheus::{
    register_histogram, register_histogram_vec, register_int_counter, register_int_gauge, Encoder,
    Histogram, HistogramVec, IntCounter, IntGauge, TextEncoder,
};
use std::sync::LazyLock;

/// Per-request HTTP latency (seconds), labelled by matched route, method, and
/// response status. Also serves as the request/error counter (its `_count`
/// series, sliced by `status`). Labelled by the route *pattern* (e.g.
/// `/accounts/{public_key}`), never the raw path, to keep cardinality bounded.
pub static HTTP_REQUEST_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "mina_indexer_http_request_duration_seconds",
        "HTTP request duration in seconds",
        &["endpoint", "method", "status"],
        vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]
    )
    .unwrap()
});

/// Total precomputed blocks applied to the witness tree.
pub static BLOCKS_PROCESSED: LazyLock<IntCounter> = LazyLock::new(|| {
    register_int_counter!(
        "mina_indexer_blocks_processed_total",
        "Total precomputed blocks applied to the witness tree"
    )
    .unwrap()
});

/// Per-block witness-tree apply latency (surfaces the O(tree) ingest slowdown).
pub static BLOCK_INGEST_SECONDS: LazyLock<Histogram> = LazyLock::new(|| {
    register_histogram!(
        "mina_indexer_block_ingest_seconds",
        "Time to apply a single block to the witness tree",
        vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]
    )
    .unwrap()
});

/// Best-tip blockchain length currently in the witness tree.
pub static BEST_TIP_HEIGHT: LazyLock<IntGauge> = LazyLock::new(|| {
    register_int_gauge!(
        "mina_indexer_best_tip_height",
        "Blockchain length of the current best tip"
    )
    .unwrap()
});

/// Age of the best tip in seconds (now - tip timestamp).
pub static TIP_AGE_SECONDS: LazyLock<IntGauge> = LazyLock::new(|| {
    register_int_gauge!(
        "mina_indexer_tip_age_seconds",
        "Seconds since the best tip's block timestamp"
    )
    .unwrap()
});

/// 1 if the tip is within ~2 slots of now, else 0.
pub static SYNCED: LazyLock<IntGauge> = LazyLock::new(|| {
    register_int_gauge!("mina_indexer_synced", "1 if the tip is recent, else 0").unwrap()
});

/// Number of dangling (disconnected) branches in the witness tree.
pub static DANGLING_BRANCHES: LazyLock<IntGauge> = LazyLock::new(|| {
    register_int_gauge!(
        "mina_indexer_dangling_branches",
        "Number of dangling branches in the witness tree"
    )
    .unwrap()
});

/// Wall time of each fetch-new-blocks invocation (surfaces a slow/blocking
/// fetcher).
pub static FETCH_SECONDS: LazyLock<Histogram> = LazyLock::new(|| {
    register_histogram!(
        "mina_indexer_fetch_seconds",
        "Wall time of a fetch-new-blocks invocation",
        vec![0.1, 0.5, 1.0, 5.0, 15.0, 30.0, 60.0, 120.0, 300.0]
    )
    .unwrap()
});

/// Total fetch-new-blocks invocations.
pub static FETCH_INVOCATIONS: LazyLock<IntCounter> = LazyLock::new(|| {
    register_int_counter!(
        "mina_indexer_fetch_invocations_total",
        "Total fetch-new-blocks invocations"
    )
    .unwrap()
});

/// Total on-disk blocks ingested by the reconcile safety net.
pub static RECONCILE_INGESTED: LazyLock<IntCounter> = LazyLock::new(|| {
    register_int_counter!(
        "mina_indexer_reconcile_ingested_total",
        "Total blocks ingested by reconcile_blocks_dir"
    )
    .unwrap()
});

/// Total ingested block files deleted from `blocks_dir` by retention pruning.
pub static BLOCKS_PRUNED: LazyLock<IntCounter> = LazyLock::new(|| {
    register_int_counter!(
        "mina_indexer_blocks_pruned_total",
        "Total ingested block files deleted from blocks_dir by retention pruning"
    )
    .unwrap()
});

/// Total blocks that parsed but failed to apply to the witness tree
/// (`block_pipeline` errored), on either the watcher or reconcile path. A
/// rising rate here means valid-looking blocks are being dropped — an ingest
/// stall the block counters alone don't surface.
pub static BLOCKS_INGEST_FAILED: LazyLock<IntCounter> = LazyLock::new(|| {
    register_int_counter!(
        "mina_indexer_blocks_ingest_failed_total",
        "Total blocks that failed to apply to the witness tree"
    )
    .unwrap()
});

/// Total fetch-new-blocks invocations that failed to run (the external fetcher
/// could not be spawned). Pairs with `FETCH_INVOCATIONS` to give a fetch error
/// rate; a stuck fetcher otherwise looks identical to "no new blocks".
pub static FETCH_FAILURES: LazyLock<IntCounter> = LazyLock::new(|| {
    register_int_counter!(
        "mina_indexer_fetch_failures_total",
        "Total fetch-new-blocks invocations that failed to run"
    )
    .unwrap()
});

/// Estimated live (logical) data size of the speedb store in bytes. Scrape-time
/// gauge (set by the `/metrics` handler from the store's property estimate).
pub static DB_ESTIMATED_LIVE_DATA_BYTES: LazyLock<IntGauge> = LazyLock::new(|| {
    register_int_gauge!(
        "mina_indexer_db_estimated_live_data_bytes",
        "Estimated live data size of the speedb store in bytes"
    )
    .unwrap()
});

/// Total on-disk size of the store's SST files in bytes — the real disk
/// footprint. Scrape-time gauge; the signal to watch for disk exhaustion.
pub static DB_SST_FILES_BYTES: LazyLock<IntGauge> = LazyLock::new(|| {
    register_int_gauge!(
        "mina_indexer_db_sst_files_bytes",
        "Total on-disk size of the speedb store's SST files in bytes"
    )
    .unwrap()
});

/// Estimated number of keys across the whole store. Scrape-time gauge; a
/// coarse growth/size proxy.
pub static DB_ESTIMATED_NUM_KEYS: LazyLock<IntGauge> = LazyLock::new(|| {
    register_int_gauge!(
        "mina_indexer_db_estimated_num_keys",
        "Estimated number of keys in the speedb store"
    )
    .unwrap()
});

/// Force every metric to register so it appears in `/metrics` before its first
/// event.
pub fn init() {
    LazyLock::force(&BLOCKS_PROCESSED);
    LazyLock::force(&BLOCK_INGEST_SECONDS);
    LazyLock::force(&BEST_TIP_HEIGHT);
    LazyLock::force(&TIP_AGE_SECONDS);
    LazyLock::force(&SYNCED);
    LazyLock::force(&DANGLING_BRANCHES);
    LazyLock::force(&FETCH_SECONDS);
    LazyLock::force(&FETCH_INVOCATIONS);
    LazyLock::force(&RECONCILE_INGESTED);
    LazyLock::force(&BLOCKS_PRUNED);
    LazyLock::force(&HTTP_REQUEST_DURATION);
    LazyLock::force(&BLOCKS_INGEST_FAILED);
    LazyLock::force(&FETCH_FAILURES);
    LazyLock::force(&DB_ESTIMATED_LIVE_DATA_BYTES);
    LazyLock::force(&DB_SST_FILES_BYTES);
    LazyLock::force(&DB_ESTIMATED_NUM_KEYS);
}

/// Encode the default registry in Prometheus text exposition format.
pub fn gather() -> String {
    let mut buf = Vec::new();
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    if encoder.encode(&metric_families, &mut buf).is_err() {
        return String::new();
    }
    String::from_utf8(buf).unwrap_or_default()
}
