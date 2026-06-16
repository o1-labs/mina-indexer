//! Prometheus metrics for the indexer.
//!
//! Each metric registers into the [`prometheus`] default registry on first use;
//! [`init`] forces them all to register at startup so they appear in `/metrics`
//! even before their first event. The HTTP `/metrics` endpoint
//! (see `web::rest::metrics`) sets the scrape-time gauges and encodes the
//! registry.

use prometheus::{
    register_histogram, register_int_counter, register_int_gauge, Encoder, Histogram, IntCounter,
    IntGauge, TextEncoder,
};
use std::sync::LazyLock;

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
