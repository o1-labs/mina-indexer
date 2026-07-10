//! Logging setup: `tracing-subscriber` with a human-readable default and an
//! opt-in structured JSON mode for log aggregation (Loki/ELK/Datadog/etc.).
//!
//! - Filtering: honours `RUST_LOG` (standard `tracing`/`env_logger` syntax). When
//!   unset, defaults to the CLI `--log-level` scoped to this crate, with deps at `warn`.
//! - Format: `MINA_LOG_FORMAT=json` emits one JSON object per line (timestamp,
//!   level, target, message, fields); anything else (default) is human-readable.
//! - The `tracing-log` bridge captures the crate's existing `log::*` calls, so no
//!   call sites change.

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn level_str(level: log::LevelFilter) -> &'static str {
    match level {
        log::LevelFilter::Off => "off",
        log::LevelFilter::Error => "error",
        log::LevelFilter::Warn => "warn",
        log::LevelFilter::Info => "info",
        log::LevelFilter::Debug => "debug",
        log::LevelFilter::Trace => "trace",
    }
}

/// Install the global tracing subscriber. Idempotent (safe to call once per
/// process; later calls are no-ops). `default_level` is used when `RUST_LOG` is unset.
pub fn init(default_level: log::LevelFilter) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        // crate at the requested level, dependencies quiet (warn) — matches the
        // previous `module(module_path!())` scoping while staying RUST_LOG-overridable.
        EnvFilter::new(format!("warn,mina_indexer={}", level_str(default_level)))
    });

    let json = std::env::var("MINA_LOG_FORMAT")
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    let registry = tracing_subscriber::registry().with(filter);
    if json {
        let _ = registry
            .with(
                fmt::layer()
                    .json()
                    // Hoist event fields (height, state_hash, duration_ms, …) to
                    // top-level JSON keys so Loki/Datadog can filter on them
                    // directly instead of digging into a nested `fields` object.
                    .flatten_event(true)
                    .with_current_span(false)
                    .with_span_list(false),
            )
            .try_init();
    } else {
        let _ = registry.with(fmt::layer().with_ansi(false)).try_init();
    }
}
