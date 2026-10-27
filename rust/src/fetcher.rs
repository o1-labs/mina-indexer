//! In-process precomputed block fetcher
//!
//! Downloads precomputed blocks from a block source (a public GCS bucket, or a
//! local directory) into `blocks_dir`. It never touches the store: the
//! filesystem watcher and the reconcile safety net ingest the files from
//! there, exactly as they ingest a block dropped in by hand.
//!
//! One loop covers the three jobs the external fetch/recovery executables and
//! the first-boot bulk fetch used to do:
//!
//! - **catch-up**: list the source by height range and download in parallel
//!   until the source tip is reached (bounded by [MAX_LOOKAHEAD] so the
//!   download never runs too far ahead of ingest)
//! - **follow**: re-list the range above the best tip every interval
//! - **gap fill**: fetch every block at the missing parent height of each
//!   dangling branch
//!
//! Source objects must already be named `<network>-<height>-<hash>.json` (the
//! `blocks_dir` filename contract), as they are in `mina_network_block_data`.

use crate::{
    block::is_valid_block_file, state::IndexerState,
    utility::functions::extract_network_height_hash,
};
use anyhow::{bail, Context};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, LazyLock},
    time::Duration,
};
use tokio::sync::RwLock;
use tokio_graceful_shutdown::SubsystemHandle;

/// Public GCS endpoint for anonymous reads
pub const GCS_ENDPOINT: &str = "https://storage.googleapis.com";

/// Heights listed per source request
const SPAN: u32 = 250;

/// Max heights downloaded ahead of the best tip before waiting for ingest
const MAX_LOOKAHEAD: u32 = 1000;

/// Attempts for each source request
const ATTEMPTS: u32 = 4;

/// Where blocks are fetched from
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockSource {
    /// Public GCS bucket, read anonymously over the JSON API
    Gcs { endpoint: String, bucket: String },

    /// Local directory holding block files (a mirror, or a test fixture)
    Dir(PathBuf),
}

impl FromStr for BlockSource {
    type Err = anyhow::Error;

    /// `gs://<bucket>` or `file://<dir>`
    fn from_str(s: &str) -> anyhow::Result<Self> {
        if let Some(bucket) = s.strip_prefix("gs://") {
            let bucket = bucket.trim_end_matches('/');
            if bucket.is_empty() || bucket.contains('/') {
                bail!("invalid block source {s:?}: expected gs://<bucket>");
            }

            return Ok(Self::Gcs {
                endpoint: GCS_ENDPOINT.to_string(),
                bucket: bucket.to_string(),
            });
        }

        if let Some(dir) = s.strip_prefix("file://") {
            if dir.is_empty() {
                bail!("invalid block source {s:?}: expected file://<dir>");
            }

            return Ok(Self::Dir(dir.into()));
        }

        bail!("invalid block source {s:?}: expected gs://<bucket> or file://<dir>")
    }
}

impl std::fmt::Display for BlockSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gcs { bucket, .. } => write!(f, "gs://{bucket}"),
            Self::Dir(dir) => write!(f, "file://{}", dir.display()),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GcsListPage {
    #[serde(default)]
    items: Vec<GcsObject>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct GcsObject {
    name: String,
}

impl BlockSource {
    /// Names of the source's block files for `network` with height in
    /// `[from, to)`, sorted by height
    pub fn list(&self, network: &str, from: u32, to: u32) -> anyhow::Result<Vec<(u32, String)>> {
        let mut names = vec![];
        match self {
            Self::Gcs { endpoint, bucket } => {
                // GCS lists in lexicographic order, which matches height order
                // only among heights with the same number of digits
                for (lo, hi) in same_digit_ranges(from, to) {
                    names.extend(gcs_list(endpoint, bucket, network, lo, hi)?);
                }
            }
            Self::Dir(dir) => {
                for entry in fs::read_dir(dir).with_context(|| format!("reading {dir:#?}"))? {
                    if let Some(name) = entry?.file_name().to_str() {
                        names.push(name.to_string());
                    }
                }
            }
        }

        let mut blocks: Vec<_> = names
            .into_iter()
            .filter_map(|name| {
                let height = block_file_height(network, &name)?;
                (from..to).contains(&height).then_some((height, name))
            })
            .collect();
        blocks.sort();
        blocks.dedup();
        Ok(blocks)
    }

    /// Writes the source object `name` to `dest`
    fn download(&self, name: &str, dest: &Path) -> anyhow::Result<()> {
        match self {
            Self::Gcs { endpoint, bucket } => {
                let url = format!("{endpoint}/storage/v1/b/{bucket}/o/{name}");
                let resp = with_retries(|| Ok(agent().get(&url).query("alt", "media").call()?))?;
                let mut file = fs::File::create(dest)?;
                std::io::copy(&mut resp.into_reader(), &mut file)?;
                file.sync_all()?;
            }
            Self::Dir(dir) => {
                fs::copy(dir.join(name), dest)?;
            }
        }

        Ok(())
    }
}

/// One agent for every request, so connections (and TLS sessions) are reused
fn agent() -> &'static ureq::Agent {
    static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(30))
            .timeout(Duration::from_secs(300))
            .build()
    });
    &AGENT
}

fn with_retries<T>(mut f: impl FnMut() -> anyhow::Result<T>) -> anyhow::Result<T> {
    let mut attempt = 1;
    loop {
        match f() {
            Ok(value) => return Ok(value),
            Err(e) if attempt < ATTEMPTS => {
                debug!("Block source request failed (attempt {attempt}/{ATTEMPTS}): {e}");
                std::thread::sleep(Duration::from_secs(2u64.pow(attempt)));
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

fn gcs_list(
    endpoint: &str,
    bucket: &str,
    network: &str,
    from: u32,
    to: u32,
) -> anyhow::Result<Vec<String>> {
    let url = format!("{endpoint}/storage/v1/b/{bucket}/o");
    let prefix = format!("{network}-");
    let start = format!("{network}-{from}-");
    let end = format!("{network}-{to}-");

    let mut names = vec![];
    let mut page_token: Option<String> = None;
    loop {
        let page: GcsListPage = with_retries(|| {
            let mut req = agent()
                .get(&url)
                .query("prefix", &prefix)
                .query("startOffset", &start)
                .query("endOffset", &end)
                .query("fields", "items(name),nextPageToken")
                .query("maxResults", "1000");
            if let Some(token) = page_token.as_deref() {
                req = req.query("pageToken", token);
            }

            Ok(req.call()?.into_json()?)
        })?;

        names.extend(page.items.into_iter().map(|o| o.name));
        match page.next_page_token {
            Some(token) => page_token = Some(token),
            None => return Ok(names),
        }
    }
}

/// Splits `[from, to)` into sub-ranges whose heights all have the same number
/// of decimal digits
fn same_digit_ranges(from: u32, to: u32) -> Vec<(u32, u32)> {
    let mut ranges = vec![];
    let mut lo = from;
    while lo < to {
        let next_power = 10u32.saturating_pow(lo.max(1).ilog10() + 1);
        let hi = to.min(next_power);
        ranges.push((lo, hi));
        if hi == lo {
            break;
        }
        lo = hi;
    }

    ranges
}

/// Height of a well-formed `<network>-<height>-<hash>.json` name, else `None`
fn block_file_height(network: &str, name: &str) -> Option<u32> {
    // a source name becomes a path in blocks_dir: reject separators, then
    // malformed names
    if name.contains(['/', '\\']) || !is_valid_block_file(name) {
        return None;
    }

    let (net, height, _) = extract_network_height_hash(Path::new(name));
    (net == network).then_some(height)
}

pub struct BlockFetcher {
    pub source: BlockSource,
    pub blocks_dir: PathBuf,
    pub interval: Duration,
    pub workers: usize,
}

impl BlockFetcher {
    /// Fetches blocks until shutdown. Source errors are logged and retried on
    /// the next cycle; they never stop the indexer.
    pub async fn run(
        self,
        subsys: SubsystemHandle,
        state: Arc<RwLock<IndexerState>>,
    ) -> anyhow::Result<()> {
        let fetcher = Arc::new(self);
        let (network, min_height) = {
            let state = state.read().await;
            (
                state.version.network.to_string(),
                state.version.genesis.blockchain_lenth + 1,
            )
        };

        info!(
            "Fetching {network} blocks from {} into {:#?} every {:?}",
            fetcher.source, fetcher.blocks_dir, fetcher.interval
        );

        // part files of downloads cut short by a previous shutdown
        if let Ok(entries) = fs::read_dir(&fetcher.blocks_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with('.') && name.ends_with(".part") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }

        let mut cursor = min_height;
        loop {
            crate::metrics::FETCH_INVOCATIONS.inc();
            let timer = crate::metrics::FETCH_SECONDS.start_timer();
            let (tip, missing) = {
                let state = state.read().await;
                let missing: BTreeSet<u32> = state
                    .dangling_branches
                    .iter()
                    .map(|b| b.root_block().blockchain_length.saturating_sub(1))
                    .filter(|h| *h >= min_height)
                    .collect();
                (state.best_tip_block().blockchain_length, missing)
            };

            // gap fill: every block at each missing parent height
            for height in missing {
                let fetched = tokio::select! {
                    _ = subsys.on_shutdown_requested() => return Ok(()),
                    fetched = fetcher.fetch_range(&network, height, height + 1) => fetched,
                };
                if let Err(e) = fetched {
                    crate::metrics::FETCH_FAILURES.inc();
                    warn!("Block fetch failed at missing height {height}: {e:#}");
                }
            }

            // catch-up & follow: continue while each listed range is full;
            // re-list the last, partial range on the next cycle to pick up
            // blocks (incl. late forks) published since
            cursor = cursor.max(tip + 1);
            while cursor <= tip + MAX_LOOKAHEAD {
                let fetched = tokio::select! {
                    _ = subsys.on_shutdown_requested() => return Ok(()),
                    fetched = fetcher.fetch_range(&network, cursor, cursor + SPAN) => fetched,
                };
                match fetched {
                    Ok(Some(max_height)) if max_height + 1 >= cursor + SPAN => cursor += SPAN,
                    Ok(_) => break,
                    Err(e) => {
                        crate::metrics::FETCH_FAILURES.inc();
                        warn!("Block fetch failed from height {cursor}: {e:#}");
                        break;
                    }
                }
            }
            drop(timer);

            tokio::select! {
                _ = subsys.on_shutdown_requested() => return Ok(()),
                _ = tokio::time::sleep(fetcher.interval) => {}
            }
        }
    }

    /// Downloads the source's blocks with height in `[from, to)` that are
    /// absent from `blocks_dir`. Returns the max height listed.
    async fn fetch_range(
        self: &Arc<Self>,
        network: &str,
        from: u32,
        to: u32,
    ) -> anyhow::Result<Option<u32>> {
        let listing = {
            let this = self.clone();
            let network = network.to_string();
            tokio::task::spawn_blocking(move || this.source.list(&network, from, to)).await??
        };

        let max_height = listing.last().map(|(height, _)| *height);
        let wanted: Vec<_> = listing
            .into_iter()
            .filter(|(_, name)| !self.blocks_dir.join(name).exists())
            .collect();

        if wanted.is_empty() {
            return Ok(max_height);
        }
        debug!("Downloading {} blocks in [{from}, {to})", wanted.len());

        // download to hidden part files, up to `workers` at a time in height
        // order; publish each with an atomic rename in that same order, so the
        // watcher only ever sees complete files, parent-first
        let mut queue = wanted.into_iter();
        let mut in_flight = VecDeque::new();
        let mut published = 0;
        loop {
            while in_flight.len() < self.workers.max(1) {
                let Some((_, name)) = queue.next() else {
                    break;
                };
                let this = self.clone();
                let part = self.blocks_dir.join(format!(".{name}.part"));
                let download = {
                    let (name, part) = (name.clone(), part.clone());
                    tokio::task::spawn_blocking(move || this.source.download(&name, &part))
                };
                in_flight.push_back((name, part, download));
            }

            let Some((name, part, download)) = in_flight.pop_front() else {
                break;
            };
            match download.await? {
                Ok(()) => {
                    fs::rename(&part, self.blocks_dir.join(&name))?;
                    crate::metrics::FETCH_BLOCKS_DOWNLOADED.inc();
                    published += 1;
                }
                Err(e) => {
                    let _ = fs::remove_file(&part);
                    crate::metrics::FETCH_FAILURES.inc();
                    error!("Failed to download block {name}: {e:#}");
                }
            }
        }

        info!("Fetched {published} {network} blocks in [{from}, {to})");
        Ok(max_height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };

    const HASH_A: &str = "3NKGgTk7en3347KH81yDra876GPAUSoSePrfVKPmwR1KHfMpvJC5";
    const HASH_B: &str = "3NKZ6DTHiMtuaeP3tJq2xe4uujVRnGT9FX1rBiZY521uNToSppUZ";

    #[test]
    fn parses_block_sources() -> anyhow::Result<()> {
        assert_eq!(
            "gs://mina_network_block_data".parse::<BlockSource>()?,
            BlockSource::Gcs {
                endpoint: GCS_ENDPOINT.into(),
                bucket: "mina_network_block_data".into()
            }
        );
        assert_eq!(
            "file:///data/mirror".parse::<BlockSource>()?,
            BlockSource::Dir("/data/mirror".into())
        );

        for bad in ["", "gs://", "gs://a/b", "file://", "https://x", "/data"] {
            assert!(bad.parse::<BlockSource>().is_err(), "{bad:?}");
        }

        Ok(())
    }

    #[test]
    fn splits_ranges_at_digit_boundaries() {
        assert_eq!(same_digit_ranges(5, 8), vec![(5, 8)]);
        assert_eq!(same_digit_ranges(0, 12), vec![(0, 10), (10, 12)]);
        assert_eq!(
            same_digit_ranges(999_990, 1_000_010),
            vec![(999_990, 1_000_000), (1_000_000, 1_000_010)]
        );
        assert_eq!(same_digit_ranges(7, 7), vec![]);
    }

    #[test]
    fn block_file_height_filters_names() {
        let name = format!("devnet-545435-{HASH_A}.json");
        assert_eq!(block_file_height("devnet", &name), Some(545435));
        assert_eq!(block_file_height("mainnet", &name), None);
        assert_eq!(block_file_height("devnet", "devnet-545435-x.json"), None);
        assert_eq!(block_file_height("devnet", &format!("../{name}")), None);
    }

    #[test]
    fn dir_source_lists_range_in_height_order() -> anyhow::Result<()> {
        let src = tempfile::tempdir()?;
        for name in [
            format!("mainnet-12-{HASH_A}.json"),
            format!("mainnet-9-{HASH_A}.json"),
            format!("mainnet-10-{HASH_B}.json"),
            format!("mainnet-10-{HASH_A}.json"),
            format!("devnet-10-{HASH_A}.json"),
            "notes.txt".to_string(),
        ] {
            fs::write(src.path().join(name), "{}")?;
        }

        let source = BlockSource::Dir(src.path().into());
        let listed = source.list("mainnet", 10, 12)?;
        assert_eq!(
            listed,
            vec![
                (10, format!("mainnet-10-{HASH_A}.json")),
                (10, format!("mainnet-10-{HASH_B}.json")),
            ]
        );

        Ok(())
    }

    /// Serves canned responses to `n` requests; returns the endpoint and the
    /// received request lines
    fn fake_gcs(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = vec![];
            for body in responses {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                requests.push(line.trim_end().to_string());
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header == "\r\n" || header.is_empty() {
                        break;
                    }
                }
                let mut stream = stream;
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            requests
        });
        (endpoint, handle)
    }

    #[test]
    fn gcs_source_pages_and_downloads() -> anyhow::Result<()> {
        let a = format!("devnet-545435-{HASH_A}.json");
        let b = format!("devnet-545436-{HASH_B}.json");
        let (endpoint, server) = fake_gcs(vec![
            format!(r#"{{"items":[{{"name":"{a}"}}],"nextPageToken":"tok/1="}}"#),
            format!(r#"{{"items":[{{"name":"{b}"}},{{"name":"devnet-545999-{HASH_A}.json"}}]}}"#),
            r#"{"block":true}"#.to_string(),
        ]);
        let source = BlockSource::Gcs {
            endpoint,
            bucket: "blocks".into(),
        };

        let listed = source.list("devnet", 545435, 545437)?;
        assert_eq!(listed, vec![(545435, a.clone()), (545436, b)]);

        let dir = tempfile::tempdir()?;
        let dest = dir.path().join(&a);
        source.download(&a, &dest)?;
        assert_eq!(fs::read_to_string(dest)?, r#"{"block":true}"#);

        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /storage/v1/b/blocks/o?"));
        assert!(requests[0].contains("startOffset=devnet-545435-"));
        assert!(requests[0].contains("endOffset=devnet-545437-"));
        assert!(requests[1].contains("pageToken=tok%2F1%3D"));
        assert_eq!(
            requests[2],
            format!("GET /storage/v1/b/blocks/o/{a}?alt=media HTTP/1.1")
        );

        Ok(())
    }

    #[tokio::test]
    async fn fetch_range_publishes_only_new_blocks() -> anyhow::Result<()> {
        let src = tempfile::tempdir()?;
        let dst = tempfile::tempdir()?;
        let a = format!("mainnet-10-{HASH_A}.json");
        let b = format!("mainnet-11-{HASH_B}.json");
        fs::write(src.path().join(&a), "a")?;
        fs::write(src.path().join(&b), "b")?;
        fs::write(dst.path().join(&a), "already here")?;

        let fetcher = Arc::new(BlockFetcher {
            source: BlockSource::Dir(src.path().into()),
            blocks_dir: dst.path().into(),
            interval: Duration::from_secs(1),
            workers: 2,
        });

        assert_eq!(fetcher.fetch_range("mainnet", 10, 20).await?, Some(11));
        assert_eq!(fs::read_to_string(dst.path().join(&a))?, "already here");
        assert_eq!(fs::read_to_string(dst.path().join(&b))?, "b");

        // no part files left behind
        let leftovers: Vec<_> = fs::read_dir(dst.path())?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
            .collect();
        assert!(leftovers.is_empty());

        assert_eq!(fetcher.fetch_range("mainnet", 12, 20).await?, None);
        Ok(())
    }
}
