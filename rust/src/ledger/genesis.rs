use super::{
    account::{
        Account, Permission as AccountPermission, Permissions as AccountPermissions,
        ReceiptChainHash, Timing, VotingFor,
    },
    token::{TokenAddress, TokenSymbol},
    Ledger, TokenLedger,
};
use crate::{
    base::{amount::Amount, nonce::Nonce, numeric::Numeric, public_key::PublicKey,
        state_hash::StateHash},
    block::genesis::GenesisBlock,
    constants::*,
    mina_blocks::v2::{
        zkapp::{
            action_state::ActionState,
            app_state::{AppState, ZkappState},
            verification_key::{VerificationKey, VerificationKeyHash},
        },
        ZkappAccount, ZkappUri,
    },
    utility::compression::decompress_gzip,
};
use anyhow::anyhow;
use log::error;
use serde::{Deserialize, Serialize};
use std::{path::Path, str::FromStr};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisLedger {
    /// Keyed by (token, pk) -- see [`GenesisLedger::new`]
    ledger: Ledger,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisRoot {
    pub genesis: Option<GenesisTimestamp>,
    pub proof: Option<GenesisProof>,
    pub ledger: GenesisAccounts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisTimestamp {
    pub genesis_state_timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisProof {
    pub fork: GenesisForkProof,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisForkProof {
    pub state_hash: StateHash,
    pub blockchain_length: u32,
    pub global_slot_since_genesis: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisAccounts {
    pub name: Option<String>,
    pub accounts: Vec<GenesisAccount>,
    pub seed: Option<String>,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct GenesisAccount {
    pub pk: String,
    pub balance: String,
    pub delegate: Option<String>,
    pub token_permissions: Option<TokenPermissions>,
    pub receipt_chain_hash: Option<ReceiptChainHash>,
    pub voting_for: Option<VotingFor>,

    /// The genesis dump states each account's permissions, and they are hashed into
    /// the account. Dropping them (as this once did) leaves every untouched genesis
    /// account with no permissions at all.
    pub permissions: Option<GenesisPermissionsJson>,
    pub timing: Option<GenesisAccountTiming>,

    #[serde(default)]
    pub token_symbol: Option<TokenSymbol>,

    /// The genesis dump carries zkApp accounts in full -- app state, action state and
    /// verification key. On mesa that is ~1,800 accounts whose 32-wide app state is the
    /// whole point of the network.
    #[serde(default)]
    pub zkapp: Option<GenesisZkapp>,

    #[serde(default)]
    pub nonce: Option<Nonce>,

    #[serde(default, deserialize_with = "deserialize_genesis_token")]
    pub token: Option<TokenAddress>,
}

/// The genesis `token` field is a numeric token id on mainnet/hardfork ledgers
/// (`"1"` = MINA) but a base58 token address on mesa-style ledgers. Accept both.
fn deserialize_genesis_token<'de, D>(deserializer: D) -> Result<Option<TokenAddress>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match Option::<serde_json::Value>::deserialize(deserializer)? {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Number(_)) => Some(TokenAddress::default()),
        Some(serde_json::Value::String(s)) => {
            if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
                Some(TokenAddress::default())
            } else {
                Some(s.parse::<TokenAddress>().map_err(serde::de::Error::custom)?)
            }
        }
        Some(other) => {
            return Err(serde::de::Error::custom(format!("invalid token: {other}")))
        }
    })
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct TokenPermissions {}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Permissions {
    pub stake: bool,
    pub edit_state: Permission,
    pub send: Permission,
    pub set_delegate: Permission,
    pub set_permissions: Permission,
    pub set_verification_key: Permission,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Permission {
    #[default]
    Signature,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisAccountTiming {
    pub initial_minimum_balance: String,
    pub cliff_time: String,
    pub cliff_amount: String,
    pub vesting_period: String,
    pub vesting_increment: String,
}

/// The version byte Mina prefixes a base58check-encoded verification key with.
const VERIFICATION_KEY_VERSION_BYTE: u8 = 0x1b;

/// Blocks write zkApp counters as strings (`"0"`), the genesis dump writes them as bare
/// numbers (`904964`). [`Numeric`] only parses the former, so accept both.
fn numeric_str_or_num<'de, D>(deserializer: D) -> Result<Numeric<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Null => Numeric::default(),
        serde_json::Value::Number(n) => {
            let n = n
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| D::Error::custom(format!("{n} is not a u32")))?;

            Numeric(n)
        }
        serde_json::Value::String(s) => s.parse().map_err(D::Error::custom)?,
        other => return Err(D::Error::custom(format!("invalid number: {other}"))),
    })
}

/// An auth requirement as the *genesis state dump* writes it: a lowercase string.
/// Blocks write the same thing as a variant array (`["Signature"]`), which is what
/// [`v2::PermissionKind`] parses -- so the two shapes need two types.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GenesisAuth {
    #[default]
    None,
    Either,
    Proof,
    Signature,
    Impossible,
}

impl From<GenesisAuth> for AccountPermission {
    fn from(value: GenesisAuth) -> Self {
        match value {
            GenesisAuth::None => Self::None,
            GenesisAuth::Either => Self::Either,
            GenesisAuth::Proof => Self::Proof,
            GenesisAuth::Signature => Self::Signature,
            GenesisAuth::Impossible => Self::Impossible,
        }
    }
}

/// `{"auth": "signature", "txn_version": "2"}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisSetVerificationKey {
    pub auth: GenesisAuth,

    #[serde(deserialize_with = "numeric_str_or_num")]
    pub txn_version: Numeric<u32>,
}

/// Genesis ledgers come in two permission shapes.
///
/// The state dumps (mesa, devnet) state permissions in full, including the
/// `set_verification_key` txn_version -- these are the ones hashed into the account.
///
/// The older mainnet/hardfork ledgers state a handful of auths and write
/// `set_verification_key` as a bare `"signature"`, with **no txn_version anywhere in the
/// file**. There is nothing to reconstruct it from, so rather than invent one we leave
/// those accounts' permissions unset, exactly as before.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GenesisPermissionsJson {
    Stated(GenesisPermissions),
    Legacy(serde_json::Value),
}

impl GenesisPermissionsJson {
    fn into_permissions(self) -> Option<AccountPermissions> {
        match self {
            Self::Stated(permissions) => Some(permissions.into()),
            Self::Legacy(_) => None,
        }
    }
}

/// Mina's account defaults: everything is `signature` except `access`/`receive`.
const fn auth_signature() -> GenesisAuth {
    GenesisAuth::Signature
}

const fn auth_none() -> GenesisAuth {
    GenesisAuth::None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisPermissions {
    #[serde(default = "auth_signature")]
    pub edit_state: GenesisAuth,

    #[serde(default = "auth_none")]
    pub access: GenesisAuth,

    #[serde(default = "auth_signature")]
    pub send: GenesisAuth,

    #[serde(default = "auth_none")]
    pub receive: GenesisAuth,

    #[serde(default = "auth_signature")]
    pub set_delegate: GenesisAuth,

    #[serde(default = "auth_signature")]
    pub set_permissions: GenesisAuth,

    pub set_verification_key: GenesisSetVerificationKey,

    #[serde(default = "auth_signature")]
    pub set_zkapp_uri: GenesisAuth,

    #[serde(default = "auth_signature")]
    pub edit_action_state: GenesisAuth,

    #[serde(default = "auth_signature")]
    pub set_token_symbol: GenesisAuth,

    #[serde(default = "auth_signature")]
    pub increment_nonce: GenesisAuth,

    #[serde(default = "auth_signature")]
    pub set_voting_for: GenesisAuth,

    #[serde(default = "auth_signature")]
    pub set_timing: GenesisAuth,
}

impl From<GenesisPermissions> for AccountPermissions {
    fn from(value: GenesisPermissions) -> Self {
        Self {
            edit_state: value.edit_state.into(),
            access: value.access.into(),
            send: value.send.into(),
            receive: value.receive.into(),
            set_delegate: value.set_delegate.into(),
            set_permissions: value.set_permissions.into(),
            set_verification_key: (
                value.set_verification_key.auth.into(),
                value.set_verification_key.txn_version.0.to_string(),
            ),
            set_zkapp_uri: value.set_zkapp_uri.into(),
            edit_action_state: value.edit_action_state.into(),
            set_token_symbol: value.set_token_symbol.into(),
            increment_nonce: value.increment_nonce.into(),
            set_voting_for: value.set_voting_for.into(),
            set_timing: value.set_timing.into(),
        }
    }
}

/// A zkApp account as the *genesis state dump* writes it. It differs from the block
/// shape in two ways that matter, and both are normalised on the way in so the store
/// holds one encoding regardless of where an account came from:
///
/// - field elements are decimal here, `0x`-prefixed hex in blocks;
/// - the verification key is bare base64 binprot here, base58check in blocks -- and the
///   dump carries **no vk hash at all**, which blocks do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisZkapp {
    pub app_state: Vec<String>,

    pub action_state: Vec<String>,

    #[serde(default)]
    pub verification_key: Option<String>,

    #[serde(default, deserialize_with = "numeric_str_or_num")]
    pub zkapp_version: Numeric<u32>,

    #[serde(default, deserialize_with = "numeric_str_or_num")]
    pub last_action_slot: Numeric<u32>,

    #[serde(default)]
    pub proved_state: bool,

    #[serde(default)]
    pub zkapp_uri: String,
}

/// A decimal field element -> the `0x`-prefixed, zero-padded, big-endian hex that
/// blocks use.
fn field_to_hex(decimal: &str) -> anyhow::Result<String> {
    let value = decimal
        .parse::<num::BigUint>()
        .map_err(|e| anyhow!("malformed genesis field element {decimal:?}: {e}"))?;

    let mut bytes = [0u8; 32];
    let be = value.to_bytes_be();
    if be.len() > 32 {
        return Err(anyhow!("genesis field element {decimal:?} exceeds 32 bytes"));
    }
    bytes[32 - be.len()..].copy_from_slice(&be);

    Ok(format!("0x{}", hex::encode_upper(bytes)))
}

/// The dump's base64 binprot key -> the base58check encoding blocks use.
fn vk_to_base58check(base64_vk: &str) -> anyhow::Result<String> {
    use base64::Engine;

    let binprot = base64::engine::general_purpose::STANDARD
        .decode(base64_vk)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(base64_vk))
        .map_err(|e| anyhow!("malformed genesis verification key: {e}"))?;

    Ok(bs58::encode(binprot)
        .with_check_version(VERIFICATION_KEY_VERSION_BYTE)
        .into_string())
}

impl GenesisZkapp {
    fn into_zkapp_account(self) -> anyhow::Result<ZkappAccount> {
        let app_state = self
            .app_state
            .iter()
            .map(|fp| field_to_hex(fp).map(AppState))
            .collect::<anyhow::Result<Vec<_>>>()?;

        let action_state = self
            .action_state
            .iter()
            .map(|fp| field_to_hex(fp).map(ActionState))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let action_state: [ActionState; 5] = action_state
            .try_into()
            .map_err(|_| anyhow!("genesis zkapp action_state must hold exactly 5 elements"))?;

        // The dump gives us the key but not its hash -- the hash is a Poseidon hash of
        // the key, which the indexer cannot compute. Leave it empty rather than invent
        // one: nothing here keys off it, a verifier recomputes it from the key, and the
        // first block to touch the account supplies the real hash.
        let verification_key = match self.verification_key.as_deref() {
            Some(vk) => VerificationKey {
                data: vk_to_base58check(vk)?.into(),
                hash: VerificationKeyHash::default(),
            },
            None => VerificationKey::default(),
        };

        Ok(ZkappAccount {
            app_state: ZkappState(app_state),
            action_state,
            verification_key,
            proved_state: self.proved_state,
            zkapp_uri: ZkappUri(self.zkapp_uri),
            zkapp_version: self.zkapp_version,
            last_action_slot: self.last_action_slot,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolConstants {
    pub k: Option<u32>,
    pub slots_per_epoch: Option<u32>,
    pub slots_per_sub_window: Option<u32>,
    pub delta: Option<u32>,
    pub txpool_max_size: Option<u32>,
}

///////////
// impls //
///////////

impl ProtocolConstants {
    pub fn from_path<P>(path: Option<P>) -> anyhow::Result<Self>
    where
        P: AsRef<Path>,
    {
        let mut constants = Self::default();
        if let Some(path) = path {
            if let Ok(ref contents) = std::fs::read(path) {
                if let Ok(override_constants) = serde_json::from_slice(contents) {
                    constants.override_with(override_constants);
                } else {
                    error!(
                        "Error parsing supplied protocol constants. Using default:\n{}",
                        serde_json::to_string_pretty(&constants)?
                    )
                }
            } else {
                error!(
                    "Error reading protocol constants file. Using default:\n{}",
                    serde_json::to_string_pretty(&constants)?
                )
            }
        }
        Ok(constants)
    }

    pub fn override_with(&mut self, constants: Self) {
        let Self {
            delta,
            k,
            slots_per_epoch,
            slots_per_sub_window,
            txpool_max_size,
        } = constants;

        if delta.is_some() {
            self.delta = delta;
        }
        if k.is_some() {
            self.k = k;
        }
        if slots_per_epoch.is_some() {
            self.slots_per_epoch = slots_per_epoch;
        }
        if slots_per_sub_window.is_some() {
            self.slots_per_sub_window = slots_per_sub_window;
        }
        if txpool_max_size.is_some() {
            self.txpool_max_size = txpool_max_size;
        }
    }
}

impl GenesisLedger {
    /// Original mainnet genesis ledger
    pub fn new_v1() -> anyhow::Result<Self> {
        Self::from_str(include_str!("../../data/genesis_ledgers/mainnet.json"))
    }

    /// Hardfork genesis ledger
    pub fn new_v2() -> anyhow::Result<Self> {
        let bytes = include_bytes!("../../data/genesis_ledgers/hardfork.json.gz");
        if let Ok(root) = decompress_gzip(bytes) {
            if let Ok(root) = serde_json::from_slice::<GenesisRoot>(&root) {
                Ok(root.into())
            } else {
                Err(anyhow::anyhow!("Failed to deserialize genesis ledger"))
            }
        } else {
            Err(anyhow::anyhow!("Failed to decompress genesis ledger"))
        }
    }

    /// This is the only way to construct a genesis ledger
    pub fn new(genesis: GenesisAccounts) -> GenesisLedger {
        // A ledger account is identified by (token, pk), not by pk. Keying the genesis
        // accounts by pk alone silently drops one of them whenever a public key holds
        // both a MINA account and a custom-token account -- which loses 141 accounts on
        // mesa, leaving the ledger with fewer leaves than the protocol's.
        let mut ledger = Ledger::new();

        // genesis block winner
        let block_creator = Account::from_genesis(GenesisBlock::new_v1().unwrap());
        ledger.insert_account(block_creator, &TokenAddress::default());

        for account in genesis.accounts {
            let balance = account
                .balance
                .parse::<Amount>()
                .unwrap_or_else(|_| panic!("Unable to parse Genesis Balance"));

            let public_key = PublicKey::from(account.pk);
            let delegate = account
                .delegate
                .map_or_else(|| public_key.to_owned(), PublicKey);
            let token = account.token.clone().unwrap_or_default();

            let zkapp = account.zkapp.map(|zkapp| {
                zkapp.into_zkapp_account().unwrap_or_else(|e| {
                    panic!("Unable to parse genesis zkApp account {public_key}: {e}")
                })
            });

            ledger.insert_account(
                Account {
                    public_key,
                    balance,
                    nonce: account.nonce,
                    delegate: delegate.into(),
                    token: Some(token.to_owned()),
                    receipt_chain_hash: account.receipt_chain_hash,
                    voting_for: account.voting_for,
                    timing: account.timing.map(Into::into),
                    permissions: account
                        .permissions
                        .and_then(GenesisPermissionsJson::into_permissions),
                    token_symbol: account.token_symbol,
                    zkapp,
                    genesis_account: Some(balance),
                    ..Default::default()
                },
                &token,
            );
        }

        Self { ledger }
    }

    pub fn parse_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        GenesisRoot::parse_file(path).map(Into::into)
    }
}

impl GenesisRoot {
    pub fn parse_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let bytes = std::fs::read(path.as_ref())?;

        // decompress if gzip'd
        if path.as_ref().extension().is_some_and(|ext| ext == "gz") {
            let bytes = decompress_gzip(&bytes[..])?;
            return Ok(serde_json::from_slice(&bytes)?);
        }

        Ok(serde_json::from_slice(&bytes)?)
    }
}

//////////////
// defaults //
//////////////

impl std::default::Default for ProtocolConstants {
    fn default() -> Self {
        Self {
            delta: Some(MAINNET_DELTA),
            k: Some(MAINNET_TRANSITION_FRONTIER_K),
            txpool_max_size: Some(MAINNET_TXPOOL_MAX_SIZE),
            slots_per_epoch: Some(MAINNET_EPOCH_SLOT_COUNT),
            slots_per_sub_window: Some(MAINNET_SLOTS_PER_SUB_WINDOW),
        }
    }
}

/////////////////
// conversions //
/////////////////

impl FromStr for GenesisRoot {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s).map_err(|e| anyhow!("Error parsing genesis root: {e}"))
    }
}

impl FromStr for GenesisLedger {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        GenesisRoot::from_str(s).map(Into::into)
    }
}

impl From<GenesisRoot> for GenesisLedger {
    fn from(value: GenesisRoot) -> Self {
        Self::new(value.ledger)
    }
}

/// Add the account-creation fee back to the MINA balances. The ledger deducts it again
/// when it serializes an account that has not paid it, so the two cancel and the served
/// balance is the one the genesis ledger states. Custom-token balances are shown as-is.
fn with_display_fee(account: Account) -> Account {
    let is_custom_token = account
        .token
        .as_ref()
        .is_some_and(|t| t.0 != MINA_TOKEN_ADDRESS);

    if is_custom_token {
        account
    } else {
        Account {
            balance: account.balance + MAINNET_ACCOUNT_CREATION_FEE,
            ..account
        }
    }
}

impl From<GenesisLedger> for Ledger {
    fn from(value: GenesisLedger) -> Self {
        let mut ledger = Ledger::new();

        for (token, token_ledger) in value.ledger.tokens.into_iter() {
            for (_pk, account) in token_ledger.accounts.into_iter() {
                ledger.insert_account(with_display_fee(account), &token);
            }
        }

        ledger
    }
}

impl From<GenesisLedger> for TokenLedger {
    /// The MINA token ledger only -- custom-token accounts do not live here. Prefer
    /// `Ledger`, which keeps every token.
    fn from(value: GenesisLedger) -> Self {
        Self {
            accounts: value
                .ledger
                .tokens
                .get(&TokenAddress::default())
                .map(|mina| {
                    mina.accounts
                        .iter()
                        .map(|(pk, account)| (pk.to_owned(), with_display_fee(account.to_owned())))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

impl From<GenesisAccountTiming> for Timing {
    fn from(value: GenesisAccountTiming) -> Self {
        Self {
            initial_minimum_balance: value
                .initial_minimum_balance
                .parse::<Amount>()
                .unwrap_or_else(|_| panic!("Unable to parse genesis initial minimum balance"))
                .0
                .into(),
            cliff_time: value.cliff_time.parse().expect("cliff time is u64"),
            cliff_amount: value
                .cliff_amount
                .parse::<Amount>()
                .unwrap_or_else(|_| panic!("Unable to parse genesis cliff amount"))
                .0
                .into(),
            vesting_period: value.vesting_period.parse().expect("vesting period is u64"),
            vesting_increment: value
                .vesting_increment
                .parse::<Amount>()
                .unwrap_or_else(|_| panic!("Unable to parse genesis vesting increment"))
                .0
                .into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ledger account is identified by (token, pk). Keying the genesis accounts by pk
    /// alone dropped one of them whenever a key held both a MINA and a custom-token
    /// account -- 141 accounts on mesa, leaving the ledger short of leaves.
    #[test]
    fn a_pk_holding_two_tokens_keeps_both_accounts() -> anyhow::Result<()> {
        let pk = "B62qqdcf6K9HyBSaxqH5JVFJkc1SUEe1VzDc5kYZFQZXWSQyGHoino1";
        let custom = "xrKHGjDubYExg7mMN6BJfjjGPcMMQb9oEtFvEdA7mfSd6zKJGy";

        let ledger: Ledger = GenesisLedger::new(serde_json::from_str::<GenesisAccounts>(
            &format!(
                r#"{{"accounts": [
                    {{"pk": "{pk}", "balance": "100"}},
                    {{"pk": "{pk}", "balance": "7", "token": "{custom}"}}
                ]}}"#
            ),
        )?)
        .into();

        let mina = ledger
            .tokens
            .get(&TokenAddress::default())
            .expect("MINA ledger")
            .accounts
            .get(&pk.into())
            .expect("the MINA account survives");
        let custom = ledger
            .tokens
            .get(&custom.parse::<TokenAddress>()?)
            .expect("custom token ledger")
            .accounts
            .get(&pk.into())
            .expect("the custom-token account survives too");

        // the MINA balance carries the display creation fee, the custom-token one does not
        assert_eq!(mina.balance.0, 101 * (1e9 as u64));
        assert_eq!(custom.balance.0, 7 * (1e9 as u64));

        Ok(())
    }

    /// The dump states permissions and they are hashed into the account; the loader used
    /// to parse and discard them.
    #[test]
    fn state_dump_permissions_reach_the_account() -> anyhow::Result<()> {
        let pk = "B62qqdcf6K9HyBSaxqH5JVFJkc1SUEe1VzDc5kYZFQZXWSQyGHoino1";
        let ledger: Ledger = GenesisLedger::new(serde_json::from_str::<GenesisAccounts>(
            &format!(
                r#"{{"accounts": [{{
                    "pk": "{pk}",
                    "balance": "1",
                    "permissions": {{
                        "edit_state": "signature", "access": "none", "send": "signature",
                        "receive": "none", "set_delegate": "signature",
                        "set_permissions": "signature",
                        "set_verification_key": {{"auth": "signature", "txn_version": "2"}},
                        "set_zkapp_uri": "signature", "edit_action_state": "signature",
                        "set_token_symbol": "impossible", "increment_nonce": "signature",
                        "set_voting_for": "signature", "set_timing": "proof"
                    }}
                }}]}}"#
            ),
        )?)
        .into();

        let permissions = ledger
            .tokens
            .get(&TokenAddress::default())
            .unwrap()
            .accounts
            .get(&pk.into())
            .unwrap()
            .permissions
            .as_ref()
            .expect("permissions are stated, so they must be stored");

        assert_eq!(permissions.access, AccountPermission::None);
        assert_eq!(permissions.set_token_symbol, AccountPermission::Impossible);
        assert_eq!(permissions.set_timing, AccountPermission::Proof);
        assert_eq!(
            permissions.set_verification_key,
            (AccountPermission::Signature, "2".to_string())
        );

        Ok(())
    }

    /// The old mainnet/hardfork ledgers write `set_verification_key` as a bare
    /// `"signature"` and state no txn_version anywhere. There is nothing to reconstruct
    /// one from, so we leave those permissions unset rather than invent a version.
    #[test]
    fn legacy_permissions_are_left_unset() -> anyhow::Result<()> {
        let pk = "B62qqdcf6K9HyBSaxqH5JVFJkc1SUEe1VzDc5kYZFQZXWSQyGHoino1";
        let ledger: Ledger = GenesisLedger::new(serde_json::from_str::<GenesisAccounts>(
            &format!(
                r#"{{"accounts": [{{
                    "pk": "{pk}", "balance": "1",
                    "permissions": {{
                        "stake": true, "edit_state": "signature", "send": "signature",
                        "set_delegate": "signature", "set_permissions": "signature",
                        "set_verification_key": "signature"
                    }}
                }}]}}"#
            ),
        )?)
        .into();

        assert!(ledger
            .tokens
            .get(&TokenAddress::default())
            .unwrap()
            .accounts
            .get(&pk.into())
            .unwrap()
            .permissions
            .is_none());

        Ok(())
    }

    /// The dump writes field elements in decimal, blocks in `0x` hex. Normalise on the
    /// way in so the store holds one encoding whatever the account's provenance.
    #[test]
    fn genesis_field_elements_become_block_hex() -> anyhow::Result<()> {
        assert_eq!(
            field_to_hex("100")?,
            "0x0000000000000000000000000000000000000000000000000000000000000064"
        );
        assert_eq!(
            field_to_hex(
                "25079927036070901246064867767436987657692091363973573142121686150614948079097"
            )?,
            "0x3772BC5435B957F81F86F752E93F2E29E886AC24580B3D1EC879C1DAD26965F9"
        );

        Ok(())
    }

    /// The dump carries the verification key as bare base64 binprot, blocks as
    /// base58check. Re-encode so the two agree; the round trip must give the key back.
    #[test]
    fn genesis_verification_key_becomes_base58check() -> anyhow::Result<()> {
        use base64::Engine;

        let binprot = [0u8, 0, 156, 122, 119, 53, 200, 183, 71, 6, 110, 49];
        let base64_vk = base64::engine::general_purpose::STANDARD.encode(binprot);

        let base58 = vk_to_base58check(&base64_vk)?;
        let decoded = bs58::decode(&base58)
            .with_check(Some(VERIFICATION_KEY_VERSION_BYTE))
            .into_vec()?;

        // bs58 leaves the version byte on the front of the decoded payload
        assert_eq!(decoded[1..], binprot);

        Ok(())
    }

    /// mesa zkApp accounts carry a 32-wide app state, and the dump states it in full.
    #[test]
    fn genesis_zkapp_accounts_are_loaded() -> anyhow::Result<()> {
        let pk = "B62qqdcf6K9HyBSaxqH5JVFJkc1SUEe1VzDc5kYZFQZXWSQyGHoino1";
        let app_state = (0..32).map(|_| "\"0\"").collect::<Vec<_>>().join(",");
        let action_state = (0..5).map(|_| "\"1\"").collect::<Vec<_>>().join(",");

        let ledger: Ledger = GenesisLedger::new(serde_json::from_str::<GenesisAccounts>(
            &format!(
                r#"{{"accounts": [{{
                    "pk": "{pk}", "balance": "1",
                    "zkapp": {{
                        "app_state": [{app_state}],
                        "action_state": [{action_state}],
                        "zkapp_version": "0",
                        "last_action_slot": 904964,
                        "proved_state": false,
                        "zkapp_uri": "https://example.com"
                    }}
                }}]}}"#
            ),
        )?)
        .into();

        let zkapp = ledger
            .tokens
            .get(&TokenAddress::default())
            .unwrap()
            .accounts
            .get(&pk.into())
            .unwrap()
            .zkapp
            .as_ref()
            .expect("a stated zkApp account must be stored");

        assert_eq!(zkapp.app_state.0.len(), 32, "mesa's app state is 32 wide");
        assert_eq!(zkapp.action_state.len(), 5);
        assert_eq!(zkapp.zkapp_uri.0, "https://example.com");

        // the dump writes this one as a bare number, not a string
        assert_eq!(zkapp.last_action_slot.0, 904964);

        // the dump has no vk hash -- a verifier recomputes it from the key, and the first
        // block to touch the account supplies the real one
        assert_eq!(zkapp.verification_key.hash, VerificationKeyHash::default());

        Ok(())
    }
    use std::path::PathBuf;

    #[test]
    fn parse_v1() -> anyhow::Result<()> {
        let v1 = GenesisLedger::new_v1()?;
        let v1: Ledger = v1.into();

        assert_eq!(v1.len(), 1676);
        Ok(())
    }

    #[test]
    fn parse_v2() -> anyhow::Result<()> {
        let v2 = GenesisLedger::new_v2()?;
        let v2: Ledger = v2.into();

        assert_eq!(v2.len(), 228174);
        Ok(())
    }

    #[test]
    fn test_genesis_ledger_default_delegation_test() -> anyhow::Result<()> {
        let ledger_json = r#"{
            "genesis": {
                "genesis_state_timestamp": "2021-03-17T00:00:00Z"
            },
            "ledger": {
                "name": "mainnet",
                "accounts": [
                    {"pk": "B62qqdcf6K9HyBSaxqH5JVFJkc1SUEe1VzDc5kYZFQZXWSQyGHoino1","balance":"0"}
                ]
            }
        }"#;

        // before turning into a [Ledger]
        let root: GenesisRoot = serde_json::from_str(ledger_json)?;
        assert_eq!(
            "B62qqdcf6K9HyBSaxqH5JVFJkc1SUEe1VzDc5kYZFQZXWSQyGHoino1",
            root.ledger.accounts.first().unwrap().pk
        );
        assert_eq!(None, root.ledger.accounts.first().unwrap().delegate);

        // after turning into a [Ledger]
        let ledger = GenesisLedger::new(root.ledger);
        let account = ledger
            .ledger
            .tokens
            .get(&TokenAddress::default())
            .unwrap()
            .accounts
            .get(&"B62qqdcf6K9HyBSaxqH5JVFJkc1SUEe1VzDc5kYZFQZXWSQyGHoino1".into())
            .unwrap();

        // The delete should be the same as the public key
        assert_eq!(
            "B62qqdcf6K9HyBSaxqH5JVFJkc1SUEe1VzDc5kYZFQZXWSQyGHoino1",
            account.public_key.0
        );
        assert_eq!(
            "B62qqdcf6K9HyBSaxqH5JVFJkc1SUEe1VzDc5kYZFQZXWSQyGHoino1",
            account.delegate.0 .0
        );

        Ok(())
    }

    #[test]
    fn override_genesis_constants() -> anyhow::Result<()> {
        // no override
        let mut none_constants = ProtocolConstants::default();
        let none_path: PathBuf = "./tests/data/genesis_constants/none.json".into();

        none_constants.override_with(serde_json::from_slice::<ProtocolConstants>(
            &std::fs::read(none_path)?,
        )?);
        assert_eq!(none_constants, ProtocolConstants::default());

        // override some
        let mut some_constants = ProtocolConstants::default();
        let some_path: PathBuf = "./tests/data/genesis_constants/some.json".into();
        let some_constants_file =
            serde_json::from_slice::<ProtocolConstants>(&std::fs::read(some_path)?)?;

        some_constants.override_with(some_constants_file);
        assert_eq!(
            some_constants,
            ProtocolConstants {
                delta: Some(1),
                txpool_max_size: Some(1000),
                ..ProtocolConstants::default()
            }
        );

        // override all
        let mut all_constants = ProtocolConstants::default();
        let all_path: PathBuf = "./tests/data/genesis_constants/all.json".into();
        let all_constants_file =
            serde_json::from_slice::<ProtocolConstants>(&std::fs::read(all_path)?)?;

        all_constants.override_with(all_constants_file.clone());
        assert_eq!(all_constants, all_constants_file);

        Ok(())
    }
}
