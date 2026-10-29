//! REST `blocks` endpoint

use crate::{
    base::state_hash::StateHash,
    block::{precomputed::PrecomputedBlock, store::BlockStore},
    canonicity::proof::{build_canonicity_proof, CanonicityProof},
    constants::MAINNET_TRANSITION_FRONTIER_K,
    store::IndexerStore,
    web::graphql::{
        blocks::{block::Block, get_counts},
        get_block,
    },
};
use actix_web::{
    get,
    http::header::ContentType,
    web::{self, Data},
    HttpResponse,
};
use log::error;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Deserialize)]
struct Params {
    limit: Option<u32>,
    height: Option<u32>,
}

fn get_limit(limit: Option<u32>) -> u32 {
    limit.map(|value| value.min(100)).unwrap_or(10)
}

fn format_blocks(blocks: Vec<Block>) -> String {
    format!("{blocks:#?}").replace(",\n]", "\n]")
}

#[get("/blocks")]
pub async fn get_blocks(
    store: Data<Arc<IndexerStore>>,
    params: web::Query<Params>,
) -> HttpResponse {
    let db = store.as_ref();
    let limit = get_limit(params.limit);

    // Resolve the block counts once (a store error here is a 500, not a panic;
    // this also avoids recomputing them per result below).
    let counts = match get_counts(db, None, None) {
        Ok(counts) => counts,
        Err(e) => {
            error!("GET /blocks failed: {e:?}");
            return HttpResponse::InternalServerError().finish();
        }
    };

    // Check for height query parameter
    if let Some(height) = params.height {
        if let Ok(blocks) = db.get_blocks_at_height(height) {
            let blocks = blocks
                .iter()
                .flat_map(|state_hash| {
                    let block = get_block(db, state_hash);
                    Some(Block::from_precomputed(db, &block, counts))
                })
                .take(limit as usize)
                .collect();
            return HttpResponse::Ok()
                .content_type(ContentType::json())
                .body(format_blocks(blocks));
        }
    }

    if let Ok(Some(best_tip)) = db.get_best_block() {
        let mut best_chain: Vec<Block> = Vec::with_capacity(limit as usize);

        // Process best tip
        best_chain.push(Block::from_precomputed(db, &best_tip, counts));

        let mut parent_state_hash = best_tip.previous_state_hash();

        while best_chain.len() < limit as usize {
            if let Ok(Some((block, _))) = db.get_block(&parent_state_hash) {
                best_chain.push(Block::from_precomputed(db, &block, counts));
                parent_state_hash = block.previous_state_hash();
            } else {
                // No parent
                break;
            }
        }

        return HttpResponse::Ok()
            .content_type(ContentType::json())
            .body(format_blocks(best_chain));
    }
    HttpResponse::NotFound().finish()
}

#[derive(Deserialize)]
pub struct ProofQuery {
    /// How far above the target the canonical parent-chain reaches (bounded).
    /// Defaults to the transition-frontier depth `k`.
    anchor_span: Option<u32>,
}

/// Tier-1 verifiable response envelope. Self-contained: the client re-runs
/// `mina-verify verify_block` on `block` and re-checks `canonicity` against a
/// tip it independently trusts — it needs nothing from the (untrusted) indexer
/// it hasn't verified. See `docs/trustless-responses.md`.
#[derive(Serialize)]
struct BlockProofResponse<'a> {
    state_hash: String,
    height: u32,
    /// Depth below the best tip; lets the client apply k-finality for deep blocks
    /// without walking `parent_chain`. `None` if the block is not canonical.
    depth: Option<u32>,
    /// The full precomputed block (incl. `protocol_state_proof`) — what the
    /// client re-verifies.
    block: &'a PrecomputedBlock,
    /// Canonicity proof, or `null` if the block is not on the canonical chain.
    /// A `null` here means the client MUST NOT treat the block as "the block at
    /// this height" (it may be a valid-but-orphaned fork).
    canonicity: Option<CanonicityProof>,
}

/// `GET /blocks/{state_hash}/proof[?anchor_span=N]` — Tier-1 verifiable block.
#[get("/blocks/{state_hash}/proof")]
pub async fn get_block_proof(
    store: Data<Arc<IndexerStore>>,
    state_hash: web::Path<String>,
    query: web::Query<ProofQuery>,
) -> HttpResponse {
    let db = store.as_ref();

    if !StateHash::is_valid(&state_hash) {
        return HttpResponse::NotFound().finish();
    }
    let sh: StateHash = state_hash.clone().into();

    let block = match db.get_block(&sh) {
        Ok(Some((block, _))) => block,
        Ok(None) => return HttpResponse::NotFound().finish(),
        Err(e) => {
            error!("GET /blocks/{{state_hash}}/proof get_block failed: {e:?}");
            return HttpResponse::InternalServerError().finish();
        }
    };

    let anchor_span = query.anchor_span.unwrap_or(MAINNET_TRANSITION_FRONTIER_K);
    let canonicity = match build_canonicity_proof(db, &sh, anchor_span) {
        Ok(canonicity) => canonicity,
        Err(e) => {
            error!("GET /blocks/{{state_hash}}/proof canonicity failed: {e:?}");
            return HttpResponse::InternalServerError().finish();
        }
    };

    let resp = BlockProofResponse {
        state_hash: state_hash.into_inner(),
        height: block.blockchain_length(),
        depth: canonicity.as_ref().map(|c| c.depth),
        block: &block,
        canonicity,
    };
    match serde_json::to_string(&resp) {
        Ok(body) => HttpResponse::Ok()
            .content_type(ContentType::json())
            .body(body),
        Err(e) => {
            error!("GET /blocks/{{state_hash}}/proof serialize failed: {e:?}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[get("/blocks/{state_hash}")]
pub async fn get_block_by_state_hash(
    store: Data<Arc<IndexerStore>>,
    state_hash: web::Path<String>,
) -> HttpResponse {
    let db = store.as_ref();

    if StateHash::is_valid(&state_hash) {
        if let Ok(Some((ref block, _))) = db.get_block(&state_hash.clone().into()) {
            let counts = match get_counts(db, None, None) {
                Ok(counts) => counts,
                Err(e) => {
                    error!("GET /blocks/{{state_hash}} failed: {e:?}");
                    return HttpResponse::InternalServerError().finish();
                }
            };
            let block = Block::from_precomputed(db, block, counts);
            return HttpResponse::Ok()
                .content_type(ContentType::json())
                .body(format!("{block:?}"));
        }
    }

    HttpResponse::NotFound().finish()
}
