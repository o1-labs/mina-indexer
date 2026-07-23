//! Zkapp store helpers

use super::common::U32_LEN;
use crate::{
    base::{public_key::PublicKey, state_hash::StateHash},
    ledger::token::TokenAddress,
};

pub mod actions;
pub mod events;
pub mod tokens;

/// Key for the height-ordered VK-history index. Layout `{pk}{block_height BE}
/// {state_hash}` -- pk-prefixed so a single public key's changes scan as a
/// contiguous range, height big-endian so the range is chronological (reverse
/// it for newest-first).
pub fn zkapp_vk_history_key(
    pk: &PublicKey,
    block_height: u32,
    state_hash: &StateHash,
) -> [u8; PublicKey::LEN + U32_LEN + StateHash::LEN] {
    let mut key = [0u8; PublicKey::LEN + U32_LEN + StateHash::LEN];
    key[..PublicKey::LEN].copy_from_slice(pk.0.as_bytes());
    key[PublicKey::LEN..][..U32_LEN].copy_from_slice(&block_height.to_be_bytes());
    key[PublicKey::LEN..][U32_LEN..].copy_from_slice(state_hash.0.as_bytes());
    key
}

/// Use with [zkapp_state_cf]
pub fn zkapp_state_key(
    token: &TokenAddress,
    pk: &PublicKey,
    index: u32,
) -> [u8; TokenAddress::LEN + PublicKey::LEN + U32_LEN] {
    let mut key = [0; TokenAddress::LEN + PublicKey::LEN + U32_LEN];

    key[..TokenAddress::LEN].copy_from_slice(token.0.as_bytes());
    key[TokenAddress::LEN..][..PublicKey::LEN].copy_from_slice(pk.0.as_bytes());
    key[TokenAddress::LEN..][PublicKey::LEN..].copy_from_slice(&index.to_be_bytes());

    key
}

/// Use with [zkapp_state_num_cf]
pub fn zkapp_state_num_key(
    token: &TokenAddress,
    pk: &PublicKey,
) -> [u8; TokenAddress::LEN + PublicKey::LEN] {
    let mut key = [0; TokenAddress::LEN + PublicKey::LEN];

    key[..TokenAddress::LEN].copy_from_slice(token.0.as_bytes());
    key[TokenAddress::LEN..].copy_from_slice(pk.0.as_bytes());

    key
}

/// Use with [zkapp_permissions_cf]
pub fn zkapp_permissions_key(
    token: &TokenAddress,
    pk: &PublicKey,
    index: u32,
) -> [u8; TokenAddress::LEN + PublicKey::LEN + U32_LEN] {
    zkapp_state_key(token, pk, index)
}

/// Use with [zkapp_permissions_num_cf]
pub fn zkapp_permissions_num_key(
    token: &TokenAddress,
    pk: &PublicKey,
) -> [u8; TokenAddress::LEN + PublicKey::LEN] {
    zkapp_state_num_key(token, pk)
}

/// Use with [zkapp_verification_key_cf]
pub fn zkapp_verification_key_key(
    token: &TokenAddress,
    pk: &PublicKey,
    index: u32,
) -> [u8; TokenAddress::LEN + PublicKey::LEN + U32_LEN] {
    zkapp_state_key(token, pk, index)
}

/// Use with [zkapp_verification_key_num_cf]
pub fn zkapp_verification_key_num_key(
    token: &TokenAddress,
    pk: &PublicKey,
) -> [u8; TokenAddress::LEN + PublicKey::LEN] {
    zkapp_state_num_key(token, pk)
}

/// Use with [zkapp_uri_cf]
pub fn zkapp_uri_key(
    token: &TokenAddress,
    pk: &PublicKey,
    index: u32,
) -> [u8; TokenAddress::LEN + PublicKey::LEN + U32_LEN] {
    zkapp_state_key(token, pk, index)
}

/// Use with [zkapp_uri_num_cf]
pub fn zkapp_uri_num_key(
    token: &TokenAddress,
    pk: &PublicKey,
) -> [u8; TokenAddress::LEN + PublicKey::LEN] {
    zkapp_state_num_key(token, pk)
}

/// Use with [zkapp_token_symbol_cf]
pub fn zkapp_token_symbol_key(
    token: &TokenAddress,
    pk: &PublicKey,
    index: u32,
) -> [u8; TokenAddress::LEN + PublicKey::LEN + U32_LEN] {
    zkapp_state_key(token, pk, index)
}

/// Use with [zkapp_token_symbol_num_cf]
pub fn zkapp_token_symbol_num_key(
    token: &TokenAddress,
    pk: &PublicKey,
) -> [u8; TokenAddress::LEN + PublicKey::LEN] {
    zkapp_state_num_key(token, pk)
}

/// Use with [zkapp_timing_cf]
pub fn zkapp_timing_key(
    token: &TokenAddress,
    pk: &PublicKey,
    index: u32,
) -> [u8; TokenAddress::LEN + PublicKey::LEN + U32_LEN] {
    zkapp_state_key(token, pk, index)
}

/// Use with [zkapp_timing_num_cf]
pub fn zkapp_timing_num_key(
    token: &TokenAddress,
    pk: &PublicKey,
) -> [u8; TokenAddress::LEN + PublicKey::LEN] {
    zkapp_state_num_key(token, pk)
}
