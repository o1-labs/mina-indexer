//! mesa-mut-blocks
//!
//! A small companion app that *connects to* the precomputed blocks of the
//! `mesa-mut` (a.k.a. `hetzner-pre-mesa-1`) Mina network and decodes them with
//! the Mina Indexer's own block types.
//!
//! `mesa-mut` is a post-hardfork (PCB **V2**) network whose genesis ledger and
//! genesis block are not the mainnet ones, so the full indexer's
//! genesis-coupled ingestion pipeline cannot bootstrap it directly. Parsing and
//! serving individual precomputed blocks, however, only needs the V2 decoder —
//! which this app exercises via `PrecomputedBlock::parse_file(.., PcbVersion::V2)`.
//!
//! Fetch the blocks first with `ops/mesa-mut/fetch-blocks.sh`, then:
//!
//!   # human-readable report over a directory of blocks
//!   mesa-mut-blocks report --blocks-dir ./mesa-mut-blocks
//!
//!   # serve the decoded blocks as JSON over HTTP
//!   mesa-mut-blocks serve --blocks-dir ./mesa-mut-blocks --port 8080
//!     GET /                      -> index / summary
//!     GET /blocks                -> all block summaries (JSON)
//!     GET /blocks/{height}       -> summaries at a height (JSON)
//!     GET /blocks/{height}/raw   -> raw precomputed block JSON

use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use anyhow::Context;
use clap::{Parser, Subcommand};
use mina_indexer::block::precomputed::{PcbVersion, PrecomputedBlock};
use mina_indexer::mina_blocks::v2::staged_ledger_diff::{SignedCommandData, ZkappCommandData};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Parser, Debug)]
#[command(name = "mesa-mut-blocks", version, about = "Connect to and decode mesa-mut precomputed blocks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Parse a directory of mesa-mut precomputed blocks and print a report
    Report {
        /// Directory of `mesa-<height>-<hash>.json` precomputed blocks
        #[arg(long)]
        blocks_dir: PathBuf,

        /// Emit the report as JSON instead of a table
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// Diagnose why a precomputed block fails to decode: for each user command,
    /// deserialize its payload into the indexer's command structs with full
    /// serde path tracking, and report the exact failing field path.
    Diag {
        /// Path to a single `mesa-<height>-<hash>.json` precomputed block
        #[arg(long)]
        block: PathBuf,

        /// Stop after this many failing commands (0 = report all)
        #[arg(long, default_value_t = 5)]
        max_failures: usize,
    },

    /// Serve the decoded mesa-mut precomputed blocks as JSON over HTTP
    Serve {
        /// Directory of `mesa-<height>-<hash>.json` precomputed blocks
        #[arg(long)]
        blocks_dir: PathBuf,

        /// Host to bind
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port to bind
        #[arg(long, default_value_t = 8080)]
        port: u16,
    },
}

/// A compact, serializable view of a single precomputed block.
#[derive(Debug, Clone, Serialize)]
struct BlockSummary {
    network: String,
    blockchain_length: u32,
    global_slot_since_genesis: u32,
    state_hash: String,
    previous_state_hash: String,
    genesis_state_hash: String,
    staged_ledger_hash: String,
    block_creator: String,
    coinbase_receiver: String,
    scheduled_time: String,
    num_user_commands: usize,
    num_zkapp_commands: usize,
    num_completed_works: usize,
    /// File the block was decoded from
    file: String,
}

impl BlockSummary {
    fn from_block(pcb: &PrecomputedBlock, file: &Path) -> Self {
        Self {
            network: pcb.network().to_string(),
            blockchain_length: pcb.blockchain_length(),
            global_slot_since_genesis: pcb.global_slot_since_genesis(),
            state_hash: pcb.state_hash().to_string(),
            previous_state_hash: pcb.previous_state_hash().to_string(),
            genesis_state_hash: pcb.genesis_state_hash().to_string(),
            staged_ledger_hash: pcb.staged_ledger_hash().to_string(),
            block_creator: pcb.block_creator().to_string(),
            coinbase_receiver: pcb.coinbase_receiver().to_string(),
            scheduled_time: pcb.scheduled_time(),
            num_user_commands: pcb.commands().len(),
            num_zkapp_commands: pcb.zkapp_commands().len(),
            num_completed_works: pcb.completed_works().len(),
            file: file
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }
}

/// Decode every `*.json` precomputed block in `blocks_dir` (as PCB V2),
/// returning the summaries sorted by `(blockchain_length, state_hash)`.
///
/// Blocks that fail to decode are reported on stderr and skipped, so a single
/// malformed file does not abort the whole run.
fn load_summaries(blocks_dir: &Path) -> anyhow::Result<Vec<BlockSummary>> {
    let pattern = blocks_dir.join("*.json");
    let pattern = pattern
        .to_str()
        .context("blocks-dir path is not valid UTF-8")?;

    let mut summaries = Vec::new();
    let mut failures = 0usize;

    for entry in glob::glob(pattern)? {
        let path = entry?;
        match PrecomputedBlock::parse_file(&path, PcbVersion::V2) {
            Ok(pcb) => summaries.push(BlockSummary::from_block(&pcb, &path)),
            Err(e) => {
                failures += 1;
                eprintln!("WARN: failed to decode {}: {e}", path.display());
            }
        }
    }

    if summaries.is_empty() {
        anyhow::bail!(
            "no decodable precomputed blocks found in {} ({failures} failed)",
            blocks_dir.display()
        );
    }

    summaries.sort_by(|a, b| {
        a.blockchain_length
            .cmp(&b.blockchain_length)
            .then_with(|| a.state_hash.cmp(&b.state_hash))
    });

    if failures > 0 {
        eprintln!("WARN: {failures} block file(s) could not be decoded");
    }

    Ok(summaries)
}

fn run_report(blocks_dir: &Path, json: bool) -> anyhow::Result<()> {
    let summaries = load_summaries(blocks_dir)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&summaries)?);
        return Ok(());
    }

    let min_h = summaries.first().map(|s| s.blockchain_length).unwrap_or(0);
    let max_h = summaries.last().map(|s| s.blockchain_length).unwrap_or(0);
    let total_user: usize = summaries.iter().map(|s| s.num_user_commands).sum();
    let total_zkapp: usize = summaries.iter().map(|s| s.num_zkapp_commands).sum();
    let total_snark: usize = summaries.iter().map(|s| s.num_completed_works).sum();
    let genesis = &summaries[0].genesis_state_hash;
    let network = &summaries[0].network;

    println!("mesa-mut precomputed blocks — {}", blocks_dir.display());
    println!("  network (from filenames): {network}");
    println!("  genesis state hash:       {genesis}");
    println!("  blocks decoded:           {}", summaries.len());
    println!("  height range:             {min_h}..={max_h}");
    println!("  total user commands:      {total_user}");
    println!("  total zkapp commands:     {total_zkapp}");
    println!("  total completed SNARKs:   {total_snark}");
    println!();
    println!(
        "{:>8}  {:>6}  {:<52}  {:>5}  {:>5}  {:>6}",
        "height", "slot", "state_hash", "cmds", "zkapp", "snarks"
    );
    for s in &summaries {
        println!(
            "{:>8}  {:>6}  {:<52}  {:>5}  {:>5}  {:>6}",
            s.blockchain_length,
            s.global_slot_since_genesis,
            s.state_hash,
            s.num_user_commands,
            s.num_zkapp_commands,
            s.num_completed_works,
        );
    }

    Ok(())
}

// ----------------------------- diag ------------------------------

/// Walk a block's user commands and, for each, deserialize its payload into the
/// indexer's command structs with serde path tracking. Reports the exact field
/// path where decoding fails — which `PrecomputedBlock::parse_file` cannot,
/// because `UserCommandData` is an untagged enum that discards inner paths.
fn run_diag(block: &Path, max_failures: usize) -> anyhow::Result<()> {
    let bytes = std::fs::read(block).with_context(|| format!("reading {}", block.display()))?;

    // Capture ONLY the per-diff `commands` arrays, as Values. We must skip the
    // proof-bearing fields (`protocol_state_proof`, and `completed_works` whose
    // `sok_digest` embeds raw binary bytes): serde_json tolerates those when
    // *skipping* a string, but rejects them when building a `Value`. This
    // mirrors how the indexer's typed `BlockFileV2`/`StagedLedgerDiff` skips
    // them, so we reach the commands (which are clean base64) intact.
    #[derive(serde::Deserialize)]
    struct BlockShell {
        data: DataShell,
    }
    #[derive(serde::Deserialize)]
    struct DataShell {
        staged_ledger_diff: SldShell,
    }
    #[derive(serde::Deserialize)]
    struct SldShell {
        diff: Vec<Option<DiffPartShell>>,
    }
    #[derive(serde::Deserialize)]
    struct DiffPartShell {
        #[serde(default)]
        commands: Vec<serde_json::Value>,
    }

    let shell: BlockShell =
        serde_json::from_slice(&bytes).context("block is not valid JSON (staged_ledger_diff)")?;
    let parts: Vec<Vec<serde_json::Value>> = shell
        .data
        .staged_ledger_diff
        .diff
        .into_iter()
        .map(|p| p.map(|p| p.commands).unwrap_or_default())
        .collect();
    let diff: Vec<(usize, &serde_json::Value)> = parts
        .iter()
        .enumerate()
        .flat_map(|(pi, cmds)| cmds.iter().map(move |c| (pi, c)))
        .collect();

    let mut checked = 0usize;
    let mut failures = 0usize;

    for (cmd_idx, (part_idx, cmd)) in diff.iter().enumerate() {
        let Some(data) = cmd.get("data").and_then(|d| d.as_array()) else {
            continue;
        };
        let kind = data.first().and_then(|k| k.as_str()).unwrap_or("?");
        let Some(payload) = data.get(1) else { continue };
        checked += 1;

        let err = match kind {
            "Zkapp_command" => {
                serde_path_to_error::deserialize::<_, ZkappCommandData>(payload).err()
            }
            "Signed_command" => {
                serde_path_to_error::deserialize::<_, SignedCommandData>(payload).err()
            }
            _ => None,
        };

        if let Some(e) = err {
            failures += 1;
            println!("FAIL  diff[{part_idx}] command #{cmd_idx}  kind={kind}");
            println!("  path:  {}", e.path());
            println!("  error: {}", e.inner());
            if max_failures != 0 && failures >= max_failures {
                println!("\nstopping after {failures} failure(s) (checked {checked} commands)");
                return Ok(());
            }
        }
    }

    println!(
        "\nchecked {checked} commands, {failures} failed in {}",
        block.display()
    );
    Ok(())
}

// ----------------------------- serve -----------------------------

struct AppState {
    blocks_dir: PathBuf,
    summaries: Vec<BlockSummary>,
    /// height -> file names at that height
    by_height: BTreeMap<u32, Vec<String>>,
}

async fn index(data: web::Data<Arc<AppState>>) -> impl Responder {
    let min_h = data.summaries.first().map(|s| s.blockchain_length);
    let max_h = data.summaries.last().map(|s| s.blockchain_length);
    HttpResponse::Ok().json(serde_json::json!({
        "app": "mesa-mut-blocks",
        "blocks_dir": data.blocks_dir.display().to_string(),
        "network": data.summaries.first().map(|s| s.network.clone()),
        "genesis_state_hash": data.summaries.first().map(|s| s.genesis_state_hash.clone()),
        "blocks": data.summaries.len(),
        "min_height": min_h,
        "max_height": max_h,
        "endpoints": [
            "GET /blocks",
            "GET /blocks/{height}",
            "GET /blocks/{height}/raw",
        ],
    }))
}

async fn all_blocks(data: web::Data<Arc<AppState>>) -> impl Responder {
    HttpResponse::Ok().json(&data.summaries)
}

async fn blocks_at_height(
    data: web::Data<Arc<AppState>>,
    path: web::Path<u32>,
) -> impl Responder {
    let height = path.into_inner();
    let matches: Vec<&BlockSummary> = data
        .summaries
        .iter()
        .filter(|s| s.blockchain_length == height)
        .collect();
    if matches.is_empty() {
        return HttpResponse::NotFound().json(serde_json::json!({
            "error": format!("no block at height {height}"),
        }));
    }
    HttpResponse::Ok().json(matches)
}

async fn block_raw(
    data: web::Data<Arc<AppState>>,
    path: web::Path<u32>,
) -> impl Responder {
    let height = path.into_inner();
    let Some(files) = data.by_height.get(&height) else {
        return HttpResponse::NotFound().json(serde_json::json!({
            "error": format!("no block at height {height}"),
        }));
    };
    let Some(file) = files.first() else {
        return HttpResponse::NotFound().finish();
    };
    match std::fs::read_to_string(data.blocks_dir.join(file)) {
        Ok(contents) => HttpResponse::Ok()
            .content_type("application/json")
            .body(contents),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": e.to_string(),
        })),
    }
}

async fn run_serve(blocks_dir: PathBuf, host: String, port: u16) -> anyhow::Result<()> {
    let summaries = load_summaries(&blocks_dir)?;

    let mut by_height: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for s in &summaries {
        by_height
            .entry(s.blockchain_length)
            .or_default()
            .push(s.file.clone());
    }

    let state = Arc::new(AppState {
        blocks_dir,
        summaries,
        by_height,
    });

    println!(
        "Serving {} mesa-mut blocks on http://{host}:{port}",
        state.summaries.len()
    );

    let data = web::Data::new(state);
    HttpServer::new(move || {
        App::new()
            .app_data(data.clone())
            .route("/", web::get().to(index))
            .route("/blocks", web::get().to(all_blocks))
            .route("/blocks/{height}", web::get().to(blocks_at_height))
            .route("/blocks/{height}/raw", web::get().to(block_raw))
    })
    .bind((host.as_str(), port))?
    .run()
    .await?;

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Report { blocks_dir, json } => run_report(&blocks_dir, json),
        Command::Diag {
            block,
            max_failures,
        } => run_diag(&block, max_failures),
        Command::Serve {
            blocks_dir,
            host,
            port,
        } => run_serve(blocks_dir, host, port).await,
    }
}
