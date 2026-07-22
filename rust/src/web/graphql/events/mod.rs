//! GraphQL `events` endpoint

use super::{block::BlockInfo, db, txn::TxnInfo};
use crate::{
    base::public_key::PublicKey,
    block::store::BlockStore,
    canonicity::store::CanonicityStore,
    command::store::UserCommandStore,
    ledger::token::TokenAddress,
    mina_blocks::v2::zkapp::event::ZkappEventWithMeta,
    store::{zkapp::events::ZkappEventStore, IndexerStore},
    utility::store::zkapp::events::zkapp_event_index,
};
use async_graphql::{Context, Enum, InputObject, Object, Result, SimpleObject};
use speedb::Direction;

#[derive(InputObject, Debug)]
pub struct EventsQueryInput {
    /// Input public key
    pub public_key: String,

    /// Input token address
    pub token: Option<String>,

    /// Input start block height
    pub start_block_height: Option<u32>,

    /// Input end block height
    pub end_block_height: Option<u32>,

    /// Input start event index
    pub start_event_index: Option<u32>,

    /// Input end event index
    pub end_event_index: Option<u32>,
}

#[derive(Default, Enum, Copy, Clone, Debug, Eq, PartialEq)]
pub enum EventsSortByInput {
    #[default]
    BlockHeightDesc,
    BlockHeightAsc,
}

/// Value event
#[derive(SimpleObject, Debug)]
pub struct Event {
    /// Value event data
    pub event: String,

    /// Value event txn
    pub txn: TxnInfo,

    /// Value event block
    pub block: BlockInfo,
}

#[derive(Default)]
pub struct EventsQueryRoot;

#[Object]
impl EventsQueryRoot {
    // Cache for 1 hour
    #[graphql(cache_control(max_age = 3600))]
    async fn events(
        &self,
        ctx: &Context<'_>,
        query: EventsQueryInput,
        sort_by: Option<EventsSortByInput>,
        #[graphql(default = 100)] limit: usize,
        // `offset`: matching events to skip before `limit`. Pair with
        // `eventsCount(query)` for total-count / page math.
        #[graphql(default = 0)] offset: usize,
    ) -> Result<Option<Vec<Event>>> {
        let limit = limit.min(crate::constants::GRAPHQL_MAX_PAGE_SIZE);
        let db = db(ctx);

        let (public_key, token) = query.validate()?;

        let direction = match sort_by.unwrap_or_default() {
            EventsSortByInput::BlockHeightAsc => Direction::Forward,
            EventsSortByInput::BlockHeightDesc => Direction::Reverse,
        };
        let index = match direction {
            Direction::Forward => query.start_event_index,
            Direction::Reverse => query.end_event_index,
        };

        let mut events = Vec::with_capacity(limit);
        let mut skipped = 0;
        for (key, value) in db
            .events_iterator(&public_key, &token, index, direction)
            .flatten()
        {
            if events.len() >= limit {
                break;
            }

            let index = zkapp_event_index(&key);
            let event: ZkappEventWithMeta = serde_json::from_slice(&value)?;

            if query.matches(&event, index) {
                if skipped < offset {
                    skipped += 1;
                    continue;
                }
                events.push(Event::new(db, event)?);
            }
        }

        Ok(Some(events))
    }

    /// Total events matching `query` -- the count companion to `events`, for a
    /// gateway to compute total pages. Applies the same `matches` filter over the
    /// same iterator, but does not build each `Event` (which does per-event
    /// block/command lookups), so it stays cheap.
    #[graphql(cache_control(max_age = 3600))]
    async fn events_count(&self, ctx: &Context<'_>, query: EventsQueryInput) -> Result<u32> {
        let db = db(ctx);
        let (public_key, token) = query.validate()?;

        let mut count = 0u32;
        for (key, value) in db
            .events_iterator(&public_key, &token, None, Direction::Forward)
            .flatten()
        {
            let index = zkapp_event_index(&key);
            let event: ZkappEventWithMeta = serde_json::from_slice(&value)?;
            if query.matches(&event, index) {
                count += 1;
            }
        }
        Ok(count)
    }
}

impl Event {
    fn new(db: &IndexerStore, event: ZkappEventWithMeta) -> async_graphql::Result<Self> {
        // These lookups reference the block/command the event came from, so they
        // should always resolve; return a GraphQL error rather than panicking if
        // the store is ever inconsistent (e.g. the block was pruned).
        let canonicity = db
            .get_block_canonicity(&event.state_hash)?
            .ok_or_else(|| {
                async_graphql::Error::new(format!("no canonicity for {}", event.state_hash))
            })?
            .to_string();
        let global_slot = db
            .get_block_global_slot(&event.state_hash)?
            .ok_or_else(|| {
                async_graphql::Error::new(format!("no global slot for {}", event.state_hash))
            })?;

        let cmd = db
            .get_user_command_state_hash(&event.txn_hash, &event.state_hash)?
            .ok_or_else(|| {
                async_graphql::Error::new(format!(
                    "no command {} in {}",
                    event.txn_hash, event.state_hash
                ))
            })?;
        let memo = cmd.command.memo();
        let status = format!("{:?}", cmd.status);

        Ok(Self {
            event: event.event.0,
            txn: TxnInfo {
                memo,
                status,
                txn_hash: event.txn_hash.to_string(),
            },
            block: BlockInfo {
                canonicity,
                global_slot,
                state_hash: event.state_hash.0,
                height: event.block_height,
            },
        })
    }
}

impl EventsQueryInput {
    /// Validate + parse the public key and token, shared by `events` and
    /// `eventsCount` so both reject the same inputs identically.
    fn validate(&self) -> Result<(PublicKey, TokenAddress)> {
        let public_key = PublicKey::new(&self.public_key).map_err(|_| {
            async_graphql::Error::new(format!("Invalid public key: {}", &self.public_key))
        })?;
        let token = match self.token.as_ref() {
            Some(token) => TokenAddress::new(token)
                .ok_or_else(|| async_graphql::Error::new(format!("Invalid token: {}", token)))?,
            None => TokenAddress::default(),
        };
        Ok((public_key, token))
    }

    fn matches(&self, event: &ZkappEventWithMeta, index: u32) -> bool {
        let Self {
            public_key: _,
            token: _,
            start_block_height,
            end_block_height,
            start_event_index,
            end_event_index,
        } = self;

        // block height
        if let Some(start_block_height) = start_block_height {
            if event.block_height < *start_block_height {
                return false;
            }
        }

        if let Some(end_block_height) = end_block_height {
            if event.block_height >= *end_block_height {
                return false;
            }
        }

        // index
        if let Some(start_event_index) = start_event_index {
            if index < *start_event_index {
                return false;
            }
        }

        if let Some(end_event_index) = end_event_index {
            if index >= *end_event_index {
                return false;
            }
        }

        true
    }
}
