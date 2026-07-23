//! Zkapp store trait
//!
//! Defines the functionality for:
//! - zkapp accounts
//! - zkapp transactions
//! - minted tokens
//! - actions
//! - events

use crate::{
    base::{public_key::PublicKey, state_hash::StateHash},
    command::TxnHash,
    ledger::{
        account::{Permissions, Timing},
        token::{TokenAddress, TokenSymbol},
    },
    mina_blocks::v2::{
        zkapp::verification_key::VerificationKeyHash, VerificationKey, ZkappState, ZkappUri,
    },
    store::Result,
};
use serde::{Deserialize, Serialize};
use speedb::{DBIterator, Direction};

pub mod actions;
pub mod events;
pub mod tokens;

/// One verification-key change on a zkApp account, recorded in the
/// height-ordered VK-history index (`getVerificationKeyHistory` /
/// `getLastVerificationKeyChange`, issue #95 item 5). Only *actual* changes are
/// recorded: `old_vk_hash` is `None` for the first-ever key (the VK's creation)
/// and `Some(prev)` thereafter; a re-set to the same hash is not recorded. The
/// block height & state hash live in the index key, so they are not duplicated
/// here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationKeyChange {
    pub token: TokenAddress,
    pub txn_hash: TxnHash,

    /// Hash of the previous verification key, or `None` when this is the first
    /// key set on the account.
    pub old_vk_hash: Option<VerificationKeyHash>,

    /// The verification key set by this change (full key + hash).
    pub verification_key: VerificationKey,
}

pub trait ZkappStore {
    ///////////////
    // app state //
    ///////////////

    /// Get the count of zkapp state changes
    fn get_zkapp_state_num(&self, token: &TokenAddress, pk: &PublicKey) -> Result<Option<u32>>;

    /// Get the zkapp state at the specified index
    fn get_zkapp_state(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        index: u32,
    ) -> Result<Option<ZkappState>>;

    /// Add zkapp state
    fn add_zkapp_state(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        app_state: &ZkappState,
    ) -> Result<()>;

    /// Remove the most recent zkapp state & return it
    ///
    /// Returns an error if no app state to remove
    fn remove_last_zkapp_state(&self, token: &TokenAddress, pk: &PublicKey) -> Result<ZkappState>;

    /////////////////
    // permissions //
    /////////////////

    /// Get the count of zkapp permissions changes
    fn get_zkapp_permissions_num(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
    ) -> Result<Option<u32>>;

    /// Get the zkapp permissions at the specified index
    fn get_zkapp_permissions(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        index: u32,
    ) -> Result<Option<Permissions>>;

    /// Add zkapp permissions
    fn add_zkapp_permissions(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        permissions: &Permissions,
    ) -> Result<()>;

    /// Remove the most recent zkapp permissions & return it
    ///
    /// Returns an error if no permissions to remove
    fn remove_last_zkapp_permissions(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
    ) -> Result<Permissions>;

    //////////////////////
    // verification key //
    //////////////////////

    /// Get the count of zkapp verification key changes
    fn get_zkapp_verification_key_num(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
    ) -> Result<Option<u32>>;

    /// Get the zkapp verification key at the specified index
    fn get_zkapp_verification_key(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        index: u32,
    ) -> Result<Option<VerificationKey>>;

    /// Add zkapp verification key
    fn add_zkapp_verification_key(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        verification_key: &VerificationKey,
    ) -> Result<()>;

    /// Remove the most recent zkapp verification key & return it
    ///
    /// Returns an error if no verification key to remove
    fn remove_last_zkapp_verification_key(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
    ) -> Result<VerificationKey>;

    ////////////////////////////////
    // zkapp verification-key history //
    ////////////////////////////////

    /// Record a verification-key change for `pk` at `(block_height,
    /// state_hash)` in the height-ordered VK-history index. Only actual
    /// changes are passed (the caller skips no-op re-sets).
    fn add_zkapp_verification_key_change(
        &self,
        pk: &PublicKey,
        block_height: u32,
        state_hash: &StateHash,
        change: &VerificationKeyChange,
    ) -> Result<()>;

    /// Remove the VK-history record for `pk` at `(block_height, state_hash)`.
    /// Idempotent: a no-op when no record exists (the apply side only writes on
    /// an actual change), so the unapply path can call it unconditionally.
    fn remove_zkapp_verification_key_change(
        &self,
        pk: &PublicKey,
        block_height: u32,
        state_hash: &StateHash,
    ) -> Result<()>;

    /// The most recent VK change for `pk`, with its block height & state hash,
    /// or `None` if the key never changed. Backs
    /// `getLastVerificationKeyChange`.
    fn get_last_zkapp_verification_key_change(
        &self,
        pk: &PublicKey,
    ) -> Result<Option<(u32, StateHash, VerificationKeyChange)>>;

    /// Iterate `pk`'s VK-history records. Key layout `{pk}{block_height BE}
    /// {state_hash}`; `Direction::Reverse` yields newest-first. Backs
    /// `getVerificationKeyHistory`.
    fn zkapp_verification_key_history_iterator(
        &self,
        pk: &PublicKey,
        direction: Direction,
    ) -> DBIterator<'_>;

    ///////////////
    // zkapp uri //
    ///////////////

    /// Get the count of zkapp uri changes
    fn get_zkapp_uri_num(&self, token: &TokenAddress, pk: &PublicKey) -> Result<Option<u32>>;

    /// Get the zkapp uri at the specified index
    fn get_zkapp_uri(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        index: u32,
    ) -> Result<Option<ZkappUri>>;

    /// Add zkapp uri
    fn add_zkapp_uri(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        zkapp_uri: &ZkappUri,
    ) -> Result<()>;

    /// Remove the most recent zkapp uri & return it
    ///
    /// Returns an error if no zkapp uri to remove
    fn remove_last_zkapp_uri(&self, token: &TokenAddress, pk: &PublicKey) -> Result<ZkappUri>;

    //////////////////
    // token symbol //
    //////////////////

    /// Get the count of zkapp token symbol changes
    fn get_zkapp_token_symbol_num(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
    ) -> Result<Option<u32>>;

    /// Get the zkapp token symbol at the specified index
    fn get_zkapp_token_symbol(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        index: u32,
    ) -> Result<Option<TokenSymbol>>;

    /// Add zkapp token symbol
    fn add_zkapp_token_symbol(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        token_symbol: &TokenSymbol,
    ) -> Result<()>;

    /// Remove the most recent zkapp token symbol & return it
    ///
    /// Returns an error if no token symbol to remove
    fn remove_last_zkapp_token_symbol(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
    ) -> Result<TokenSymbol>;

    ////////////
    // timing //
    ////////////

    /// Get the count of zkapp timing changes
    fn get_zkapp_timing_num(&self, token: &TokenAddress, pk: &PublicKey) -> Result<Option<u32>>;

    /// Get the zkapp timing at the specified index
    fn get_zkapp_timing(
        &self,
        token: &TokenAddress,
        pk: &PublicKey,
        index: u32,
    ) -> Result<Option<Timing>>;

    /// Add zkapp timing
    fn add_zkapp_timing(&self, token: &TokenAddress, pk: &PublicKey, timing: &Timing)
        -> Result<()>;

    /// Remove the most recent zkapp timing & return it
    ///
    /// Returns an error if no timing to remove
    fn remove_last_zkapp_timing(&self, token: &TokenAddress, pk: &PublicKey) -> Result<Timing>;
}
