use crate::base::amount::Amount;
use chrono::{DateTime, SecondsFormat, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

// version

pub const GIT_COMMIT_HASH: &str = env!("GIT_COMMIT_HASH");
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-", env!("GIT_COMMIT_HASH"));

// indexer constants

pub const BLOCK_REPORTING_FREQ_NUM: u32 = 1000;
pub const BLOCK_REPORTING_FREQ_SEC: u64 = 180;
pub const LEDGER_CADENCE: u32 = 1000;
pub const CANONICAL_UPDATE_THRESHOLD: u32 = PRUNE_INTERVAL_DEFAULT / 5;
pub const MAINNET_CANONICAL_THRESHOLD: u32 = 10;
pub const PRUNE_INTERVAL_DEFAULT: u32 = 10;

// mina constants

pub const MINA_SCALE: u64 = 1_000_000_000;
pub const MINA_SCALE_DEC: Decimal = dec!(1_000_000_000);

pub const MINA_TOKEN_ADDRESS: &str = "wSHV2S4qX9jFsLjQo8r1BsMLH2ZRKsZx6EJd1sbozGPieEC4Jf";
pub const MINA_TOKEN_ID: u64 = 1;

pub const MAINNET_BLOCK_SLOT_TIME_MILLIS: u64 = 180000;
pub const MAINNET_TRANSITION_FRONTIER_K: u32 = 290;
pub const MAINNET_ACCOUNT_CREATION_FEE: Amount = Amount(1e9 as u64);
pub const MAINNET_COINBASE_REWARD: u64 = 720000000000;

pub const MAINNET_GENESIS_HASH: &str = "3NKeMoncuHab5ScarV5ViyF16cJPT4taWNSaTLS64Dp67wuXigPZ";
pub const MAINNET_GENESIS_PREV_STATE_HASH: &str =
    "3NLoKn22eMnyQ7rxh5pxB6vBA3XhSAhhrf7akdqS6HbAKD14Dh1d";
pub const MAINNET_GENESIS_LAST_VRF_OUTPUT: &str = "NfThG1r1GxQuhaGLSJWGxcpv24SudtXG4etB0TnGqwg=";
pub const MAINNET_GENESIS_TIMESTAMP: u64 = 1615939200000;
pub const MAINNET_GENESIS_LEDGER_HASH: &str = "jx7buQVWFLsXTtzRgSxbYcT8EYLS8KCZbLrfDcJxMtyy4thw2Ee";

// protocol constants

pub const MAINNET_PROTOCOL_CONSTANTS: &[u32] = &[
    MAINNET_TRANSITION_FRONTIER_K,
    MAINNET_EPOCH_SLOT_COUNT,
    MAINNET_SLOTS_PER_SUB_WINDOW,
    MAINNET_DELTA,
    MAINNET_TXPOOL_MAX_SIZE,
];
pub const MAINNET_EPOCH_SLOT_COUNT: u32 = 7140;
pub const MAINNET_SLOTS_PER_SUB_WINDOW: u32 = 7;
pub const MAINNET_DELTA: u32 = 0;
pub const MAINNET_TXPOOL_MAX_SIZE: u32 = 3000;

// constraint system digests

pub const MAINNET_CONSTRAINT_SYSTEM_DIGESTS: &[&str] = &[
    MAINNET_DIGEST_TXN_MERGE,
    MAINNET_DIGEST_TXN_BASE,
    MAINNET_DIGEST_BLOCKCHAIN_STEP,
];
pub const MAINNET_DIGEST_TXN_MERGE: &str = "d0f8e5c3889f0f84acac613f5c1c29b1";
pub const MAINNET_DIGEST_TXN_BASE: &str = "922bd415f24f0958d610607fc40ef227";
pub const MAINNET_DIGEST_BLOCKCHAIN_STEP: &str = "06d85d220ad13e03d51ef357d2c9d536";

pub const MAINNET_CHAIN_ID: &str =
    "5f704cc0c82e0ed70e873f0893d7e06f148524e3f0bdae2afb02e7819a0c24d1";

pub const MAINNET_LAST_GLOBAL_SLOT: u32 = 564479;

// post hardfork

pub const HARDFORK_GENESIS_BLOCKCHAIN_LENGTH: u32 = 359605;
pub const HARDFORK_GENESIS_GLOBAL_SLOT: u32 = 564480;
pub const HARDFORK_GENESIS_HASH: &str = "3NK4BpDSekaqsG6tx8Nse2zJchRft2JpnbvMiog55WCr5xJZaKeP";
pub const HARDFORK_GENESIS_TIMESTAMP: u64 = 1717545600000;
pub const HARDFORK_GENESIS_PREV_STATE_HASH: &str =
    "3NLRTfY4kZyJtvaP4dFenDcxfoMfT3uEpkWS913KkeXLtziyVd15";
pub const HARDFORK_GENESIS_LEDGER_HASH: &str =
    "jwNw4qb6tnNhpQNxiMLem9WumxZTwmbSx3fYXW4FP3hZRkoQJSE";
pub const HARDFORK_GENESIS_LAST_VRF_OUTPUT: &str =
    "FSBXKqZKgSiy1T6SsjbrT0i84oDkBpUVsLH1zRviuIj0DjuGEXs=";

// mesa-mut hardfork network (protocol transaction version 3).
//
// Values are taken from the fork block `mina-mesa-mut-1-297734-3NLp6dKN…json`
// (the genesis/root of the mesa-mut chain). The genesis ledger is supplied at
// runtime via `--genesis-ledger` (the state dump). See the `ops/mesa-mut`
// tooling and the embedded genesis block in `block::genesis`.
pub const MESA_GENESIS_BLOCKCHAIN_LENGTH: u32 = 297735;
pub const MESA_GENESIS_GLOBAL_SLOT: u32 = 449660;
pub const MESA_GENESIS_HASH: &str = "3NKQttwm8QRdvSZL62Lid8YAPCXBuAucZPDT8mJriHmw2qk9cVcr";
// The pre-fork chain's genesis hash, which mesa-mut blocks carry in their
// `genesis_state_hash` field for protocol continuity. The indexer remaps it to
// MESA_GENESIS_HASH so the whole mesa chain shares one genesis for canonicity.
pub const MESA_ORIGINAL_GENESIS_HASH: &str =
    "3NL4ZJ3SEfc7yZwiyh6otjopKGgfFhGmU2R3HQbjBuXCFtLJQcoY";
pub const MESA_GENESIS_PREV_STATE_HASH: &str =
    "3NLp6dKNhYtsqUj49QYV5GtDaeocSJBAa2y2ER2QQLqLukE3wuZT";
pub const MESA_GENESIS_LEDGER_HASH: &str = "jxicjVogngTDjJh5EEsTUrvBxa3R4fhepqrAeexiRVMogJGqHdT";
pub const MESA_GENESIS_LAST_VRF_OUTPUT: &str = "8oxYNPIKw0xNLJJrhcXRICHIS34t4z-8fsvfTfSbIAA=";
// Placeholder chain id (valid 64-char hex). mesa-mut's real chain id depends on
// its constraint-system digests, which are not needed for indexing block data.
pub const MESA_CHAIN_ID: &str =
    "6d6573612d6d75740000000000000000000000000000000000000000000000aa";

pub const HARDFORK_CONSTRAINT_SYSTEM_DIGESTS: &[&str] = &[
    HARDFORK_DIGEST_TXN_MERGE,
    HARDFORK_DIGEST_TXN_BASE,
    HARDFORK_DIGEST_BLOCKCHAIN_STEP,
];
pub const HARDFORK_DIGEST_TXN_MERGE: &str = "b8879f677f622a1d86648030701f43e1";
pub const HARDFORK_DIGEST_TXN_BASE: &str = "d31948e661cc662675b0c079458f714a";
pub const HARDFORK_DIGEST_BLOCKCHAIN_STEP: &str = "14ab5562ed292de7a3deb9e12f00aec0";

pub const HARDFORK_PROTOCOL_NETWORK_VERSION_DIGEST: &str = "eccbc87e4b5ce2fe28308fd9f2a7baf3";
pub const HARDFORK_PROTOCOL_TXN_VERSION_DIGEST: &str = "eccbc87e4b5ce2fe28308fd9f2a7baf3";

pub const HARDFORK_CHAIN_ID: &str =
    "a7351abc7ddf2ea92d1b38cc8e636c271c1dfd2c081c637f62ebc2af34eb7cc1";

pub const ZKAPP_STATE_FIELD_ELEMENTS_NUM: usize = 8;

// Name service constants
pub const MINA_EXPLORER_NAME_SERVICE_ADDRESS: &str =
    "B62qjzJvc59DdG9ahht9rwxkEz7GedKuUMsnaVTuXFUeANKqfBeWpRE";
pub const MINA_SEARCH_NAME_SERVICE_ADDRESS: &str =
    "B62qjMINASEARCHMINASEARCHMINASEARCHMINASEARCHMINASEARCH";
pub const NAME_SERVICE_MEMO_PREFIX: &str = "Name: ";

/// Convert epoch milliseconds to an ISO 8601 formatted date
pub fn millis_to_iso_date_string(millis: i64) -> String {
    from_timestamp_millis(millis).to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Convert epoch milliseconds to DateTime<Utc>
pub fn from_timestamp_millis(millis: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(millis).unwrap()
}

/// Convert epoch milliseconds to global slot number
pub fn millis_to_global_slot(millis: i64) -> u32 {
    let millis_since_genesis = millis as u64 - MAINNET_GENESIS_TIMESTAMP;
    (millis_since_genesis / MAINNET_BLOCK_SLOT_TIME_MILLIS) as u32
}

/// Get the epoch slot number from the global slot number
pub fn epoch_slot(global_slot: u32) -> u32 {
    if global_slot <= MAINNET_LAST_GLOBAL_SLOT {
        global_slot % MAINNET_EPOCH_SLOT_COUNT
    } else {
        (global_slot - 420) % MAINNET_EPOCH_SLOT_COUNT
    }
}

pub mod berkeley {
    pub const BERKELEY_GENESIS_STATE_HASH: &str =
        "3NK512ryRJvj1TUKGgPoGZeHSNbn37e9BbnpyeqHL9tvKLeD8yrY";
    pub const BERKELEY_GENESIS_TIMESTAMP: u64 = 1706882461000;
}

pub const DEFAULT_WEB_HOSTNAME: &str = "0.0.0.0";
pub const DEFAULT_WEB_PORT: u16 = 8080;
