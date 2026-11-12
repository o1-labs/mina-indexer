//! Version store trait

use crate::constants::GIT_COMMIT_HASH;
use serde::{Deserialize, Serialize};

pub trait VersionStore {
    /// Set db version with env var `GIT_COMMIT_HASH`
    fn set_db_version_with_git_commit(
        &self,
        major: u32,
        minor: u32,
        patch: u32,
    ) -> anyhow::Result<()>;

    /// Get db version
    fn get_db_version(&self) -> anyhow::Result<IndexerStoreVersion>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerStoreVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub git_commit_sha: String,
}

impl IndexerStoreVersion {
    pub const MAJOR: u32 = 0;
    // 18: `PcbVersion::V2` carries the producer's currency encoding, so the
    // block version column family reads `{"V2":"Nanomina"}` where it used to
    // read `"V2"`. Databases built before this also hold devnet balances that
    // were a billionth of their real size, and have to be rebuilt regardless.
    // 19: adds the `zkapp-verification-key-history` column family (per-account
    // VK change log, keyed by block height + state hash). Databases built
    // before this have no VK-history index and must be rebuilt to serve
    // `verificationKeyHistory` / `lastVerificationKeyChange`.
    // 20: a zkApp account update keeps all of its events and actions. Databases
    // built before this kept only the first of each update, and the rest cannot
    // be recovered without replaying the blocks. zkApp commands are also keyed
    // by the protocol's transaction hash, where they had a stand-in digest.
    // 21: fee transfers are stored as the protocol's net internal commands,
    // with the fee transfers via coinbase. Databases built before this hold
    // gross fee flows in blocks with SNARK work and must be rebuilt.
    pub const MINOR: u32 = 21;
    pub const PATCH: u32 = 0;

    /// Output as `MAJOR`.`MINOR`.`PATCH`
    pub fn major_minor_patch(&self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }

    /// Whether this binary can open a store of this version as-is
    pub fn matches_binary(&self) -> bool {
        self.major == Self::MAJOR && self.minor == Self::MINOR && self.patch == Self::PATCH
    }
}

impl std::default::Default for IndexerStoreVersion {
    fn default() -> Self {
        Self {
            major: Self::MAJOR,
            minor: Self::MINOR,
            patch: Self::PATCH,
            git_commit_sha: GIT_COMMIT_HASH.to_string(),
        }
    }
}

impl std::fmt::Display for IndexerStoreVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.major_minor_patch(), self.git_commit_sha)
    }
}
