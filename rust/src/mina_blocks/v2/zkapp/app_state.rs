//! Zkapp state

use crate::constants::ZKAPP_STATE_FIELD_ELEMENTS_NUM;
use serde::{Deserialize, Serialize};

/// zkApp on-chain state. Variable-length to support protocols with different
/// field counts: mainnet/devnet use [`ZKAPP_STATE_FIELD_ELEMENTS_NUM`] (8),
/// while the mesa protocol (transaction version 3) uses 32. The `Default` is
/// the 8-field all-zero state used by mainnet/devnet new-account creation.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Hash, Serialize, Deserialize)]
pub struct ZkappState(pub Vec<AppState>);

impl Default for ZkappState {
    fn default() -> Self {
        Self(vec![AppState::default(); ZKAPP_STATE_FIELD_ELEMENTS_NUM])
    }
}

/// 32 bytes
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Hash, Serialize, Deserialize)]
pub struct AppState(pub String);

//////////
// impl //
//////////

impl AppState {
    pub const PREFIX: &'static str = "0x";

    // 32 bytes = 64 hex + 2 prefix chars
    pub const LEN: usize = 66;
}

/////////////////
// conversions //
/////////////////

impl<T> From<T> for AppState
where
    T: Into<String>,
{
    fn from(value: T) -> Self {
        let app_state: String = value.into();

        assert!(app_state.starts_with(Self::PREFIX));
        assert_eq!(app_state.len(), Self::LEN);

        Self(app_state)
    }
}

/////////////
// default //
/////////////

impl std::default::Default for AppState {
    fn default() -> Self {
        Self("0x0000000000000000000000000000000000000000000000000000000000000000".to_string())
    }
}

/////////////
// display //
/////////////

impl std::fmt::Display for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
impl quickcheck::Arbitrary for AppState {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        let mut bytes = [0u8; 32];

        for byte in bytes.iter_mut() {
            *byte = u8::arbitrary(g);
        }

        Self(format!("0x{}", hex::encode(bytes)))
    }
}

#[cfg(test)]
impl quickcheck::Arbitrary for ZkappState {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        Self(
            (0..ZKAPP_STATE_FIELD_ELEMENTS_NUM)
                .map(|_| AppState::arbitrary(g))
                .collect(),
        )
    }
}
