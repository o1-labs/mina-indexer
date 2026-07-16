//! Server & file

use crate::{
    base::state_hash::StateHash,
    block::{
        self,
        parser::BlockParser,
        precomputed::{CurrencyEncoding, PcbVersion, PrecomputedBlock},
        vrf_output::VrfOutput,
    },
    chain::{ChainId, Network},
    cli::server::ServerArgsJson,
    constants::*,
    ledger::{
        genesis::GenesisLedger, staking::StakingLedger, store::staking::StakingLedgerStore,
        LedgerHash,
    },
    state::{IndexerState, IndexerStateConfig},
    store::IndexerStore,
    unix_socket_server::{create_socket_listener, handle_connection},
    utility::functions::extract_network_height_hash,
};
use log::{debug, error, info, trace, warn};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use speedb::checkpoint::Checkpoint;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process,
    sync::Arc,
    time::Duration,
};
use tokio::{
    runtime::Handle,
    sync::{mpsc, RwLock},
};
use tokio_graceful_shutdown::{SubsystemBuilder, SubsystemHandle};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IndexerVersion {
    pub network: Network,
    pub version: PcbVersion,
    pub chain_id: ChainId,
    pub genesis: GenesisVersion,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GenesisVersion {
    pub state_hash: StateHash,
    pub prev_hash: StateHash,
    pub blockchain_lenth: u32,
    pub global_slot: u32,
    pub last_vrf_output: VrfOutput,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IndexerConfiguration {
    pub genesis_ledger: GenesisLedger,
    pub version: IndexerVersion,
    pub blocks_dir: Option<PathBuf>,
    pub staking_ledgers_dir: Option<PathBuf>,
    pub prune_interval: u32,
    pub canonical_threshold: u32,
    pub canonical_update_threshold: u32,
    pub initialization_mode: InitializationMode,
    pub ledger_cadence: u32,
    pub reporting_freq: u32,
    pub domain_socket_path: PathBuf,
    pub do_not_ingest_orphan_blocks: bool,
    pub fetch_new_blocks_exe: Option<PathBuf>,
    pub fetch_new_blocks_delay: Option<u64>,
    pub verify_block_exe: Option<PathBuf>,
    pub missing_block_recovery_exe: Option<PathBuf>,
    pub missing_block_recovery_delay: Option<u64>,
    pub missing_block_recovery_batch: bool,
    pub blocks_retention_length: Option<u32>,
    pub check_mode: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum InitializationMode {
    BuildDB,
    Replay,
    Sync,
}

///////////
// impls //
///////////

impl IndexerConfiguration {
    /// Initializes indexer database
    ///
    /// The purpose of this mode is to create a known good initial
    /// database so that it may be used and shared with other Mina
    /// Indexers
    pub async fn initialize_indexer_database(
        self,
        store: &Arc<IndexerStore>,
    ) -> anyhow::Result<()> {
        let state = self.initialize(store).await.unwrap_or_else(|e| {
            error!("Failed to initialize mina indexer store: {e}");
            std::process::exit(1);
        });

        if let Some(indexer_store) = state.indexer_store.as_ref() {
            // Persist memtables + WAL so a subsequent open is fast (see the
            // server-loop shutdown for rationale).
            let _ = indexer_store.database.flush();
            let _ = indexer_store.database.flush_wal(true);
            indexer_store.database.cancel_all_background_work(true);
        }

        Ok(())
    }

    /// Initializes the indexer with the given config & store
    async fn initialize(self, store: &Arc<IndexerStore>) -> anyhow::Result<IndexerState> {
        debug!("Initializing mina indexer database");
        let db_path = store.db_path.clone();

        // read the config from the store if it exists or write it
        let IndexerConfiguration {
            genesis_ledger,
            blocks_dir,
            staking_ledgers_dir,
            prune_interval,
            canonical_threshold,
            canonical_update_threshold,
            initialization_mode,
            ledger_cadence,
            reporting_freq,
            version,
            do_not_ingest_orphan_blocks,
            check_mode,
            ..
        } = self;

        // blocks dir
        if let Some(ref blocks_dir) = blocks_dir {
            if let Err(e) = fs::create_dir_all(blocks_dir) {
                error!(
                    "Failed to create blocks directory in {:#?}: {}",
                    blocks_dir, e
                );
                process::exit(1);
            }
        }

        // staking ledger dir
        if let Some(ref staking_ledgers_dir) = staking_ledgers_dir {
            if let Err(e) = fs::create_dir_all(staking_ledgers_dir) {
                error!(
                    "Failed to create staking ledgers directory in {:#?}: {}",
                    staking_ledgers_dir, e
                );
                process::exit(1);
            }
        }

        let pcb_version = version.version.to_owned();
        let state_config = IndexerStateConfig {
            indexer_store: store.clone(),
            version: version.clone(),
            genesis_ledger: genesis_ledger.clone(),
            transition_frontier_length: MAINNET_TRANSITION_FRONTIER_K,
            do_not_ingest_orphan_blocks,
            prune_interval,
            canonical_threshold,
            canonical_update_threshold,
            ledger_cadence,
            reporting_freq,
            check_mode,
        };

        let mut state = match initialization_mode {
            InitializationMode::BuildDB => {
                log_dirs_msg(blocks_dir.as_ref(), staking_ledgers_dir.as_ref());
                IndexerState::new_from_config(state_config)?
            }
            InitializationMode::Replay => {
                info!("Replaying indexer events from db at {db_path:#?}");
                IndexerState::new_without_genesis_events(state_config)?
            }
            InitializationMode::Sync => {
                info!("Syncing indexer state from db at {db_path:#?}");
                IndexerState::new_without_genesis_events(state_config)?
            }
        };

        // ingest staking ledgers
        if let Some(ref staking_ledgers_dir) = staking_ledgers_dir {
            if let Err(e) = state
                .add_startup_staking_ledgers_to_store(staking_ledgers_dir)
                .await
            {
                panic!("Failed to ingest staking ledger {staking_ledgers_dir:#?}: {e}");
            }
        }

        // build witness tree & ingest precomputed blocks
        match initialization_mode {
            InitializationMode::BuildDB => {
                if let Some(ref blocks_dir) = blocks_dir {
                    let mut block_parser = BlockParser::new_with_canonical_chain_discovery(
                        blocks_dir,
                        pcb_version,
                        canonical_threshold,
                        do_not_ingest_orphan_blocks,
                        reporting_freq,
                    )
                    .await
                    .unwrap_or_else(|e| panic!("Obtaining block parser failed: {e}"));
                    state
                        .initialize_with_canonical_chain_discovery(&mut block_parser)
                        .await?;
                }
            }
            InitializationMode::Replay => {
                if let Ok(ref replay_state) =
                    IndexerState::new_without_genesis_events(IndexerStateConfig {
                        indexer_store: store.clone(),
                        version,
                        genesis_ledger,
                        transition_frontier_length: MAINNET_TRANSITION_FRONTIER_K,
                        prune_interval,
                        canonical_threshold,
                        canonical_update_threshold,
                        ledger_cadence,
                        reporting_freq,
                        do_not_ingest_orphan_blocks,
                        check_mode,
                    })
                {
                    let min_length_filter = state.replay_events(replay_state)?;
                    if let Some(ref blocks_dir) = blocks_dir {
                        let mut block_parser = BlockParser::new_length_sorted_min_filtered(
                            blocks_dir,
                            pcb_version,
                            min_length_filter,
                        )?;

                        if block_parser.total_num_blocks > 0 {
                            info!("Adding new blocks from {blocks_dir:#?}");
                            state.add_blocks(&mut block_parser).await?;
                        }
                    }
                }
            }
            InitializationMode::Sync => {
                let min_length_filter = state.sync_from_db()?;
                if let Some(ref blocks_dir) = blocks_dir {
                    let mut block_parser = BlockParser::new_length_sorted_min_filtered(
                        blocks_dir,
                        pcb_version,
                        min_length_filter,
                    )?;

                    if block_parser.total_num_blocks > 0 {
                        info!("Adding new blocks from {blocks_dir:#?}");
                        state.add_blocks(&mut block_parser).await?;
                    }
                }
            }
        }

        // flush/compress database
        let store = state.indexer_store.as_ref().unwrap();
        let temp_checkpoint_dir = store.db_path.join("tmp-checkpoint");

        Checkpoint::new(&store.database)?.create_checkpoint(&temp_checkpoint_dir)?;
        fs::remove_dir_all(&temp_checkpoint_dir)?;

        Ok(state)
    }

    /// Initializes witness tree, connects database, starts UDS server & runs
    /// the indexer
    pub async fn start_indexer(
        self,
        subsys: SubsystemHandle,
        store: Arc<IndexerStore>,
    ) -> anyhow::Result<()> {
        let blocks_dir = self.blocks_dir.clone();
        let staking_ledgers_dir = self.staking_ledgers_dir.clone();
        let domain_socket_path = self.domain_socket_path.clone();

        let fetch_new_blocks_delay = self.fetch_new_blocks_delay;
        let fetch_new_blocks_exe = self.fetch_new_blocks_exe.clone();
        let verify_block_exe = self.verify_block_exe.clone();

        let missing_block_recovery_delay = self.missing_block_recovery_delay;
        let missing_block_recovery_exe = self.missing_block_recovery_exe.clone();
        let missing_block_recovery_batch = self.missing_block_recovery_batch;
        let blocks_retention_length = self.blocks_retention_length;

        // initialize witness tree & connect database
        let state = Arc::new(RwLock::new(self.initialize(&store).await.unwrap_or_else(
            |e| {
                error!("Failed to initialize mina indexer state: {e}");
                std::process::exit(1);
            },
        )));

        // read-only state
        start_uds_server(&subsys, state.clone(), &domain_socket_path).await?;

        // modifies the state
        let missing_block_recovery =
            missing_block_recovery_exe.map(|exe| MissingBlockRecoveryOptions {
                exe,
                batch: missing_block_recovery_batch,
                delay: missing_block_recovery_delay.unwrap_or(180),
            });
        let fetch_new_blocks = fetch_new_blocks_exe.map(|exe| FetchNewBlocksOptions {
            exe,
            delay: fetch_new_blocks_delay.unwrap_or(180),
        });

        // optional periodic DB checkpoints (opt-in via MINA_CHECKPOINT_DIR; the
        // configless images enable it so a crash resumes from a recent consistent
        // checkpoint instead of a slow WAL replay)
        if let Ok(dir) = std::env::var("MINA_CHECKPOINT_DIR") {
            spawn_periodic_checkpoints(store.clone(), PathBuf::from(dir));
        }

        run_indexer(
            &subsys,
            blocks_dir,
            staking_ledgers_dir,
            missing_block_recovery,
            fetch_new_blocks,
            verify_block_exe,
            blocks_retention_length,
            state.clone(),
        )
        .await?;

        Ok(())
    }
}

/// Spawn a background task that writes a rolling speedb checkpoint of the DB to
/// `<dir>/latest` every `MINA_CHECKPOINT_INTERVAL_SECS` (default 3600 = hourly).
/// On an ungraceful crash, restart from the checkpoint instead of replaying a
/// large WAL. The checkpoint is hard-link based (cheap) and consistent.
fn spawn_periodic_checkpoints(store: Arc<IndexerStore>, dir: PathBuf) {
    let secs: u64 = std::env::var("MINA_CHECKPOINT_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(3600);
    info!("Periodic DB checkpoints enabled -> {dir:#?} (every {secs}s)");
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(secs));
        tick.tick().await; // consume the immediate first tick
        loop {
            tick.tick().await;
            let store = store.clone();
            let dir = dir.clone();
            // create_checkpoint is blocking I/O — keep it off the async workers
            match tokio::task::spawn_blocking(move || write_db_checkpoint(&store, &dir)).await {
                Ok(Ok(path)) => {
                    tracing::info!(path = %path.display(), "DB checkpoint written")
                }
                Ok(Err(e)) => tracing::error!(error = %e, "DB checkpoint failed"),
                Err(e) => tracing::error!(error = %e, "DB checkpoint task panicked"),
            }
        }
    });
}

/// Write a consistent speedb checkpoint to `<dir>/latest`, atomically (tmp + rename).
pub(crate) fn write_db_checkpoint(store: &IndexerStore, dir: &Path) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let tmp = dir.join(".tmp-checkpoint");
    let latest = dir.join("latest");
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    Checkpoint::new(&store.database)?.create_checkpoint(&tmp)?;
    if latest.exists() {
        fs::remove_dir_all(&latest)?;
    }
    fs::rename(&tmp, &latest)?;
    Ok(latest)
}

/// Starts UDS server with read-only state for summary
async fn start_uds_server(
    subsys: &SubsystemHandle,
    state: Arc<RwLock<IndexerState>>,
    domain_socket_path: &Path,
) -> anyhow::Result<()> {
    let listener = create_socket_listener(domain_socket_path);

    subsys.start(SubsystemBuilder::new("Socket Listener", {
        move |subsys| handle_connection(listener, state, subsys)
    }));

    Ok(())
}

fn matches_event_kind(kind: EventKind) -> bool {
    use notify::event::{AccessKind, AccessMode, CreateKind, ModifyKind};

    matches!(
        kind,
        EventKind::Create(CreateKind::File)
            | EventKind::Modify(ModifyKind::Name(_))
            | EventKind::Access(AccessKind::Close(AccessMode::Write))
    )
}

struct MissingBlockRecoveryOptions {
    pub delay: u64,
    pub exe: PathBuf,
    pub batch: bool,
}

struct FetchNewBlocksOptions {
    pub delay: u64,
    pub exe: PathBuf,
}

/// Starts filesystem watchers & runs the mina indexer
#[allow(clippy::too_many_arguments)]
async fn run_indexer<P: AsRef<Path>>(
    subsys: &SubsystemHandle,
    blocks_dir: Option<P>,
    staking_ledgers_dir: Option<P>,
    missing_block_recovery: Option<MissingBlockRecoveryOptions>,
    fetch_new_blocks_opts: Option<FetchNewBlocksOptions>,
    verify_block_exe: Option<PathBuf>,
    blocks_retention_length: Option<u32>,
    state: Arc<RwLock<IndexerState>>,
) -> anyhow::Result<()> {
    // setup fs-based precomputed block & staking ledger watchers
    let (tx, mut rx) = mpsc::channel(4096);
    let rt = Handle::current();
    let mut watcher = RecommendedWatcher::new(
        move |result| {
            let tx = tx.clone();
            rt.spawn(async move {
                if let Err(e) = tx.send(result).await {
                    error!("Failed to send watcher event, closing: {e}");
                    drop(tx);
                }
            });
        },
        Config::default(),
    )?;

    if let Some(ref blocks_dir) = blocks_dir {
        watcher.watch(blocks_dir.as_ref(), RecursiveMode::NonRecursive)?;
        info!(
            "Watching for precomputed blocks in directory: {:#?}",
            blocks_dir.as_ref()
        );
    }

    if let Some(ref staking_ledgers_dir) = staking_ledgers_dir {
        watcher.watch(staking_ledgers_dir.as_ref(), RecursiveMode::NonRecursive)?;
        info!(
            "Watching for staking ledgers in directory: {:#?}",
            staking_ledgers_dir.as_ref()
        );
    }

    // fetch new block options
    let fetch_new_blocks_delay = fetch_new_blocks_opts.as_ref().map(|f| f.delay);
    let fetch_new_blocks_exe = fetch_new_blocks_opts.map(|f| f.exe);

    // missing block recovery options
    let missing_block_recovery_batch = missing_block_recovery.as_ref().is_some_and(|m| m.batch);
    let missing_block_recovery_delay = missing_block_recovery.as_ref().map(|m| m.delay);
    let missing_block_recovery_exe = missing_block_recovery.map(|m| m.exe);

    if let Some(retention) = blocks_retention_length {
        let effective = retention.max(MAINNET_TRANSITION_FRONTIER_K);
        info!(
            "Block-file retention enabled: keeping blocks within {effective} of the tip \
             (requested {retention}, floored at k={MAINNET_TRANSITION_FRONTIER_K})"
        );
    }

    loop {
        tokio::select! {
            // watch for shutdown signals
            _ = subsys.on_shutdown_requested() => {
                break;
            }

            // watch for precomputed blocks & staking ledgers
            Some(res) = rx.recv() => {
                match res {
                    Ok(event) => process_event(event, &state, verify_block_exe.as_deref()).await?,
                    Err(e) => {
                        error!("Filesystem watcher error: {e}");
                        break;
                    }
                }
            }

            // fetch new blocks & recover missing blocks
            _ = tokio::time::sleep(std::time::Duration::from_secs(
                fetch_new_blocks_delay.unwrap_or(180).min(missing_block_recovery_delay.unwrap_or(180))
            )) => {
                if let Some(ref blocks_dir) = blocks_dir {
                    if let Some(ref fetch_new_blocks_exe) = fetch_new_blocks_exe {
                        fetch_new_blocks(&state, &blocks_dir, fetch_new_blocks_exe).await?
                    }

                    if let Some(ref missing_block_recovery_exe) = missing_block_recovery_exe {
                        recover_missing_blocks(&state, &blocks_dir, missing_block_recovery_exe, missing_block_recovery_batch).await?
                    }

                    // Safety net: ingest any on-disk block the fs-watcher missed.
                    // inotify has a bounded event queue; a bulk fetch (hundreds
                    // of files at once) overflows it and silently drops events,
                    // leaving connectable blocks orphaned on disk that the fetch
                    // hook then skips (already downloaded) — wedging the tip.
                    reconcile_blocks_dir(&state, blocks_dir.as_ref(), verify_block_exe.as_deref()).await?;

                    // Retention: drop ingested block files below the window so
                    // blocks_dir doesn't grow unbounded as the tip advances.
                    if let Some(retention) = blocks_retention_length {
                        prune_blocks_dir(&state, blocks_dir.as_ref(), retention).await?;
                    }
                }
            }
        }
    }

    // shutdown
    let state = state.write().await;
    if let Some(store) = state.indexer_store.as_ref() {
        debug!("Flushing db memtables + WAL before shutdown");
        // Drain memtables to SST and sync the WAL so the next `server start`
        // opens cleanly instead of replaying a multi-GB WAL (minutes of
        // recovery). Atomic-flush makes the single flush() cover every CF.
        let _ = store.database.flush();
        let _ = store.database.flush_wal(true);
        debug!("Canceling db background work");
        store.database.cancel_all_background_work(true)
    }

    debug!("Filesystem watchers successfully shutdown");
    Ok(())
}

async fn retry_parse_precomputed_block(
    path: &Path,
    version: PcbVersion,
) -> anyhow::Result<PrecomputedBlock> {
    let num_attempts = 5;
    let mut last_err = None;
    for attempt in 1..num_attempts {
        // Parse with the indexer's configured network version, NOT
        // `from_path` (which guesses the version from blockchain length via the
        // mainnet hardfork threshold). On a non-mainnet hardfork like mesa-mut
        // that guess is wrong — every live-fetched block would be parsed as the
        // pre-fork version and fail with "missing field protocol_state", so the
        // tip could only ever advance via the BuildDB scan, never live fetch.
        match PrecomputedBlock::parse_file(path, version.clone()) {
            Ok(block) => return Ok(block),
            Err(e) => {
                warn!("Attempt {attempt}: {e}. Retrying in 100ms...");
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }

    // Don't panic on a single bad block — flag it and let the caller skip it so
    // the indexer stays up. A transiently-bad block (e.g. a partial download) is
    // re-fetched later by missing-block-recovery and inserted then; a
    // permanently-incompatible one (e.g. a pre-fork block) stays skipped.
    anyhow::bail!(
        "Failed to parse precomputed block {} after {num_attempts} attempts: {}",
        path.display(),
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown error".to_string()),
    )
}

async fn retry_parse_staking_ledger(path: &Path) -> anyhow::Result<StakingLedger> {
    let num_attempts = 5;
    let mut last_err = None;
    for attempt in 1..num_attempts {
        match StakingLedger::parse_file(path).await {
            Ok(ledger) => return Ok(ledger),
            Err(e) => {
                warn!("Attempt {attempt}: {e}. Retrying in 1s...");
                last_err = Some(e);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }

    // Flag-and-skip instead of crashing the indexer (see
    // retry_parse_precomputed_block).
    anyhow::bail!(
        "Failed to parse staking ledger {} after {num_attempts} attempts: {}",
        path.display(),
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown error".to_string()),
    )
}

/// Apply a block via [`IndexerState::block_pipeline`], catching any panic so a
/// single poison block cannot take down the whole ingestion subsystem.
///
/// `block_pipeline` is synchronous. Without this guard a panic in it propagates
/// out of the watcher/reconcile task; `tokio_graceful_shutdown` treats a
/// subsystem panic as fatal, so the process exits — and on restart it re-ingests
/// the very same block and panics again: a crash loop that takes the indexer
/// permanently offline. Catching converts that into a logged, counted, skipped
/// block (via the callers' existing `Err` arms) while every other block stays
/// consistent and the service keeps serving reads.
///
/// Trade-off: after a caught panic the in-memory state for *that* block may be
/// partially applied. That is strictly better than a total outage — a
/// deterministic panic just skips that one block forever. `tokio`'s `RwLock`
/// does not poison on panic, so the held write guard remains usable.
fn apply_block_catching_panic(
    state: &mut IndexerState,
    block: &PrecomputedBlock,
    block_bytes: u64,
) -> anyhow::Result<bool> {
    catch_block_apply(|| state.block_pipeline(block, block_bytes))
}

/// Run a synchronous block-apply closure, converting a panic into an `Err`
/// rather than letting it unwind out of the ingestion task. Split out from
/// [`apply_block_catching_panic`] so the catch behavior is unit-testable without
/// constructing a full `IndexerState`.
fn catch_block_apply<F>(apply: F) -> anyhow::Result<bool>
where
    F: FnOnce() -> anyhow::Result<bool>,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(apply)) {
        Ok(res) => res,
        Err(_) => Err(anyhow::anyhow!(
            "panic while applying block to the witness tree (block skipped to keep ingestion alive)"
        )),
    }
}

/// Precomputed block & staking ledger event handler
async fn process_event(
    event: Event,
    state: &Arc<RwLock<IndexerState>>,
    verify_block_exe: Option<&Path>,
) -> anyhow::Result<()> {
    trace!("{:?}", event);

    if matches_event_kind(event.kind) {
        for path in event.paths {
            if block::is_valid_block_file(&path) {
                debug!("Valid precomputed block file: {:#?}", path);

                // exit early if present
                if check_block(state, &path).await {
                    return Ok(());
                }

                // trustless gate: only ingest blocks whose proof verifies
                if let Some(exe) = verify_block_exe {
                    let network = state.read().await.version.network.clone();
                    if !verify_block(exe, &network, &path).await {
                        warn!("Rejected block (proof did not verify): {:#?}", path);
                        continue;
                    }
                }

                // if the block isn't in the witness tree, parse & pipeline it
                // using the indexer's configured network version
                let pcb_version = state.read().await.version.version.clone();
                match retry_parse_precomputed_block(&path, pcb_version).await {
                    Ok(block) => {
                        let mut state = state.write().await;
                        let height = block.blockchain_length();
                        let state_hash = block.state_hash();
                        let apply_start = std::time::Instant::now();

                        let len = path.metadata()?.len();
                        match apply_block_catching_panic(&mut state, &block, len) {
                            Ok(is_added) => {
                                if is_added {
                                    tracing::info!(
                                        height,
                                        state_hash = %state_hash,
                                        duration_ms = apply_start.elapsed().as_millis() as u64,
                                        "Added block"
                                    );
                                }
                            }
                            Err(e) => {
                                crate::metrics::BLOCKS_INGEST_FAILED.inc();
                                tracing::error!(
                                    height,
                                    state_hash = %state_hash,
                                    error = %e,
                                    "Error adding block"
                                );
                            }
                        }
                    }
                    Err(e) => tracing::error!(error = %e, "Error parsing precomputed block"),
                }
            } else if StakingLedger::is_valid(&path) {
                debug!("Valid staking ledger file: {:#?}", path);

                // exit early if present
                if check_staking_ledger(state, &path).await {
                    return Ok(());
                }

                // if staking ledger is not in the witness tree, parse & add it
                let mut state = state.write().await;
                if let Some(store) = state.indexer_store.as_ref() {
                    match retry_parse_staking_ledger(&path).await {
                        Ok(staking_ledger) => {
                            let epoch = staking_ledger.epoch;
                            let ledger_hash = staking_ledger.ledger_hash.clone();
                            let ledger_summary = staking_ledger.summary();

                            info!("Adding staking ledger {}", ledger_summary);
                            store
                                .add_staking_ledger(staking_ledger)
                                .unwrap_or_else(|e| {
                                    error!("Error adding staking ledger {}: {}", ledger_summary, e)
                                });

                            state.staking_ledgers.insert((epoch, ledger_hash));
                        }
                        Err(e) => {
                            error!("Error parsing staking ledger: {}", e)
                        }
                    }
                } else {
                    error!("Indexer store unavailable");
                }
            }
        }
    }

    Ok(())
}

/// Re-scan the blocks directory and ingest any valid block the filesystem
/// watcher missed. inotify has a bounded kernel event queue; a bulk fetch
/// (hundreds of files mv'd in at once) overflows it and silently drops events,
/// so connectable blocks sit orphaned on disk while the fetch hook skips them
/// (already downloaded) — wedging the tip. This periodic reconcile is the
/// safety net that makes ingestion robust regardless of OS event delivery.
///
/// Idempotent: blocks already in the witness tree are skipped. Bounded to the
/// transition frontier (best tip - k and above) so it doesn't re-stat the whole
/// chain every cycle. Ascending height so parents are applied before children.
async fn reconcile_blocks_dir(
    state: &Arc<RwLock<IndexerState>>,
    blocks_dir: &Path,
    verify_block_exe: Option<&Path>,
) -> anyhow::Result<()> {
    let floor = {
        let st = state.read().await;
        st.best_tip_block()
            .blockchain_length
            .saturating_sub(MAINNET_TRANSITION_FRONTIER_K)
    };

    let mut candidates: Vec<(u32, PathBuf)> = match std::fs::read_dir(blocks_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| block::is_valid_block_file(p))
            .map(|p| (extract_network_height_hash(&p).1, p))
            .filter(|(height, _)| *height >= floor)
            .collect(),
        Err(e) => {
            error!("reconcile: cannot read {blocks_dir:#?}: {e}");
            return Ok(());
        }
    };
    candidates.sort_by_key(|(height, _)| *height);

    let mut reconciled = 0;
    for (_, path) in candidates {
        // quiet presence check (unlike check_block, which logs per block)
        let (_, _, state_hash) = extract_network_height_hash(&path);
        let state_hash: StateHash = state_hash.into();
        if state.read().await.diffs_map.contains_key(&state_hash) {
            continue;
        }

        // trustless gate: only ingest blocks whose proof verifies
        if let Some(exe) = verify_block_exe {
            let network = state.read().await.version.network.clone();
            if !verify_block(exe, &network, &path).await {
                warn!("Rejected block (proof did not verify): {:#?}", path);
                continue;
            }
        }

        let version = state.read().await.version.version.clone();
        match retry_parse_precomputed_block(&path, version).await {
            Ok(block) => {
                let len = match path.metadata() {
                    Ok(m) => m.len(),
                    Err(_) => continue,
                };
                let height = block.blockchain_length();
                let state_hash = block.state_hash();
                let mut st = state.write().await;
                match apply_block_catching_panic(&mut st, &block, len) {
                    Ok(true) => {
                        reconciled += 1;
                        tracing::info!(height, state_hash = %state_hash, "Reconciled on-disk block");
                    }
                    Ok(false) => {}
                    Err(e) => {
                        crate::metrics::BLOCKS_INGEST_FAILED.inc();
                        tracing::error!(
                            height,
                            state_hash = %state_hash,
                            error = %e,
                            "Error reconciling block"
                        );
                    }
                }
            }
            // unparseable blocks are skipped (logged once on the watcher path)
            Err(e) => trace!("reconcile: skipping {path:#?}: {e}"),
        }
    }
    crate::metrics::RECONCILE_INGESTED.inc_by(reconciled as u64);
    crate::metrics::DANGLING_BRANCHES.set(state.read().await.dangling_branches.len() as i64);
    if reconciled > 0 {
        tracing::info!(
            ingested = reconciled,
            "Reconcile ingested on-disk block(s) the fs-watcher missed"
        );
    }
    Ok(())
}

/// Bound `blocks_dir` growth by deleting ingested precomputed-block files older
/// than the retention window. Block files at height >= `best_tip - retention`
/// are kept; everything below already lives in the speedb store and is never
/// re-read — queries serve from the store, and `reconcile_blocks_dir` only
/// re-scans heights >= `tip - k`. The window is floored at `k`
/// ([`MAINNET_TRANSITION_FRONTIER_K`]) so reconcile always retains the recent
/// blocks it depends on, regardless of the requested value.
///
/// Best-effort and idempotent: read/remove errors are logged and skipped, and a
/// tip shallower than the window simply prunes nothing.
async fn prune_blocks_dir(
    state: &Arc<RwLock<IndexerState>>,
    blocks_dir: &Path,
    retention: u32,
) -> anyhow::Result<()> {
    let retention = retention.max(MAINNET_TRANSITION_FRONTIER_K);
    let floor = {
        let st = state.read().await;
        st.best_tip_block().blockchain_length.saturating_sub(retention)
    };
    if floor == 0 {
        return Ok(()); // tip not yet deep enough for anything to fall out of the window
    }

    let mut pruned = 0u64;
    let mut bytes_freed = 0u64;
    for path in prunable_block_files(blocks_dir, floor) {
        let len = path.metadata().map(|m| m.len()).unwrap_or(0);
        match std::fs::remove_file(&path) {
            Ok(()) => {
                pruned += 1;
                bytes_freed += len;
            }
            Err(e) => warn!("prune: failed to remove {path:#?}: {e}"),
        }
    }

    if pruned > 0 {
        crate::metrics::BLOCKS_PRUNED.inc_by(pruned);
        info!(
            "Pruned {pruned} ingested block file(s) below height {floor} ({:.1} MiB freed)",
            bytes_freed as f64 / (1024.0 * 1024.0)
        );
    }
    Ok(())
}

/// Block files in `blocks_dir` strictly below `floor` (height < floor) — the set
/// safe to delete once their height has fallen out of the retention window.
/// Non-block files and unreadable dirs yield nothing. Pure I/O selection, split
/// out from [`prune_blocks_dir`] so the height gating is unit-testable.
fn prunable_block_files(blocks_dir: &Path, floor: u32) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(blocks_dir) {
        Ok(rd) => rd,
        Err(e) => {
            error!("prune: cannot read {blocks_dir:#?}: {e}");
            return Vec::new();
        }
    };
    entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| block::is_valid_block_file(p))
        .filter(|p| extract_network_height_hash(p).1 < floor)
        .collect()
}

/// Trustless gate: run the external verifier on a block file. The contract
/// mirrors the fetch hook — `EXE <network> <block-file>` — and a zero exit code
/// means the block's SNARK proof verified. Fails closed: if the verifier can't
/// be run, the block is rejected (an unverifiable block is never ingested in
/// trustless mode).
async fn verify_block(exe: &Path, network: &Network, path: &Path) -> bool {
    let mut cmd = std::process::Command::new(exe.display().to_string());
    cmd.args([&network.to_string(), &path.display().to_string()]);

    match cmd.output() {
        Ok(output) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stderr = stderr.trim();
                if !stderr.is_empty() {
                    warn!("verify-block-exe rejected {path:#?}: {stderr}");
                }
            }
            output.status.success()
        }
        Err(e) => {
            error!("verify-block-exe failed to run ({exe:#?}): {e}");
            false
        }
    }
}

/// Checks if the PCB is already present in the witness tree
async fn check_block(state: &Arc<RwLock<IndexerState>>, path: &Path) -> bool {
    let (network, height, state_hash) = extract_network_height_hash(path);
    let state_hash: StateHash = state_hash.into();
    let ro_state = state.read().await;

    // check if the block is already in the witness tree
    if ro_state.diffs_map.contains_key(&state_hash) {
        info!(
            "Block is already present in the witness tree {}-{}-{}",
            network, height, state_hash
        );

        return true;
    }

    false
}

/// Checks if the staking ledger is already present in the witness tree
async fn check_staking_ledger(state: &Arc<RwLock<IndexerState>>, path: &Path) -> bool {
    let (network, epoch, ledger_hash) = extract_network_height_hash(path);
    let ledger_hash: LedgerHash = ledger_hash.into();
    let ro_state = state.read().await;

    // check if the staking ledger is already in the witness tree
    if ro_state
        .staking_ledgers
        .contains(&(epoch, ledger_hash.clone()))
    {
        info!(
            "Staking ledger is already present in the witness tree {}-{}-{}",
            network, epoch, ledger_hash
        );

        return true;
    }

    false
}

/// Fetch new blocks
async fn fetch_new_blocks(
    state: &Arc<RwLock<IndexerState>>,
    blocks_dir: impl AsRef<Path>,
    fetch_new_blocks_exe: impl AsRef<Path>,
) -> anyhow::Result<()> {
    debug!("Fetching new blocks");
    crate::metrics::FETCH_INVOCATIONS.inc();
    let _fetch_timer = crate::metrics::FETCH_SECONDS.start_timer();

    let state = state.read().await;
    let network = state.version.network.clone();
    let new_block_length = state.best_tip_block().blockchain_length + 1;

    let mut cmd = std::process::Command::new(fetch_new_blocks_exe.as_ref().display().to_string());
    let cmd = cmd.args([
        &network.to_string(),
        &new_block_length.to_string(),
        &blocks_dir.as_ref().display().to_string(),
    ]);

    match cmd.output() {
        Ok(output) => {
            let stdout = String::from_utf8(output.stdout)?;
            let stdout = stdout.trim_end();

            if !stdout.is_empty() {
                tracing::info!(network = %network, output = %stdout, "fetch-new-blocks output");
            }

            let stderr = String::from_utf8(output.stderr)?;
            let stderr = stderr.trim_end();

            if !stderr.is_empty() {
                tracing::info!(network = %network, output = %stderr, "fetch-new-blocks stderr");
            }
        }
        Err(e) => {
            crate::metrics::FETCH_FAILURES.inc();
            tracing::error!(
                network = %network,
                error = %e,
                program = %cmd.get_program().to_string_lossy(),
                args = ?cmd.get_args().map(|a| a.to_string_lossy()).collect::<Vec<_>>(),
                "Error fetching new blocks"
            );
        }
    }

    Ok(())
}

/// Recovers missing blocks
async fn recover_missing_blocks(
    state: &Arc<RwLock<IndexerState>>,
    blocks_dir: impl AsRef<Path>,
    missing_block_recovery_exe: impl AsRef<Path>,
    batch_recovery: bool,
) -> anyhow::Result<()> {
    debug!("Running missing block recovery");

    let state = state.read().await;
    let network = state.version.network.clone();
    let missing_parent_lengths = state
        .dangling_branches
        .iter()
        .map(|b| b.root_block().blockchain_length.saturating_sub(1))
        .collect::<HashSet<_>>();

    // exit early if no missing blocks
    if missing_parent_lengths.is_empty() {
        debug!("No missing blocks found");
        return Ok(());
    }

    let run_missing_blocks_recovery = |blockchain_length: u32| {
        let mut cmd =
            std::process::Command::new(missing_block_recovery_exe.as_ref().display().to_string());
        let cmd = cmd.args([
            &network.to_string(),
            &blockchain_length.to_string(),
            &blocks_dir.as_ref().display().to_string(),
        ]);

        match cmd.output() {
            Ok(output) => {
                // non-UTF8 subprocess output must not crash the indexer
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stdout = stdout.trim_end();

                if !stdout.is_empty() {
                    info!("Missing block recovery: {}", stdout);
                }

                let stderr = String::from_utf8_lossy(&output.stderr);
                let stderr = stderr.trim_end();

                if !stderr.is_empty() {
                    info!("Missing block recovery: {}", stderr);
                }
            }
            Err(e) => error!(
                "Error recovery missing block: {}, pgm: {}, args: {:?}",
                e,
                cmd.get_program().to_str().unwrap(),
                cmd.get_args()
                    .map(|arg| arg.to_str().unwrap())
                    .collect::<Vec<_>>()
            ),
        }
    };

    debug!("Getting missing parent blocks of dangling roots");

    if batch_recovery {
        let min_missing_length = missing_parent_lengths.iter().min().cloned();
        let max_missing_length = missing_parent_lengths.iter().max().cloned();

        if let (Some(min), Some(max)) = (min_missing_length, max_missing_length) {
            (min..=max).for_each(run_missing_blocks_recovery)
        }
    } else {
        missing_parent_lengths
            .into_iter()
            .for_each(run_missing_blocks_recovery);
    }

    Ok(())
}

impl GenesisVersion {
    pub fn v1() -> Self {
        use std::str::FromStr;
        let last_vrf_output =
            VrfOutput::from_str(MAINNET_GENESIS_LAST_VRF_OUTPUT).expect("v1 last vrf output");

        Self {
            state_hash: MAINNET_GENESIS_HASH.into(),
            prev_hash: MAINNET_GENESIS_PREV_STATE_HASH.into(),
            last_vrf_output,
            blockchain_lenth: 1,
            global_slot: 0,
        }
    }

    pub fn v2() -> Self {
        use std::str::FromStr;
        let last_vrf_output =
            VrfOutput::from_str(HARDFORK_GENESIS_LAST_VRF_OUTPUT).expect("v2 last vrf output");

        Self {
            last_vrf_output,
            state_hash: HARDFORK_GENESIS_HASH.into(),
            prev_hash: HARDFORK_GENESIS_PREV_STATE_HASH.into(),
            blockchain_lenth: HARDFORK_GENESIS_BLOCKCHAIN_LENGTH,
            global_slot: HARDFORK_GENESIS_GLOBAL_SLOT,
        }
    }

    pub fn mesa() -> Self {
        use std::str::FromStr;
        let last_vrf_output =
            VrfOutput::from_str(MESA_GENESIS_LAST_VRF_OUTPUT).expect("mesa last vrf output");

        Self {
            last_vrf_output,
            state_hash: MESA_GENESIS_HASH.into(),
            prev_hash: MESA_GENESIS_PREV_STATE_HASH.into(),
            blockchain_lenth: MESA_GENESIS_BLOCKCHAIN_LENGTH,
            global_slot: MESA_GENESIS_GLOBAL_SLOT,
        }
    }

    pub fn devnet() -> Self {
        use std::str::FromStr;
        let last_vrf_output =
            VrfOutput::from_str(DEVNET_GENESIS_LAST_VRF_OUTPUT).expect("devnet last vrf output");

        Self {
            last_vrf_output,
            state_hash: DEVNET_GENESIS_HASH.into(),
            prev_hash: DEVNET_GENESIS_PREV_STATE_HASH.into(),
            blockchain_lenth: DEVNET_GENESIS_BLOCKCHAIN_LENGTH,
            global_slot: DEVNET_GENESIS_GLOBAL_SLOT,
        }
    }
}

impl IndexerVersion {
    pub fn v1() -> Self {
        Self {
            network: Network::Mainnet,
            version: PcbVersion::V1,
            chain_id: ChainId::v1(),
            genesis: GenesisVersion::v1(),
        }
    }

    pub fn v2() -> Self {
        let network = Network::Mainnet;

        Self {
            version: PcbVersion::V2(CurrencyEncoding::for_network(&network)),
            network,
            chain_id: ChainId::v2(),
            genesis: GenesisVersion::v2(),
        }
    }

    pub fn mesa() -> Self {
        let network = Network::from("mesa");

        Self {
            version: PcbVersion::V2(CurrencyEncoding::for_network(&network)),
            network,
            chain_id: ChainId::mesa(),
            genesis: GenesisVersion::mesa(),
        }
    }

    pub fn devnet() -> Self {
        let network = Network::Devnet;

        Self {
            version: PcbVersion::V2(CurrencyEncoding::for_network(&network)),
            network,
            chain_id: ChainId::devnet(),
            genesis: GenesisVersion::devnet(),
        }
    }
}

impl From<(ServerArgsJson, PathBuf)> for IndexerConfiguration {
    fn from(value: (ServerArgsJson, PathBuf)) -> Self {
        let genesis_ledger = if value.0.genesis_hash == HARDFORK_GENESIS_HASH {
            GenesisLedger::new_v2().expect("v2 genesis ledger")
        } else {
            GenesisLedger::new_v1().expect("v1 genesis ledger")
        };
        let version = if value.0.genesis_hash == HARDFORK_GENESIS_HASH {
            IndexerVersion::v2()
        } else {
            IndexerVersion::v1()
        };

        Self {
            version,
            genesis_ledger,
            domain_socket_path: value.1,
            blocks_dir: value.0.blocks_dir.map(Into::into),
            staking_ledgers_dir: value.0.staking_ledgers_dir.map(Into::into),
            prune_interval: value.0.prune_interval,
            canonical_threshold: value.0.canonical_threshold,
            canonical_update_threshold: value.0.canonical_update_threshold,
            initialization_mode: InitializationMode::Sync,
            ledger_cadence: value.0.ledger_cadence,
            reporting_freq: value.0.reporting_freq,
            do_not_ingest_orphan_blocks: value.0.do_not_ingest_orphan_blocks,
            fetch_new_blocks_exe: value.0.fetch_new_blocks_exe.map(Into::into),
            fetch_new_blocks_delay: value.0.fetch_new_blocks_delay,
            verify_block_exe: value.0.verify_block_exe.map(Into::into),
            missing_block_recovery_exe: value.0.missing_block_recovery_exe.map(Into::into),
            missing_block_recovery_delay: value.0.missing_block_recovery_delay,
            missing_block_recovery_batch: value.0.missing_block_recovery_batch.unwrap_or_default(),
            blocks_retention_length: value.0.blocks_retention_length,
            check_mode: value.0.check_mode,
        }
    }
}

impl Default for IndexerVersion {
    fn default() -> Self {
        Self::v1()
    }
}

fn log_dirs_msg(blocks_dir: Option<&PathBuf>, staking_ledgers_dir: Option<&PathBuf>) {
    match (blocks_dir, staking_ledgers_dir) {
        (Some(blocks_dir), Some(staking_ledgers_dir)) => info!(
            "Initializing database from blocks in {blocks_dir:#?} and staking ledgers in {staking_ledgers_dir:#?}"
        ),
        (Some(blocks_dir), None) => info!(
            "Initializing database from blocks in {blocks_dir:#?}"
        ),
        (None, Some(staking_ledgers_dir)) => info!(
            "Initializing database from staking ledgers in {staking_ledgers_dir:#?}"
        ),
        (None, None) => info!("Initializing database without blocks and staking ledgers"),
    }
}

#[cfg(test)]
mod tests {
    use super::{catch_block_apply, prunable_block_files};
    use std::{collections::HashSet, fs, path::PathBuf};

    #[test]
    fn catch_block_apply_passes_through_ok_and_err() {
        assert!(matches!(catch_block_apply(|| Ok(true)), Ok(true)));
        assert!(matches!(catch_block_apply(|| Ok(false)), Ok(false)));
        assert!(catch_block_apply(|| anyhow::bail!("pipeline error")).is_err());
    }

    #[test]
    fn catch_block_apply_converts_panic_to_err() {
        // The whole point: a panic in the sync pipeline must become an `Err`
        // (block skipped) instead of unwinding out and killing the process.
        // Silence the default panic hook so the caught panic doesn't spam a
        // backtrace into the test output.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = catch_block_apply(|| panic!("boom in block_pipeline"));
        std::panic::set_hook(prev);

        assert!(
            result.is_err(),
            "a panic must be caught and returned as Err"
        );
    }

    // any 52-char "3N…" string is a structurally valid state hash
    fn hash(seed: char) -> String {
        format!("3N{}", std::iter::repeat(seed).take(50).collect::<String>())
    }

    fn write_block(dir: &std::path::Path, height: u32, seed: char) -> PathBuf {
        let p = dir.join(format!("mainnet-{height}-{}.json", hash(seed)));
        fs::write(&p, b"{}").unwrap();
        p
    }

    #[test]
    fn prunes_only_blocks_strictly_below_floor() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();

        let below_a = write_block(d, 100, 'a');
        let below_b = write_block(d, 289, 'b');
        let at_floor = write_block(d, 500, 'c'); // == floor: kept
        let above = write_block(d, 900, 'd'); // > floor: kept

        // non-block files and subdirs must be ignored
        fs::write(d.join("notes.txt"), b"x").unwrap();
        fs::create_dir(d.join("checkpoints")).unwrap();

        let got: HashSet<PathBuf> = prunable_block_files(d, 500).into_iter().collect();
        let want: HashSet<PathBuf> = [below_a, below_b].into_iter().collect();
        assert_eq!(got, want);

        // the kept files are untouched on disk
        assert!(at_floor.exists() && above.exists());
    }

    #[test]
    fn floor_zero_prunes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        write_block(dir.path(), 0, 'a');
        write_block(dir.path(), 10, 'b');
        assert!(prunable_block_files(dir.path(), 0).is_empty());
    }

    #[test]
    fn missing_dir_yields_empty() {
        assert!(prunable_block_files(std::path::Path::new("/no/such/dir"), 100).is_empty());
    }

    // ---- checkpoint crash-consistency ----
    //
    // The periodic checkpoint writes `<dir>/latest` via a fully-written
    // `.tmp-checkpoint` + atomic `rename`, so `latest` is never partial — the
    // recovery path (`maybe_restore_from_checkpoint`) gates on `latest/CURRENT`.
    // These tests pin those guarantees.

    use crate::store::IndexerStore;
    use super::write_db_checkpoint;

    #[test]
    fn checkpoint_is_complete_and_reopenable() {
        let src = tempfile::tempdir().unwrap();
        let ckpt = tempfile::tempdir().unwrap();

        let store = IndexerStore::new(src.path(), true).unwrap();
        store.database.put(b"crash-key", b"crash-val").unwrap();
        store.database.flush().unwrap();

        let latest = write_db_checkpoint(&store, ckpt.path()).unwrap();

        // A complete DB: CURRENT is the openable marker the restore path checks.
        assert!(
            latest.join("CURRENT").exists(),
            "checkpoint `latest` must be a complete, openable DB"
        );

        // Reopen the checkpoint as an independent store — the data round-trips.
        let restored = IndexerStore::new(&latest, false).unwrap();
        assert_eq!(
            restored.database.get(b"crash-key").unwrap().as_deref(),
            Some(&b"crash-val"[..]),
            "restored checkpoint must contain the data present at checkpoint time"
        );
    }

    #[test]
    fn stale_tmp_checkpoint_is_cleaned_and_next_checkpoint_succeeds() {
        let src = tempfile::tempdir().unwrap();
        let ckpt = tempfile::tempdir().unwrap();

        let store = IndexerStore::new(src.path(), true).unwrap();
        store.database.put(b"k", b"v").unwrap();
        store.database.flush().unwrap();

        // Simulate a crash *during* a previous checkpoint: a partial
        // `.tmp-checkpoint` is left behind. The next checkpoint must remove it
        // and still produce a good `latest` (self-healing).
        let stale = ckpt.path().join(".tmp-checkpoint");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("garbage.sst"), b"partial write").unwrap();

        let latest = write_db_checkpoint(&store, ckpt.path()).unwrap();

        assert!(latest.join("CURRENT").exists());
        assert!(
            !stale.exists(),
            "a stale `.tmp-checkpoint` from a crashed run must be cleaned up"
        );
    }

    #[test]
    fn checkpoint_latest_reflects_newest_write() {
        let src = tempfile::tempdir().unwrap();
        let ckpt = tempfile::tempdir().unwrap();

        let store = IndexerStore::new(src.path(), true).unwrap();
        store.database.put(b"gen", b"1").unwrap();
        store.database.flush().unwrap();
        write_db_checkpoint(&store, ckpt.path()).unwrap();

        // A second checkpoint must atomically replace `latest` with the newer DB.
        store.database.put(b"gen", b"2").unwrap();
        store.database.flush().unwrap();
        let latest = write_db_checkpoint(&store, ckpt.path()).unwrap();

        let restored = IndexerStore::new(&latest, false).unwrap();
        assert_eq!(
            restored.database.get(b"gen").unwrap().as_deref(),
            Some(&b"2"[..]),
            "`latest` must reflect the most recent checkpoint"
        );
    }
}
