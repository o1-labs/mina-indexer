use clap::{Parser, Subcommand};
use log::{debug, error, info, warn, LevelFilter};
use mina_indexer::{
    base::state_hash::StateHash,
    block::precomputed::{CurrencyEncoding, PcbVersion},
    block::store::BlockStore,
    canonicity::store::CanonicityStore,
    chain::ChainId,
    cli::{
        database::DatabaseArgs,
        server::{ServerArgs, ServerArgsJson},
        LogLevelFilter,
    },
    client,
    constants::*,
    ledger::genesis::GenesisLedger,
    server::{GenesisVersion, IndexerConfiguration, IndexerVersion, InitializationMode},
    store::{
        restore_snapshot,
        version::{IndexerStoreVersion, VersionStore},
        IndexerStore,
    },
    unix_socket_server::remove_unix_socket,
    web::start_web_server,
};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process,
    sync::Arc,
    time::Duration,
};
use tempfile::TempDir;
use tokio_graceful_shutdown::{
    errors::SubsystemError, SubsystemBuilder, SubsystemHandle, Toplevel,
};

#[derive(Parser, Debug)]
#[command(name = "mina-indexer", author, version = VERSION, about, long_about = Some("Mina Indexer\n\n\
Efficiently index and query the Mina blockchain"))]
struct Cli {
    #[command(subcommand)]
    command: IndexerCommand,

    /// Path to the Unix domain socket file
    #[arg(long, default_value = "./mina-indexer.sock", num_args = 1)]
    socket: PathBuf,
}

#[derive(Subcommand, Debug)]
#[allow(clippy::large_enum_variant)]
enum IndexerCommand {
    /// Database commands
    Database {
        #[command(subcommand)]
        db_command: DatabaseCommand,
    },

    /// Server commands
    Server {
        #[command(subcommand)]
        server_command: ServerCommand,
    },

    /// Client commands
    #[clap(flatten)]
    Client(#[command(subcommand)] client::ClientCli),

    /// Mina indexer version
    Version,
}

#[derive(Subcommand, Debug)]
enum ServerCommand {
    /// Start a new mina indexer
    Start(Box<ServerArgs>),

    /// Shutdown the server
    Shutdown,
}

#[derive(Subcommand, Debug)]
enum DatabaseCommand {
    /// Create a new mina indexer database to use with `mina-indexer start`
    Create(Box<DatabaseArgs>),

    /// Create a snapshot of a mina indexer database
    Snapshot {
        /// Full path to the snapshot file to be created
        #[arg(long, default_value = "./snapshot")]
        output_path: PathBuf,

        /// Full path to a mina indexer database directory.
        /// If null, snapshot a running indexer database.
        #[arg(long)]
        database_dir: Option<PathBuf>,
    },

    /// Restore an indexer database from an archived snapshot file
    Restore {
        /// Full path to the archive snapshot file
        #[arg(long, default_value = "./snapshot")]
        snapshot_file: PathBuf,

        /// Full path to the database directory
        #[arg(long)]
        restore_dir: PathBuf,
    },

    /// Query mina indexer database version
    Version {
        /// Output JSON data
        #[arg(long)]
        json: bool,
    },

    /// Check a database for silent corruption: store version, best tip, and the
    /// canonical chain's contiguity / block-presence / parent linkage. Opens the
    /// database read-only (safe to run against a live indexer). Exits non-zero if
    /// any problem is found.
    VerifyIntegrity {
        /// Full path to a mina indexer database directory
        #[arg(long)]
        database_dir: PathBuf,

        /// Output JSON data
        #[arg(long)]
        json: bool,
    },
}

impl From<&DatabaseCommand> for LevelFilter {
    fn from(value: &DatabaseCommand) -> Self {
        if let DatabaseCommand::Create(args) = value {
            args.log_level.clone().0
        } else {
            LogLevelFilter::default().0
        }
    }
}

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    let domain_socket_path = args.socket;

    let result = Toplevel::new(|s| async move {
        s.start(SubsystemBuilder::new("Main", |s| async move {
            match args.command {
                IndexerCommand::Client(cli) => cli.run(domain_socket_path).await,
                IndexerCommand::Database { db_command } => db_command.run(domain_socket_path).await,
                IndexerCommand::Server { server_command } => {
                    server_command.run(s, domain_socket_path).await
                }
                IndexerCommand::Version => {
                    println!("{VERSION}");
                    Ok(())
                }
            }
        }));
    })
    .catch_signals()
    .handle_shutdown_requests(Duration::from_millis(1000))
    .await;

    match result {
        Ok(_) => Ok(()),
        Err(shutdown_error) => {
            // Extract and log the specific error details
            let subsystem_errors = shutdown_error.get_subsystem_errors();

            if !subsystem_errors.is_empty() {
                let first_error = subsystem_errors.iter().next().unwrap();

                match first_error {
                    SubsystemError::Failed(_, error) => Err(anyhow::anyhow!("{}", error)),
                    SubsystemError::Panicked(name) => {
                        Err(anyhow::anyhow!("Subsystem '{}' panicked", name))
                    }
                }
            } else {
                Err(anyhow::anyhow!("{}", shutdown_error))
            }
        }
    }
}

impl ServerCommand {
    async fn run(self, subsys: SubsystemHandle, domain_socket_path: PathBuf) -> anyhow::Result<()> {
        let (args, mode) = match self {
            Self::Shutdown => return client::ClientCli::Shutdown.run(domain_socket_path).await,
            Self::Start(args) => {
                // bring logging up before the (rare, operator-triggered) restore
                // so its progress is visible; the later init() call is a no-op.
                mina_indexer::logging::init(args.db.log_level.0);
                // reject conflicting flags up front; surface ineffective ones.
                for w in args.validate()? {
                    warn!("config: {w}");
                }
                // disaster recovery: optionally seed the database dir from a
                // periodic checkpoint before we decide how to initialize, so a
                // restored DB is then opened in Sync mode (CURRENT present).
                maybe_restore_from_checkpoint(
                    args.restore_from_checkpoint.as_deref(),
                    &args.db.database_dir,
                    args.restore_force,
                )?;
                if let Some(config_path) = args.db.config {
                    let contents = std::fs::read(config_path)?;
                    let args: ServerArgsJson = serde_json::from_slice(&contents)?;
                    (args.into(), InitializationMode::Sync)
                } else if args.self_check {
                    (*args, InitializationMode::Replay)
                } else {
                    // Self-initialize: if there's no database yet, build it from
                    // blocks; otherwise just open and sync. Lets `server start`
                    // run standalone without a separate `database create` first.
                    let mode = if args.db.database_dir.join("CURRENT").exists() {
                        InitializationMode::Sync
                    } else {
                        InitializationMode::BuildDB
                    };
                    (*args, mode)
                }
            }
        };
        let args = args.with_dynamic_defaults(std::process::id());
        let database_dir = args.db.database_dir.clone();
        let web_hostname = args.web_hostname.clone();
        let web_port = args.web_port;
        let graphql_max_depth = args.graphql_max_depth;
        let graphql_max_complexity = args.graphql_max_complexity;
        let graphql_timeout_secs = args.graphql_timeout_secs;
        let graphql_disable_introspection = args.graphql_disable_introspection;
        let web_cors_allowed_origins = args.web_cors_allowed_origins.clone();
        let web_request_timeout_secs = args.web_request_timeout_secs;
        let web_max_body_bytes = args.web_max_body_bytes;
        let web_rate_limit_per_second = args.web_rate_limit_per_second;
        let web_rate_limit_burst = args.web_rate_limit_burst;

        // initialize logging (human-readable by default; MINA_LOG_FORMAT=json for structured)
        mina_indexer::logging::init(args.db.log_level.0);

        check_or_write_pid_file(&database_dir);

        let config = process_indexer_configuration(args, mode, domain_socket_path.clone())?;

        info!("Starting the mina indexer filesystem watchers & UDS server");
        let db = Arc::new(IndexerStore::new(&database_dir, false)?);
        let store = db.clone();

        subsys.start(SubsystemBuilder::new("Indexer", move |s| {
            config.start_indexer(s, store)
        }));

        info!("Starting the web server listening on {web_hostname}:{web_port}");
        let store = db.clone();
        let host = web_hostname.clone();

        subsys.start(SubsystemBuilder::new("Web Server", move |s| {
            start_web_server(
                s,
                store,
                (host, web_port),
                mina_indexer::web::WebServerConfig {
                    graphql_max_depth,
                    graphql_max_complexity,
                    graphql_timeout_secs,
                    graphql_disable_introspection,
                    cors_allowed_origins: web_cors_allowed_origins,
                    request_timeout_secs: web_request_timeout_secs,
                    max_body_bytes: web_max_body_bytes,
                    rate_limit_per_second: web_rate_limit_per_second,
                    rate_limit_burst: web_rate_limit_burst,
                },
            )
        }));

        info!("GraphQL server started at: http://{web_hostname}:{web_port}/graphql");
        subsys.on_shutdown_requested().await;

        debug!("Shutting down primary database instance");
        // Flush memtables to SST + sync the WAL so the next open doesn't replay a
        // large WAL. Without this, an abrupt stop makes the next `server start`
        // spend minutes recovering before it can serve.
        let _ = db.database.flush();
        let _ = db.database.flush_wal(true);
        db.database.cancel_all_background_work(true);

        remove_pid(&database_dir);
        drop(db);
        remove_unix_socket(&domain_socket_path)?;

        Ok(())
    }
}

impl DatabaseCommand {
    async fn run(self, domain_socket_path: PathBuf) -> anyhow::Result<()> {
        // initialize logging (human-readable by default; MINA_LOG_FORMAT=json for structured)
        mina_indexer::logging::init(LevelFilter::from(&self));

        match self {
            Self::Version { json } => {
                let version = IndexerStoreVersion::default();
                println!(
                    "{}",
                    if json {
                        serde_json::to_string(&version)?
                    } else {
                        version.to_string()
                    }
                )
            }
            Self::VerifyIntegrity { database_dir, json } => {
                // Open read-only (secondary) so this is safe to run against a
                // live indexer and never mutates the database.
                let tmp_dir = TempDir::new()?;
                let db = IndexerStore::read_only(&database_dir, tmp_dir.as_ref())?;
                let report = verify_database_integrity(&db)?;

                if json {
                    println!("{}", serde_json::to_string(&report)?);
                } else {
                    print!("{report}");
                }
                if !report.ok {
                    process::exit(2);
                }
            }
            Self::Snapshot {
                output_path,
                database_dir,
            } => {
                if let Some(database_dir) = database_dir {
                    if !database_dir.exists() {
                        error!("Database dir {database_dir:#?} does not exist");
                    } else {
                        info!("Creating snapshot of database dir {database_dir:#?}");
                        let tmp_dir = TempDir::new()?;
                        let db = IndexerStore::read_only(&database_dir, tmp_dir.as_ref())?;
                        db.create_snapshot(&output_path)?;
                    }
                } else {
                    info!("Creating snapshot of running mina indexer");
                    return client::ClientCli::CreateSnapshot { output_path }
                        .run(domain_socket_path)
                        .await;
                }
            }
            Self::Restore {
                snapshot_file,
                restore_dir,
            } => {
                info!("Restoring mina indexer database from snapshot file {snapshot_file:#?} to {restore_dir:#?}");
                restore_snapshot(&snapshot_file, &restore_dir)?
            }
            Self::Create(args) => {
                let database_dir = args.database_dir.clone();
                debug!("Ensuring mina indexer database exists in {database_dir:#?}");

                if let Err(e) = fs::create_dir_all(&database_dir) {
                    error!("Failed to create database directory: {e}");
                    process::exit(1);
                }

                debug!("Building mina indexer configuration");
                let mut mode = InitializationMode::BuildDB;

                if let Ok(dir) = std::fs::read_dir(database_dir.clone()) {
                    if dir.count() > 0 {
                        mode = InitializationMode::Sync;
                    }
                };

                let config = if let Some(config_path) = args.config {
                    let contents = std::fs::read(config_path)?;
                    let args: ServerArgsJson = serde_json::from_slice(&contents)?;
                    IndexerConfiguration::from((args, domain_socket_path))
                } else {
                    process_indexer_configuration((*args).into(), mode, domain_socket_path)?
                };
                let db = Arc::new(IndexerStore::new(&database_dir, true)?);
                let store = db.clone();

                tokio::select! {
                    // wait for SIGINT
                    _ = tokio::signal::ctrl_c() => {
                        info!("SIGINT received");
                        let _ = store.database.flush();
                        let _ = store.database.flush_wal(true);
                        store.database.cancel_all_background_work(true);
                    }

                    // build the database
                    res = config.initialize_indexer_database(&store) => {
                        if let Err(e) = res {
                            error!("Failed to initialize indexer database: {e}");
                        };
                    }
                }
            }
        }
        Ok(())
    }
}

/// Creates directories, processes constants & parses genesis ledger.
/// Returns indexer config.
fn process_indexer_configuration(
    args: ServerArgs,
    initialization_mode: InitializationMode,
    domain_socket_path: PathBuf,
) -> anyhow::Result<IndexerConfiguration> {
    let genesis_hash = args.db.genesis_hash;
    let blocks_dir = args.db.blocks_dir;
    let staking_ledgers_dir = args.db.staking_ledgers_dir;
    let prune_interval = args.db.prune_interval;
    let canonical_threshold = args.db.canonical_threshold;
    let canonical_update_threshold = args.db.canonical_update_threshold;
    let ledger_cadence = args.db.ledger_cadence;
    let reporting_freq = args.db.reporting_freq;
    let do_not_ingest_orphan_blocks = args.db.do_not_ingest_orphan_blocks;
    let fetch_new_blocks_exe = args.fetch_new_blocks_exe;
    let fetch_new_blocks_delay = args.fetch_new_blocks_delay;
    let verify_block_exe = args.verify_block_exe;
    let missing_block_recovery_exe = args.missing_block_recovery_exe;
    let missing_block_recovery_delay = args.missing_block_recovery_delay;
    let missing_block_recovery_batch = args.missing_block_recovery_batch.unwrap_or(false);
    let blocks_retention_length = args.blocks_retention_length;
    let check_mode = args.db.check_mode;

    // ensure blocks dir exists
    if let Some(ref blocks_dir) = blocks_dir {
        debug!("Ensuring blocks directory exists: {blocks_dir:#?}");
        if let Err(e) = fs::create_dir_all(blocks_dir) {
            error!("Failed to create blocks directory: {e}");
            process::exit(1);
        }
    }

    // ensure staking ledgers dir exists
    if let Some(ref staking_ledgers_dir) = staking_ledgers_dir {
        debug!("Ensuring staking ledgers directory exists: {staking_ledgers_dir:#?}");
        if let Err(e) = fs::create_dir_all(staking_ledgers_dir) {
            error!("Failed to create staging ledger directory: {e}");
            process::exit(1);
        }
    }

    // indexer version
    let network = args.db.network;
    // mesa and devnet run the newer node, which writes currency as decimal MINA;
    // the hardfork mainnet node writes nanomina. Nothing in a block distinguishes
    // them, so the genesis hash is what decides. See [CurrencyEncoding].
    let (version, chain_id, genesis) = if genesis_hash == MESA_GENESIS_HASH {
        (
            PcbVersion::V2(CurrencyEncoding::DecimalMina),
            ChainId::mesa(),
            GenesisVersion::mesa(),
        )
    } else if genesis_hash == DEVNET_GENESIS_HASH {
        (
            PcbVersion::V2(CurrencyEncoding::DecimalMina),
            ChainId::devnet(),
            GenesisVersion::devnet(),
        )
    } else if genesis_hash == HARDFORK_GENESIS_HASH {
        (
            PcbVersion::V2(CurrencyEncoding::Nanomina),
            ChainId::v2(),
            GenesisVersion::v2(),
        )
    } else {
        (PcbVersion::V1, ChainId::v1(), GenesisVersion::v1())
    };

    let genesis_ledger = parse_genesis_ledger(args.db.genesis_ledger, &version)?;
    let version = IndexerVersion {
        network,
        version,
        chain_id,
        genesis,
    };

    Ok(IndexerConfiguration {
        genesis_ledger,
        version,
        blocks_dir,
        staking_ledgers_dir,
        prune_interval,
        canonical_threshold,
        canonical_update_threshold,
        initialization_mode,
        ledger_cadence,
        reporting_freq,
        domain_socket_path,
        fetch_new_blocks_exe,
        fetch_new_blocks_delay,
        verify_block_exe,
        missing_block_recovery_exe,
        missing_block_recovery_delay,
        missing_block_recovery_batch,
        blocks_retention_length,
        do_not_ingest_orphan_blocks,
        check_mode,
    })
}

fn parse_genesis_ledger(
    path: Option<PathBuf>,
    version: &PcbVersion,
) -> anyhow::Result<GenesisLedger> {
    let genesis_ledger = if let Some(path) = path {
        assert!(path.is_file(), "Ledger file does not exist at {path:#?}");
        info!("Parsing ledger file at {path:#?}");

        match GenesisLedger::parse_file(&path) {
            Err(err) => {
                error!("Unable to parse genesis ledger: {err}");
                std::process::exit(100)
            }
            Ok(genesis) => {
                info!("Successfully parsed genesis ledger");
                genesis
            }
        }
    } else {
        info!("Using default {} genesis ledger", version);
        match version {
            PcbVersion::V1 => GenesisLedger::new_v1()?,
            PcbVersion::V2(_) => GenesisLedger::new_v2()?,
        }
    };

    Ok(genesis_ledger)
}

/// Read the pid from a file
fn read_pid_from_file<P: AsRef<Path>>(pid_path: P) -> anyhow::Result<i32> {
    let content = fs::read_to_string(pid_path)?;
    let pid = content.trim().parse()?;
    Ok(pid)
}

/// Write the current pid to a file
fn write_pid_to_file<P: AsRef<Path>>(pid_path: P) -> anyhow::Result<()> {
    let mut pid_file = File::create(pid_path)?;
    let pid = process::id();
    write!(pid_file, "{pid}")?;
    Ok(())
}

/// Restore the database from a periodic checkpoint (the dir written via
/// `MINA_CHECKPOINT_DIR`, containing `latest/`) before the store is opened.
///
/// A speedb checkpoint is itself a complete, openable DB, so "restore" is just
/// copying `<checkpoint_dir>/latest` into `database_dir`. To avoid silently
/// reverting to a (possibly hour-stale) checkpoint over a healthy live DB, an
/// already-populated `database_dir` is left untouched unless `force` is set.
fn maybe_restore_from_checkpoint(
    checkpoint_dir: Option<&Path>,
    database_dir: &Path,
    force: bool,
) -> anyhow::Result<()> {
    let Some(checkpoint_dir) = checkpoint_dir else {
        return Ok(());
    };
    let latest = checkpoint_dir.join("latest");
    if !latest.join("CURRENT").exists() {
        // No usable checkpoint: a fresh PVC, a restart before the first periodic
        // checkpoint is written, or a crash caught mid rename. Don't restore garbage
        // -- but don't abort either. Skip the restore and let the indexer build the
        // DB from blocks. This is what makes --restore-from-checkpoint safe to pass
        // unconditionally from the configless entrypoint (which is the whole point of
        // baking it in), instead of forcing a bespoke conditional-restore wrapper.
        warn!(
            "--restore-from-checkpoint {checkpoint_dir:#?}: no usable checkpoint at {latest:#?} \
             (missing CURRENT); skipping restore, building from blocks"
        );
        return Ok(());
    }

    if database_dir.join("CURRENT").exists() {
        if !force {
            info!(
                "--restore-from-checkpoint: {database_dir:#?} already holds a database; opening it \
                 as-is (it is likely newer than the checkpoint). Pass --restore-force to overwrite."
            );
            return Ok(());
        }
        info!("--restore-force: removing existing database at {database_dir:#?}");
        fs::remove_dir_all(database_dir)?;
    }

    info!("Restoring database from checkpoint {latest:#?} -> {database_dir:#?}");
    copy_dir_recursive(&latest, database_dir)?;
    info!("Checkpoint restore complete");
    Ok(())
}

/// Recursively copy `src` into `dst` (creating `dst`). Used to materialize a
/// speedb checkpoint as a fresh working database directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Result of a `database verify-integrity` scan.
#[derive(serde::Serialize)]
struct IntegrityReport {
    ok: bool,
    store_version: String,
    best_tip_height: Option<u32>,
    best_tip_hash: Option<String>,
    canonical_blocks_checked: u32,
    problems: Vec<String>,
}

impl std::fmt::Display for IntegrityReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Mina Indexer database integrity check")?;
        writeln!(f, "  store version:            {}", self.store_version)?;
        writeln!(
            f,
            "  best tip height:          {}",
            self.best_tip_height
                .map_or_else(|| "<none>".to_string(), |h| h.to_string())
        )?;
        if let Some(h) = &self.best_tip_hash {
            writeln!(f, "  best tip hash:            {h}")?;
        }
        writeln!(
            f,
            "  canonical blocks checked: {}",
            self.canonical_blocks_checked
        )?;
        if self.problems.is_empty() {
            writeln!(f, "  result:                   OK — no problems found")?;
        } else {
            writeln!(
                f,
                "  result:                   {} PROBLEM(S) found:",
                self.problems.len()
            )?;
            for p in &self.problems {
                writeln!(f, "    - {p}")?;
            }
        }
        Ok(())
    }
}

/// Read-only integrity scan: store schema version, best-tip presence, and a walk
/// of the canonical chain (heights `1..=best`) checking contiguity, that each
/// canonical block resolves in the store, and that each block's parent hash is
/// its canonical predecessor. (Ledger-hash recomputation is a future
/// enhancement — this catches the common silent-corruption modes: schema drift,
/// a missing best tip, chain holes, dangling block references, broken linkage.)
fn verify_database_integrity(db: &IndexerStore) -> anyhow::Result<IntegrityReport> {
    let mut problems = Vec::new();

    // 1) store schema version matches this binary
    let version = db.get_db_version()?;
    if version.major != IndexerStoreVersion::MAJOR
        || version.minor != IndexerStoreVersion::MINOR
        || version.patch != IndexerStoreVersion::PATCH
    {
        problems.push(format!(
            "store schema version {} != this binary's {}.{}.{} (a migration / re-index may be required)",
            version.major_minor_patch(),
            IndexerStoreVersion::MAJOR,
            IndexerStoreVersion::MINOR,
            IndexerStoreVersion::PATCH,
        ));
    }

    // 2) best tip present + resolvable
    let best_tip = db.get_best_block()?;
    let best_tip_height = db.get_best_block_height()?;
    let best_tip_hash = best_tip.as_ref().map(|b| b.state_hash().0);
    if best_tip.is_none() {
        problems.push("no best block in the store (empty or corrupt database)".to_string());
    }

    // 3) canonical chain: contiguity + block presence + parent linkage
    let mut canonical_blocks_checked = 0u32;
    if let Some(best_height) = best_tip_height {
        let mut prev_hash: Option<StateHash> = None;
        let mut resumed_after_gap = false;
        for height in 1..=best_height {
            match db.get_canonical_hash_at_height(height)? {
                Some(hash) => {
                    if resumed_after_gap {
                        problems.push(format!(
                            "canonical chain has a hole before height {height} (not contiguous)"
                        ));
                        resumed_after_gap = false;
                    }
                    if db.get_block(&hash)?.is_none() {
                        problems.push(format!(
                            "height {height}: canonical block {hash} is referenced but missing from the store"
                        ));
                    } else if let Some(prev) = prev_hash.as_ref() {
                        match db.get_block_parent_hash(&hash)? {
                            Some(parent) if &parent == prev => {}
                            Some(parent) => problems.push(format!(
                                "height {height}: block {hash} parent {parent} != canonical predecessor {prev}"
                            )),
                            None => problems.push(format!(
                                "height {height}: block {hash} has no parent hash"
                            )),
                        }
                    }
                    prev_hash = Some(hash);
                    canonical_blocks_checked += 1;
                }
                // The canonical chain ends ~k blocks below the tip, so a trailing
                // run of `None` is expected. Only a `Some` *after* a `None` is a
                // real gap (flagged above).
                None => resumed_after_gap = true,
            }
        }
    }

    Ok(IntegrityReport {
        ok: problems.is_empty(),
        store_version: version.to_string(),
        best_tip_height,
        best_tip_hash,
        canonical_blocks_checked,
        problems,
    })
}

/// Remove PID file located in the database directory
fn remove_pid<P: AsRef<Path>>(database_dir: P) {
    let pid_path = database_dir.as_ref().join("PID");
    if let Err(e) = fs::remove_file(pid_path) {
        warn!("Failed to remove PID file: {e}");
    }
}

/// Checks if the current process is the owner of the database by verifying the
/// presence of a PID file. If another process is already running as the owner
/// of the database, the function stops the indexer. Otherwise, it claims
/// ownership by writing the current process ID (PID) into the database
/// directory.
///
/// This function ensures that only one process can own and operate on the
/// database at a time, preventing multiple instances of the indexer from
/// running concurrently.
///
/// # Arguments
///
/// * `database_dir` - A reference to the path of the database directory where
///   the PID file will be located.
fn check_or_write_pid_file<P: AsRef<Path>>(database_dir: P) {
    use mina_indexer::platform;
    let database_dir = database_dir.as_ref();
    let pid_path = database_dir.join("PID");

    if let Err(e) = fs::create_dir_all(database_dir) {
        error!("Failed to create database directory in {database_dir:?}: {e}");
        process::exit(1);
    }

    if let Ok(pid) = read_pid_from_file(&pid_path) {
        if platform::is_process_running(pid) {
            error!("Will not start due to a running Indexer with PID {pid}");
            process::exit(130);
        }
    }

    if let Err(e) = write_pid_to_file(&pid_path) {
        error!("Error writing PID to {pid_path:?}: {e}");
        process::exit(131);
    }
}

#[cfg(test)]
mod verify_integrity_tests {
    use super::{verify_database_integrity, IndexerStore};

    #[test]
    fn clean_but_empty_database_is_flagged() {
        // A freshly-created store has a matching schema version but no blocks —
        // verify-integrity must report a problem (missing best tip) and not "OK".
        let dir = tempfile::tempdir().unwrap();
        let db = IndexerStore::new(dir.path(), true).unwrap();

        let report = verify_database_integrity(&db).unwrap();

        assert!(!report.ok, "an empty database must not report OK");
        assert!(
            report
                .problems
                .iter()
                .any(|p| p.contains("no best block")),
            "expected a 'no best block' problem, got: {:?}",
            report.problems
        );
        assert_eq!(report.best_tip_height, None);
        assert_eq!(report.canonical_blocks_checked, 0);
        // The store version itself matches the binary, so that is NOT a problem.
        assert!(
            !report.problems.iter().any(|p| p.contains("schema version")),
            "freshly-created store version should match the binary"
        );
    }
}

#[cfg(test)]
mod checkpoint_restore_tests {
    //! Recovery half of the checkpoint crash-consistency story (the write half
    //! lives in `server::tests`). The periodic checkpoint only ever swaps
    //! `<dir>/latest` via an atomic rename, so a `kill -9` mid-write can leave
    //! `latest` briefly absent — but never a *partial* DB. `maybe_restore_from_checkpoint`
    //! gates on `latest/CURRENT`: an absent/partial checkpoint is *skipped* (no
    //! garbage restored) and the caller builds the DB from blocks — it must never
    //! abort, so `--restore-from-checkpoint` is safe to pass unconditionally from the
    //! configless entrypoint (a fresh PVC has no checkpoint yet).
    use super::{copy_dir_recursive, maybe_restore_from_checkpoint};
    use std::fs;

    /// Make `<dir>/latest` look like a complete speedb DB (CURRENT is the marker
    /// the restore path checks) with a couple of files to copy.
    fn make_valid_latest(checkpoint_dir: &std::path::Path) {
        let latest = checkpoint_dir.join("latest");
        fs::create_dir_all(latest.join("subdir")).unwrap();
        fs::write(latest.join("CURRENT"), b"MANIFEST-000001\n").unwrap();
        fs::write(latest.join("MANIFEST-000001"), b"manifest").unwrap();
        fs::write(latest.join("subdir/000001.sst"), b"sst").unwrap();
    }

    #[test]
    fn none_checkpoint_is_a_noop() {
        let db = tempfile::tempdir().unwrap();
        assert!(maybe_restore_from_checkpoint(None, db.path(), false).is_ok());
        assert!(!db.path().join("CURRENT").exists());
    }

    #[test]
    fn absent_checkpoint_skips_and_continues() {
        // Reproduction for the recurring bootstrap failure: on a fresh PVC there is
        // no checkpoint yet, but the configless entrypoint always passes
        // --restore-from-checkpoint. This function must SKIP the restore and let the
        // indexer build from blocks -- NOT abort. Aborting is what forced the gitops
        // entrypoint workaround that then dropped the bulk-fetch (a whole prod incident).
        let ckpt = tempfile::tempdir().unwrap(); // empty: no latest/CURRENT
        let db = tempfile::tempdir().unwrap();
        maybe_restore_from_checkpoint(Some(ckpt.path()), db.path(), false)
            .expect("absent checkpoint must skip (build from blocks), not abort");
        assert!(
            !db.path().join("CURRENT").exists(),
            "nothing should be restored from an absent checkpoint"
        );
    }

    #[test]
    fn partial_latest_without_current_skips_restore() {
        // A `latest` dir that exists but lacks CURRENT (a crash caught mid rename).
        // Skip it -- don't copy the orphan files, and don't abort.
        let ckpt = tempfile::tempdir().unwrap();
        fs::create_dir_all(ckpt.path().join("latest")).unwrap();
        fs::write(ckpt.path().join("latest/000001.sst"), b"orphan sst").unwrap();
        let db = tempfile::tempdir().unwrap();
        maybe_restore_from_checkpoint(Some(ckpt.path()), db.path(), false)
            .expect("partial checkpoint must skip, not error");
        assert!(
            !db.path().join("000001.sst").exists(),
            "orphan checkpoint files must not be restored"
        );
        assert!(!db.path().join("CURRENT").exists());
    }

    #[test]
    fn absent_checkpoint_keeps_existing_db() {
        // Absent checkpoint must never destroy an already-populated DB (a restart
        // before the first periodic checkpoint is written).
        let ckpt = tempfile::tempdir().unwrap(); // no latest/CURRENT
        let db = tempfile::tempdir().unwrap();
        fs::write(db.path().join("CURRENT"), b"live db\n").unwrap();
        maybe_restore_from_checkpoint(Some(ckpt.path()), db.path(), false)
            .expect("absent checkpoint must skip, not error");
        assert_eq!(
            fs::read(db.path().join("CURRENT")).unwrap(),
            b"live db\n",
            "existing DB must be preserved"
        );
    }

    #[test]
    fn valid_checkpoint_is_restored_into_empty_dir() {
        let ckpt = tempfile::tempdir().unwrap();
        make_valid_latest(ckpt.path());
        let db = tempfile::tempdir().unwrap();
        // remove so the target dir is truly absent (fresh restore)
        fs::remove_dir_all(db.path()).unwrap();

        maybe_restore_from_checkpoint(Some(ckpt.path()), db.path(), false).unwrap();

        assert!(db.path().join("CURRENT").exists());
        assert_eq!(fs::read(db.path().join("MANIFEST-000001")).unwrap(), b"manifest");
        assert_eq!(fs::read(db.path().join("subdir/000001.sst")).unwrap(), b"sst");
    }

    #[test]
    fn existing_db_is_kept_without_force_and_overwritten_with_force() {
        let ckpt = tempfile::tempdir().unwrap();
        make_valid_latest(ckpt.path());

        // Target already holds a (different) DB.
        let db = tempfile::tempdir().unwrap();
        fs::write(db.path().join("CURRENT"), b"existing db\n").unwrap();

        // Without force: left as-is (the live DB is usually newer than the checkpoint).
        maybe_restore_from_checkpoint(Some(ckpt.path()), db.path(), false).unwrap();
        assert_eq!(fs::read(db.path().join("CURRENT")).unwrap(), b"existing db\n");

        // With force: replaced by the checkpoint.
        maybe_restore_from_checkpoint(Some(ckpt.path()), db.path(), true).unwrap();
        assert_eq!(fs::read(db.path().join("CURRENT")).unwrap(), b"MANIFEST-000001\n");
        assert!(db.path().join("subdir/000001.sst").exists());
    }

    #[test]
    fn copy_dir_recursive_copies_nested_tree() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("a/b")).unwrap();
        fs::write(src.path().join("top.txt"), b"top").unwrap();
        fs::write(src.path().join("a/mid.txt"), b"mid").unwrap();
        fs::write(src.path().join("a/b/leaf.txt"), b"leaf").unwrap();

        let dst = tempfile::tempdir().unwrap();
        let dst = dst.path().join("copy");
        copy_dir_recursive(src.path(), &dst).unwrap();

        assert_eq!(fs::read(dst.join("top.txt")).unwrap(), b"top");
        assert_eq!(fs::read(dst.join("a/mid.txt")).unwrap(), b"mid");
        assert_eq!(fs::read(dst.join("a/b/leaf.txt")).unwrap(), b"leaf");
    }
}
