//! Server commands

use super::{database::DatabaseArgs, LogLevelFilter};
use crate::constants::*;
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

    /// Start with data consistency checks
    #[arg(long, default_value_t = false)]
    pub self_check: bool,

    /// Path to the fetch new blocks executable
    #[arg(long)]
    pub fetch_new_blocks_exe: Option<PathBuf>,

    /// Delay (sec) in between fetch new blocks attempts
    #[arg(long)]
    pub fetch_new_blocks_delay: Option<u64>,

    /// Path to a block-verification executable. When set, every live-ingested
    /// block is gated on it: the indexer runs `EXE <network> <block-file>` and
    /// only ingests the block if it exits 0. Enables a trustless setup where a
    /// SNARK-proof verifier (e.g. mina-verify-server) vouches for each block.
    #[arg(long)]
    pub verify_block_exe: Option<PathBuf>,

    /// Path to the missing block recovery executable
    #[arg(long)]
    pub missing_block_recovery_exe: Option<PathBuf>,

    /// Delay (sec) in between missing block recovery attempts
    #[arg(long)]
    pub missing_block_recovery_delay: Option<u64>,

    /// Recover all blocks at all missing heights
    #[arg(long)]
    pub missing_block_recovery_batch: Option<bool>,

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

fn default_graphql_max_depth() -> usize {
    DEFAULT_GRAPHQL_MAX_DEPTH
}

fn default_graphql_max_complexity() -> usize {
    DEFAULT_GRAPHQL_MAX_COMPLEXITY
}

fn default_graphql_timeout_secs() -> u64 {
    DEFAULT_GRAPHQL_TIMEOUT_SECS
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
    pub pid: Option<u32>,
    pub do_not_ingest_orphan_blocks: bool,
    pub fetch_new_blocks_exe: Option<String>,
    pub fetch_new_blocks_delay: Option<u64>,
    pub verify_block_exe: Option<String>,
    pub missing_block_recovery_exe: Option<String>,
    pub missing_block_recovery_delay: Option<u64>,
    pub missing_block_recovery_batch: Option<bool>,
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
            pid: value.pid,
            fetch_new_blocks_delay: value.fetch_new_blocks_delay,
            fetch_new_blocks_exe: value.fetch_new_blocks_exe.map(|p| p.display().to_string()),
            verify_block_exe: value.verify_block_exe.map(|p| p.display().to_string()),
            missing_block_recovery_delay: value.missing_block_recovery_delay,
            missing_block_recovery_exe: value
                .missing_block_recovery_exe
                .map(|p| p.display().to_string()),
            missing_block_recovery_batch: value.missing_block_recovery_batch,
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
            self_check: false,
            pid: value.pid,
            fetch_new_blocks_delay: value.fetch_new_blocks_delay,
            fetch_new_blocks_exe: value.fetch_new_blocks_exe.map(Into::into),
            verify_block_exe: value.verify_block_exe.map(Into::into),
            missing_block_recovery_delay: value.missing_block_recovery_delay,
            missing_block_recovery_exe: value.missing_block_recovery_exe.map(Into::into),
            missing_block_recovery_batch: value.missing_block_recovery_batch,
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
