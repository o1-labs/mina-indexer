//! Daily consistency check: the mina-indexer GraphQL API vs the canonical mina
//! archive-node API, over the block-height range both serve.
//!
//! The indexer is meant to be a drop-in for the archive-node `blocks` query, so
//! for every canonical block both serve we assert they agree on:
//!   - block identity      (`stateHash`, `creator`)
//!   - transaction counts  (payments/delegations and zkApp commands)
//!   - signed-command content, joined by `(sender, nonce)`: kind, amount, fee,
//!     receiver
//!   - signed-command transaction hashes (hard failure — the v2 hash is fixed)
//!
//! Targets the archive-compatible indexer schema: `userCommands` are
//! payments/delegations and zkApp commands live under `zkappCommands` (same as
//! the archive). The only normalization left is `kind` casing (archive
//! lower-case `payment`, indexer upper-case); amount/fee are parsed leniently
//! (both sides now return strings).
//!
//! This is network-bound and `#[ignore]`d; the scheduled job runs it with
//! `--ignored`. Everything is overridable via env so it can target other
//! networks too:
//!
//! - `INDEXER_GQL_URL` / `ARCHIVE_GQL_URL`: the two endpoints.
//! - `COMPARE_MIN_HEIGHT`: low end of the range (default: the indexer's min).
//! - `COMPARE_TIP_LAG`: blocks to trim below the shared tip (default 5).
//!
//! zkApp-command hashes are not compared yet (a separate `Zkapp_command` hasher
//! is still to be ported) — they're covered by the count check instead.

use anyhow::{anyhow, Context};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

const DEFAULT_INDEXER: &str = "https://devnet-indexer.gcp.o1test.net/graphql";
const DEFAULT_ARCHIVE: &str = "https://devnet-archive-node-api.gcp.o1test.net/";

/// Heights pulled per GraphQL request.
const PAGE: u32 = 1000;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_u32(key: &str) -> Option<u32> {
    std::env::var(key).ok().and_then(|v| v.parse().ok())
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(120))
        .build()
}

fn gql(agent: &ureq::Agent, url: &str, query: &str) -> anyhow::Result<Value> {
    let resp: Value = agent
        .post(url)
        .set("Content-Type", "application/json")
        .send_json(json!({ "query": query }))
        .map_err(|e| anyhow!("request to {url} failed: {e}"))?
        .into_json()
        .with_context(|| format!("decoding response from {url}"))?;
    if let Some(errs) = resp.get("errors") {
        anyhow::bail!("GraphQL errors from {url}: {errs}");
    }
    Ok(resp.get("data").cloned().unwrap_or(Value::Null))
}

/// Empty-slice fallback so missing/null arrays iterate cleanly.
fn arr(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}

/// `amount`/`fee`/`nonce` come back as either JSON numbers (indexer) or strings
/// (archive).
fn as_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn norm_kind(k: &str) -> String {
    match k.to_ascii_uppercase().as_str() {
        "DELEGATION" => "STAKE_DELEGATION".to_string(),
        other => other.to_string(),
    }
}

/// A signed command (payment or delegation), in a form comparable across APIs.
/// The `hash` is held separately from the content so a hash-only divergence is
/// distinguishable from a content divergence.
#[derive(Debug, PartialEq, Eq)]
struct Signed {
    kind: String,
    amount: u64,
    fee: u64,
    receiver: String,
}

#[derive(Default)]
struct Block {
    state_hash: String,
    creator: String,
    /// `(sender, nonce)` -> (content, tx-hash)
    signed: BTreeMap<(String, u64), (Signed, String)>,
    zkapp_count: usize,
}

fn tip(agent: &ureq::Agent, url: &str) -> anyhow::Result<u32> {
    let d = gql(
        agent,
        url,
        "{ blocks(query: {canonical: true}, limit: 1, sortBy: BLOCKHEIGHT_DESC) { blockHeight } }",
    )?;
    Ok(d["blocks"][0]["blockHeight"]
        .as_u64()
        .context("no tip block")? as u32)
}

fn min_height(agent: &ureq::Agent, url: &str) -> anyhow::Result<u32> {
    let d = gql(
        agent,
        url,
        "{ blocks(query: {canonical: true}, limit: 1, sortBy: BLOCKHEIGHT_ASC) { blockHeight } }",
    )?;
    Ok(d["blocks"][0]["blockHeight"]
        .as_u64()
        .context("no min block")? as u32)
}

fn fetch_archive(
    agent: &ureq::Agent,
    url: &str,
    lo: u32,
    hi: u32,
) -> anyhow::Result<BTreeMap<u32, Block>> {
    let query = format!(
        "{{ blocks(query: {{blockHeight_gte: {lo}, blockHeight_lt: {hi}, canonical: true}}, \
         limit: {PAGE}, sortBy: BLOCKHEIGHT_ASC) {{ blockHeight stateHash creator \
         transactions {{ userCommands {{ hash kind from to amount fee nonce }} \
         zkappCommands {{ hash }} }} }} }}"
    );
    let data = gql(agent, url, &query)?;
    let mut out = BTreeMap::new();
    for b in arr(&data["blocks"]) {
        let height = b["blockHeight"].as_u64().context("archive block height")? as u32;
        let mut blk = Block {
            state_hash: b["stateHash"].as_str().unwrap_or_default().to_string(),
            creator: b["creator"].as_str().unwrap_or_default().to_string(),
            ..Default::default()
        };
        let tx = &b["transactions"];
        for u in arr(&tx["userCommands"]) {
            let sender = u["from"].as_str().unwrap_or_default().to_string();
            let nonce = as_u64(&u["nonce"]).unwrap_or_default();
            blk.signed.insert(
                (sender, nonce),
                (
                    Signed {
                        kind: norm_kind(u["kind"].as_str().unwrap_or_default()),
                        amount: as_u64(&u["amount"]).unwrap_or_default(),
                        fee: as_u64(&u["fee"]).unwrap_or_default(),
                        receiver: u["to"].as_str().unwrap_or_default().to_string(),
                    },
                    u["hash"].as_str().unwrap_or_default().to_string(),
                ),
            );
        }
        blk.zkapp_count = arr(&tx["zkappCommands"]).len();
        out.insert(height, blk);
    }
    Ok(out)
}

fn fetch_indexer(
    agent: &ureq::Agent,
    url: &str,
    lo: u32,
    hi: u32,
) -> anyhow::Result<BTreeMap<u32, Block>> {
    let query = format!(
        "{{ blocks(query: {{blockHeight_gte: {lo}, blockHeight_lt: {hi}, canonical: true}}, \
         limit: {PAGE}, sortBy: BLOCKHEIGHT_ASC) {{ blockHeight stateHash creator \
         transactions {{ userCommands {{ hash kind sender amount fee nonce \
         receiver_account {{ publicKey }} }} zkappCommands {{ hash }} }} }} }}"
    );
    let data = gql(agent, url, &query)?;
    let mut out = BTreeMap::new();
    for b in arr(&data["blocks"]) {
        let height = b["blockHeight"].as_u64().context("indexer block height")? as u32;
        let mut blk = Block {
            state_hash: b["stateHash"].as_str().unwrap_or_default().to_string(),
            creator: b["creator"].as_str().unwrap_or_default().to_string(),
            ..Default::default()
        };
        let tx = &b["transactions"];
        // The archive-compatible indexer splits zkApp commands into a separate
        // `zkappCommands` field (same as the archive), so `userCommands` holds
        // only payments/delegations.
        blk.zkapp_count = arr(&tx["zkappCommands"]).len();
        for u in arr(&tx["userCommands"]) {
            let kind = norm_kind(u["kind"].as_str().unwrap_or_default());
            let sender = u["sender"].as_str().unwrap_or_default().to_string();
            let nonce = as_u64(&u["nonce"]).unwrap_or_default();
            blk.signed.insert(
                (sender, nonce),
                (
                    Signed {
                        kind,
                        amount: as_u64(&u["amount"]).unwrap_or_default(),
                        fee: as_u64(&u["fee"]).unwrap_or_default(),
                        receiver: u["receiver_account"]["publicKey"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                    },
                    u["hash"].as_str().unwrap_or_default().to_string(),
                ),
            );
        }
        out.insert(height, blk);
    }
    Ok(out)
}

/// Appends any hard discrepancies for one height; returns the tx-hash mismatch
/// count (asserted by the caller — signed-command hashes must match).
fn compare_block(height: u32, a: &Block, i: &Block, hard: &mut Vec<String>) -> usize {
    if a.state_hash != i.state_hash {
        hard.push(format!(
            "h{height}: stateHash archive={} indexer={}",
            a.state_hash, i.state_hash
        ));
    }
    if a.creator != i.creator {
        hard.push(format!(
            "h{height}: creator archive={} indexer={}",
            a.creator, i.creator
        ));
    }
    if a.zkapp_count != i.zkapp_count {
        hard.push(format!(
            "h{height}: zkApp-command count archive={} indexer={}",
            a.zkapp_count, i.zkapp_count
        ));
    }

    let akeys: BTreeSet<_> = a.signed.keys().collect();
    let ikeys: BTreeSet<_> = i.signed.keys().collect();
    for k in akeys.difference(&ikeys) {
        hard.push(format!("h{height}: signed command {k:?} only in archive"));
    }
    for k in ikeys.difference(&akeys) {
        hard.push(format!("h{height}: signed command {k:?} only in indexer"));
    }

    let mut hash_mismatches = 0;
    for k in akeys.intersection(&ikeys) {
        let (ac, ah) = &a.signed[*k];
        let (ic, ih) = &i.signed[*k];
        if ac != ic {
            hard.push(format!(
                "h{height}: signed command {k:?} content archive={ac:?} indexer={ic:?}"
            ));
        }
        if ah != ih {
            hash_mismatches += 1;
        }
    }
    hash_mismatches
}

#[test]
#[ignore = "hits live devnet endpoints; run via the scheduled compare job with --ignored"]
fn devnet_indexer_matches_archive() -> anyhow::Result<()> {
    let indexer = env_or("INDEXER_GQL_URL", DEFAULT_INDEXER);
    let archive = env_or("ARCHIVE_GQL_URL", DEFAULT_ARCHIVE);
    let agent = agent();

    // Only compare heights both serve, and trim a few below the shared tip to
    // avoid near-tip canonicity churn / archive lag.
    let archive_tip = tip(&agent, &archive)?;
    let indexer_tip = tip(&agent, &indexer)?;
    let lag = env_u32("COMPARE_TIP_LAG").unwrap_or(5);
    let hi = archive_tip.min(indexer_tip).saturating_sub(lag);
    let lo = match env_u32("COMPARE_MIN_HEIGHT") {
        Some(h) => h,
        None => min_height(&agent, &indexer)?,
    };
    assert!(
        lo <= hi,
        "no overlapping range: lo={lo} hi={hi} (archive_tip={archive_tip} indexer_tip={indexer_tip})"
    );
    eprintln!(
        "comparing canonical blocks [{lo}, {hi}] (archive_tip={archive_tip} indexer_tip={indexer_tip})"
    );

    let mut hard: Vec<String> = Vec::new();
    let mut hash_mismatches = 0usize;
    let mut checked = 0usize;

    let mut start = lo;
    while start <= hi {
        let end = (start as u64 + PAGE as u64).min(hi as u64 + 1) as u32;
        let a = fetch_archive(&agent, &archive, start, end)?;
        let i = fetch_indexer(&agent, &indexer, start, end)?;
        for h in start..end {
            match (a.get(&h), i.get(&h)) {
                (Some(ab), Some(ib)) => {
                    checked += 1;
                    hash_mismatches += compare_block(h, ab, ib, &mut hard);
                }
                (Some(_), None) => {
                    hard.push(format!("h{h}: canonical in archive, missing in indexer"))
                }
                (None, Some(_)) => {
                    hard.push(format!("h{h}: canonical in indexer, missing in archive"))
                }
                (None, None) => {}
            }
        }
        eprintln!("  .. through {end} (checked {checked})");
        start = end;
    }

    eprintln!(
        "\nchecked {checked} canonical blocks: {} hard discrepancies, {hash_mismatches} tx-hash mismatches",
        hard.len()
    );
    for d in hard.iter().take(50) {
        eprintln!("  DISCREPANCY {d}");
    }
    if hard.len() > 50 {
        eprintln!("  .. and {} more", hard.len() - 50);
    }
    // Signed-command tx hashes must match (fixed in the v2 hash work). zkApp
    // command hashes aren't compared yet — mina's Zkapp_command hasher is a
    // separate algorithm still to be ported — so they're covered by the count
    // check, not by hash.
    assert!(
        hard.is_empty() && hash_mismatches == 0,
        "{} hard discrepancies, {hash_mismatches} signed-command tx-hash mismatches (see log)",
        hard.len()
    );
    Ok(())
}
