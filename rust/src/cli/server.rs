//! Server commands

use super::{database::DatabaseArgs, LogLevelFilter};
use crate::{constants::*, fetcher::BlockSource};
use std::{path::PathBuf, str::FromStr};

#[derive(clap::Parser, Debug, Clone, Default)]
#[command(author, version, about, long_about = None)]
pub struct ServerArgs {
    #[clap(flatten)]
    pub db: DatabaseArgs,

    /// Web server hostname for REST and GraphQL
    #[arg(long, default_value = DEFAULT_WEB_HOSTNAME)]
    pub web_hostname: String,

    /// Web server port for REST and GraphQL
    #[arg(long, default_value_t = DEFAULT_WEB_PORT)]
    pub web_port: u16,

    /// Max GraphQL query nesting depth (DoS guard; `0` disables). Rejected at
    /// validation, before any resolver runs.
    #[arg(long, env = "MINA_GRAPHQL_MAX_DEPTH", default_value_t = DEFAULT_GRAPHQL_MAX_DEPTH)]
    pub graphql_max_depth: usize,

    /// Max GraphQL query structural complexity — total selected fields (DoS guard;
    /// `0` disables).
    #[arg(long, env = "MINA_GRAPHQL_MAX_COMPLEXITY", default_value_t = DEFAULT_GRAPHQL_MAX_COMPLEXITY)]
    pub graphql_max_complexity: usize,

    /// Max wall-clock seconds a single GraphQL query may run before it's aborted
    /// (DoS guard against slow-but-valid queries; `0` disables).
    #[arg(long, env = "MINA_GRAPHQL_TIMEOUT_SECS", default_value_t = DEFAULT_GRAPHQL_TIMEOUT_SECS)]
    pub graphql_timeout_secs: u64,

    /// Disable GraphQL introspection (recommended in production to hide the schema;
    /// also disables the GraphiQL explorer's schema view).
    #[arg(long, env = "MINA_GRAPHQL_DISABLE_INTROSPECTION", default_value_t = false)]
    pub graphql_disable_introspection: bool,

    /// Comma-separated list of origins allowed to make cross-origin (browser)
    /// requests, e.g. `https://minasearch.com,https://app.example.com`. When
    /// unset the server is wildcard-open (`Access-Control-Allow-Origin: *`) for
    /// backward compatibility; set this on any public/multi-tenant deployment.
    #[arg(long, env = "MINA_WEB_CORS_ALLOWED_ORIGINS", value_delimiter = ',')]
    pub web_cors_allowed_origins: Vec<String>,

    /// Seconds to wait for a client's request headers before dropping the
    /// connection (slowloris guard; `0` disables).
    #[arg(long, env = "MINA_WEB_REQUEST_TIMEOUT_SECS", default_value_t = DEFAULT_WEB_REQUEST_TIMEOUT_SECS)]
    pub web_request_timeout_secs: u64,

    /// Max accepted request body size in bytes (oversized-payload guard; `0`
    /// disables).
    #[arg(long, env = "MINA_WEB_MAX_BODY_BYTES", default_value_t = DEFAULT_WEB_MAX_BODY_BYTES)]
    pub web_max_body_bytes: usize,

    /// Sustained per-IP request rate (requests/second) for the app-level rate
    /// limiter. Off unless this and `--web-rate-limit-burst` are both > 0.
    #[arg(long, env = "MINA_WEB_RATE_LIMIT_PER_SECOND", default_value_t = DEFAULT_WEB_RATE_LIMIT_PER_SECOND)]
    pub web_rate_limit_per_second: u64,

    /// Per-IP burst allowance (requests) for the rate limiter.
    #[arg(long, env = "MINA_WEB_RATE_LIMIT_BURST", default_value_t = DEFAULT_WEB_RATE_LIMIT_BURST)]
    pub web_rate_limit_burst: u32,

    /// Start with data consistency checks
    #[arg(long, default_value_t = false)]
    pub self_check: bool,

    /// Fetch new & missing precomputed blocks into `--blocks-dir` from this
    /// source: `gs://<bucket>` (a public GCS bucket, e.g.
    /// `gs://mina_network_block_data`) or `file://<dir>`. Source objects must
    /// be named `<network>-<height>-<hash>.json`. Unset: no blocks are fetched.
    #[arg(long, value_parser = parse_block_source)]
    pub block_source: Option<BlockSource>,

    /// Seconds between block fetch cycles
    #[arg(long, default_value_t = DEFAULT_BLOCK_FETCH_INTERVAL)]
    pub block_fetch_interval: u64,

    /// Max parallel block downloads
    #[arg(long, default_value_t = DEFAULT_BLOCK_FETCH_WORKERS)]
    pub block_fetch_workers: usize,

    /// Path to a block-verification executable. When set, every live-ingested
    /// block is gated on it: the indexer runs `EXE <network> <block-file>` and
    /// only ingests the block if it exits 0. Enables a trustless setup where a
    /// SNARK-proof verifier (e.g. mina-verify-server) vouches for each block.
    #[arg(long)]
    pub verify_block_exe: Option<PathBuf>,

    /// Bound `blocks-dir` growth by deleting ingested block files older than the
    /// retention window. Keeps block files at height >= `best_tip - N`; older
    /// blocks already live in the speedb store and are never re-read. Disabled
    /// when unset (every fetched block is kept). Floored at the transition
    /// frontier depth (k = 290) so reconcile always has recent blocks.
    #[arg(long)]
    pub blocks_retention_length: Option<u32>,

    /// Restore the database from a periodic checkpoint dir (the one written via
    /// `MINA_CHECKPOINT_DIR`, containing `latest/`) before starting. Only seeds
    /// an empty/absent `--database-dir`; an already-populated dir is opened
    /// as-is (likely newer than the checkpoint) unless `--restore-force` is set.
    #[arg(long)]
    pub restore_from_checkpoint: Option<PathBuf>,

    /// With `--restore-from-checkpoint`, overwrite a non-empty `--database-dir`
    /// with the checkpoint. Use only when the live DB is corrupt/unwanted.
    #[arg(long, default_value_t = false)]
    pub restore_force: bool,

    /// Indexer process ID
    #[arg(last = true)]
    pub pid: Option<u32>,
}

fn parse_block_source(s: &str) -> Result<BlockSource, String> {
    s.parse().map_err(|e: anyhow::Error| e.to_string())
}

fn default_block_fetch_interval() -> u64 {
    DEFAULT_BLOCK_FETCH_INTERVAL
}

fn default_block_fetch_workers() -> usize {
    DEFAULT_BLOCK_FETCH_WORKERS
}

fn default_graphql_max_depth() -> usize {
    DEFAULT_GRAPHQL_MAX_DEPTH
}

fn default_graphql_max_complexity() -> usize {
    DEFAULT_GRAPHQL_MAX_COMPLEXITY
}

fn default_graphql_timeout_secs() -> u64 {
    DEFAULT_GRAPHQL_TIMEOUT_SECS
}

fn default_web_request_timeout_secs() -> u64 {
    DEFAULT_WEB_REQUEST_TIMEOUT_SECS
}

fn default_web_max_body_bytes() -> usize {
    DEFAULT_WEB_MAX_BODY_BYTES
}

fn default_web_rate_limit_per_second() -> u64 {
    DEFAULT_WEB_RATE_LIMIT_PER_SECOND
}

fn default_web_rate_limit_burst() -> u32 {
    DEFAULT_WEB_RATE_LIMIT_BURST
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ServerArgsJson {
    pub genesis_ledger: Option<String>,
    pub genesis_hash: String,
    pub genesis_constants: Option<String>,
    pub constraint_system_digests: Option<Vec<String>>,
    pub protocol_txn_version_digest: Option<String>,
    pub protocol_network_version_digest: Option<String>,
    pub blocks_dir: Option<String>,
    pub staking_ledgers_dir: Option<String>,
    pub database_dir: String,
    pub log_level: String,
    pub ledger_cadence: u32,
    pub reporting_freq: u32,
    pub prune_interval: u32,
    pub canonical_threshold: u32,
    pub canonical_update_threshold: u32,
    pub web_hostname: String,
    pub web_port: u16,
    #[serde(default = "default_graphql_max_depth")]
    pub graphql_max_depth: usize,
    #[serde(default = "default_graphql_max_complexity")]
    pub graphql_max_complexity: usize,
    #[serde(default = "default_graphql_timeout_secs")]
    pub graphql_timeout_secs: u64,
    #[serde(default)]
    pub graphql_disable_introspection: bool,
    #[serde(default)]
    pub web_cors_allowed_origins: Vec<String>,
    #[serde(default = "default_web_request_timeout_secs")]
    pub web_request_timeout_secs: u64,
    #[serde(default = "default_web_max_body_bytes")]
    pub web_max_body_bytes: usize,
    #[serde(default = "default_web_rate_limit_per_second")]
    pub web_rate_limit_per_second: u64,
    #[serde(default = "default_web_rate_limit_burst")]
    pub web_rate_limit_burst: u32,
    pub pid: Option<u32>,
    pub do_not_ingest_orphan_blocks: bool,
    pub block_source: Option<String>,
    #[serde(default = "default_block_fetch_interval")]
    pub block_fetch_interval: u64,
    #[serde(default = "default_block_fetch_workers")]
    pub block_fetch_workers: usize,
    pub verify_block_exe: Option<String>,
    pub blocks_retention_length: Option<u32>,
    pub network: String,
    pub check_mode: bool,
}

//////////
// impl //
//////////

impl ServerArgs {
    pub fn with_dynamic_defaults(mut self, pid: u32) -> Self {
        self.pid = Some(pid);
        self
    }

    /// Reject conflicting / silently-ineffective flag combinations at startup so
    /// an operator finds out immediately, not after a "why isn't this working"
    /// incident. Returns non-fatal `warnings` on success; `Err` on a hard
    /// conflict that should abort startup.
    pub fn validate(&self) -> anyhow::Result<Vec<String>> {
        let mut warnings = Vec::new();

        // Hard: the rate limiter is only active when BOTH knobs are > 0. Setting
        // just one leaves it silently OFF — the operator thinks the deployment is
        // protected when it isn't.
        let rps = self.web_rate_limit_per_second;
        let burst = self.web_rate_limit_burst;
        if (rps > 0) != (burst > 0) {
            anyhow::bail!(
                "rate limiting needs BOTH --web-rate-limit-per-second and \
                 --web-rate-limit-burst > 0 (got per-second={rps}, burst={burst}); \
                 set both to enable it, or neither to disable it"
            );
        }

        // Hard: --restore-force only means anything alongside --restore-from-checkpoint.
        if self.restore_force && self.restore_from_checkpoint.is_none() {
            anyhow::bail!(
                "--restore-force has no effect without --restore-from-checkpoint"
            );
        }

        // Hard: the block fetcher downloads into --blocks-dir, and needs a
        // non-zero interval and worker count to make progress.
        if let Some(source) = &self.block_source {
            if self.db.blocks_dir.is_none() {
                anyhow::bail!("--block-source {source} needs --blocks-dir to fetch into");
            }
            if self.block_fetch_interval == 0 || self.block_fetch_workers == 0 {
                anyhow::bail!("--block-fetch-interval and --block-fetch-workers must be > 0");
            }
        }

        // Soft: port 0 binds an OS-assigned ephemeral port clients can't predict.
        if self.web_port == 0 {
            warnings.push(
                "--web-port 0 binds an OS-assigned ephemeral port; clients cannot \
                 predict it"
                    .to_string(),
            );
        }

        Ok(warnings)
    }
}

/////////////////
// conversions //
/////////////////

impl From<ServerArgs> for ServerArgsJson {
    fn from(value: ServerArgs) -> Self {
        let pid = value.pid.unwrap();
        let value = value.with_dynamic_defaults(pid);
        Self {
            genesis_ledger: value
                .db
                .genesis_ledger
                .map(|path| path.display().to_string()),
            genesis_hash: value.db.genesis_hash,
            genesis_constants: value.db.genesis_constants.map(|g| g.display().to_string()),
            constraint_system_digests: value.db.constraint_system_digests,
            protocol_txn_version_digest: value.db.protocol_txn_version_digest,
            protocol_network_version_digest: value.db.protocol_network_version_digest,
            blocks_dir: value.db.blocks_dir.map(|d| d.display().to_string()),
            staking_ledgers_dir: value
                .db
                .staking_ledgers_dir
                .map(|d| d.display().to_string()),
            database_dir: value.db.database_dir.display().to_string(),
            log_level: value.db.log_level.to_string(),
            ledger_cadence: value.db.ledger_cadence,
            reporting_freq: value.db.reporting_freq,
            prune_interval: value.db.prune_interval,
            canonical_threshold: value.db.canonical_threshold,
            canonical_update_threshold: value.db.canonical_update_threshold,
            web_hostname: value.web_hostname,
            web_port: value.web_port,
            graphql_max_depth: value.graphql_max_depth,
            graphql_max_complexity: value.graphql_max_complexity,
            graphql_timeout_secs: value.graphql_timeout_secs,
            graphql_disable_introspection: value.graphql_disable_introspection,
            web_cors_allowed_origins: value.web_cors_allowed_origins,
            web_request_timeout_secs: value.web_request_timeout_secs,
            web_max_body_bytes: value.web_max_body_bytes,
            web_rate_limit_per_second: value.web_rate_limit_per_second,
            web_rate_limit_burst: value.web_rate_limit_burst,
            pid: value.pid,
            block_source: value.block_source.map(|s| s.to_string()),
            block_fetch_interval: value.block_fetch_interval,
            block_fetch_workers: value.block_fetch_workers,
            verify_block_exe: value.verify_block_exe.map(|p| p.display().to_string()),
            blocks_retention_length: value.blocks_retention_length,
            network: value.db.network.to_string(),
            do_not_ingest_orphan_blocks: value.db.do_not_ingest_orphan_blocks,
            check_mode: value.db.check_mode,
        }
    }
}

impl From<ServerArgsJson> for ServerArgs {
    fn from(value: ServerArgsJson) -> Self {
        let db = DatabaseArgs {
            genesis_ledger: value.genesis_ledger.and_then(|path| path.parse().ok()),
            genesis_hash: value.genesis_hash,
            genesis_constants: value.genesis_constants.map(Into::into),
            protocol_txn_version_digest: value.protocol_txn_version_digest,
            protocol_network_version_digest: value.protocol_network_version_digest,
            constraint_system_digests: value.constraint_system_digests,
            blocks_dir: value.blocks_dir.map(Into::into),
            staking_ledgers_dir: value.staking_ledgers_dir.map(Into::into),
            database_dir: value.database_dir.into(),
            log_level: LogLevelFilter::from_str(&value.log_level).expect("log level"),
            ledger_cadence: value.ledger_cadence,
            reporting_freq: value.reporting_freq,
            prune_interval: value.prune_interval,
            canonical_threshold: value.canonical_threshold,
            canonical_update_threshold: value.canonical_update_threshold,
            config: None,
            network: (&value.network as &str).into(),
            do_not_ingest_orphan_blocks: value.do_not_ingest_orphan_blocks,
            check_mode: value.check_mode,
        };

        Self {
            db,
            web_hostname: value.web_hostname,
            web_port: value.web_port,
            graphql_max_depth: value.graphql_max_depth,
            graphql_max_complexity: value.graphql_max_complexity,
            graphql_timeout_secs: value.graphql_timeout_secs,
            graphql_disable_introspection: value.graphql_disable_introspection,
            web_cors_allowed_origins: value.web_cors_allowed_origins,
            web_request_timeout_secs: value.web_request_timeout_secs,
            web_max_body_bytes: value.web_max_body_bytes,
            web_rate_limit_per_second: value.web_rate_limit_per_second,
            web_rate_limit_burst: value.web_rate_limit_burst,
            self_check: false,
            pid: value.pid,
            block_source: value.block_source.and_then(|s| s.parse().ok()),
            block_fetch_interval: value.block_fetch_interval,
            block_fetch_workers: value.block_fetch_workers,
            verify_block_exe: value.verify_block_exe.map(Into::into),
            blocks_retention_length: value.blocks_retention_length,
            restore_from_checkpoint: None,
            restore_force: false,
        }
    }
}

impl From<DatabaseArgs> for ServerArgs {
    fn from(value: DatabaseArgs) -> Self {
        Self {
            db: value,
            web_hostname: DEFAULT_WEB_HOSTNAME.to_string(),
            web_port: DEFAULT_WEB_PORT,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ServerArgs;
    use crate::constants::DEFAULT_WEB_PORT;

    fn base() -> ServerArgs {
        // A minimal, valid config (a real port; everything else off/unset).
        ServerArgs {
            web_port: DEFAULT_WEB_PORT,
            ..Default::default()
        }
    }

    #[test]
    fn valid_config_has_no_warnings() {
        let w = base().validate().expect("valid config");
        assert!(w.is_empty(), "unexpected warnings: {w:?}");
    }

    #[test]
    fn half_configured_rate_limiter_is_rejected() {
        // per-second without burst
        let a = ServerArgs {
            web_rate_limit_per_second: 100,
            web_rate_limit_burst: 0,
            ..base()
        };
        assert!(a.validate().is_err());

        // burst without per-second
        let a = ServerArgs {
            web_rate_limit_per_second: 0,
            web_rate_limit_burst: 5,
            ..base()
        };
        assert!(a.validate().is_err());

        // both set is fine
        let a = ServerArgs {
            web_rate_limit_per_second: 100,
            web_rate_limit_burst: 5,
            ..base()
        };
        assert!(a.validate().unwrap().is_empty());
    }

    #[test]
    fn restore_force_without_checkpoint_is_rejected() {
        let a = ServerArgs {
            restore_force: true,
            restore_from_checkpoint: None,
            ..base()
        };
        assert!(a.validate().is_err());

        let a = ServerArgs {
            restore_force: true,
            restore_from_checkpoint: Some("/data/checkpoints".into()),
            ..base()
        };
        assert!(a.validate().unwrap().is_empty());
    }

    #[test]
    fn block_source_needs_blocks_dir_and_progress() {
        let source = Some("gs://mina_network_block_data".parse().unwrap());
        let a = ServerArgs {
            block_source: source.clone(),
            block_fetch_interval: 60,
            block_fetch_workers: 16,
            ..base()
        };
        assert!(a.validate().is_err(), "no --blocks-dir");

        let mut a = a;
        a.db.blocks_dir = Some("/data/blocks".into());
        assert!(a.validate().unwrap().is_empty());

        let b = ServerArgs {
            block_fetch_interval: 0,
            ..a.clone()
        };
        assert!(b.validate().is_err(), "zero interval");
    }

    #[test]
    fn web_port_zero_warns() {
        let a = ServerArgs {
            web_port: 0,
            ..base()
        };
        let w = a.validate().unwrap();
        assert!(w.iter().any(|m| m.contains("--web-port 0")));
    }
}
