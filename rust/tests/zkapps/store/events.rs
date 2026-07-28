use crate::{generators::*, helpers::store::*};
use mina_indexer::{
    base::{public_key::PublicKey, state_hash::StateHash},
    command::TxnHash,
    ledger::{
        diff::account::{zkapp::ZkappEventsDiff, AccountDiff},
        store::update::{AccountUpdate, DbAccountUpdate},
        token::TokenAddress,
    },
    mina_blocks::v2::{zkapp::event::ZkappEventWithMeta, ZkappEvent},
    store::{zkapp::events::ZkappEventStore, IndexerStore},
};
use quickcheck::Arbitrary;

#[test]
fn event_store_test() -> anyhow::Result<()> {
    let store_dir = setup_new_db_dir("zkapp-event-store")?;
    let indexer_store = IndexerStore::new(store_dir.path(), true)?;

    // generate arbitrary events
    let g = &mut gen();
    let events = vec![
        <TestGen<ZkappEvent>>::arbitrary(g).0,
        <TestGen<ZkappEvent>>::arbitrary(g).0,
        <TestGen<ZkappEvent>>::arbitrary(g).0,
    ];
    let events_length = events.len() as u32;

    // set block/txn
    let state_hash = StateHash::default();
    let block_height = u32::arbitrary(g);
    let txn_hash = TxnHash::default();

    // set token account
    let pk = PublicKey::default();
    let token = TokenAddress::default();

    /////////////////
    // add events //
    /////////////////

    // before
    assert_eq!(None, indexer_store.get_num_events(&pk, &token)?);

    let events_added =
        indexer_store.add_events(&pk, &token, &events, &state_hash, block_height, &txn_hash)?;
    assert_eq!(events_added, events_length);

    // after
    assert_eq!(
        events_added,
        indexer_store.get_num_events(&pk, &token)?.unwrap()
    );

    ////////////////
    // get events //
    ////////////////

    for (idx, event) in events.iter().cloned().enumerate() {
        assert_eq!(
            indexer_store.get_event(&pk, &token, idx as u32)?.unwrap(),
            ZkappEventWithMeta {
                event,
                block_height,
                txn_hash: txn_hash.clone(),
                state_hash: state_hash.clone(),
            }
        );
    }

    ///////////////
    // set event //
    ///////////////

    let index = u32::arbitrary(g);
    let index = index % events_length;

    let set_event = <TestGen<ZkappEvent>>::arbitrary(g).0;
    let set_event = ZkappEventWithMeta {
        event: set_event,
        block_height,
        txn_hash,
        state_hash,
    };

    indexer_store.set_event(&pk, &token, &set_event, index)?;
    assert_eq!(
        set_event,
        indexer_store.get_event(&pk, &token, index)?.unwrap()
    );

    ///////////////////
    // remove events //
    ///////////////////

    let num = u32::arbitrary(g);
    let num = num % events_length;

    assert_eq!(
        indexer_store.remove_events(&pk, &token, num)?,
        events_length - num
    );

    // check remaining number
    assert_eq!(
        indexer_store.get_num_events(&pk, &token)?.unwrap(),
        events_length - num
    );

    Ok(())
}

/// Fluent builder for a per-block `AccountUpdate` in tests. Lets a test read as
/// "the block at this height emitted these events" instead of restating the
/// whole ledger-update struct, so the setup shows intent, not bookkeeping.
struct BlockUpdate {
    state_hash: StateHash,
    block_height: u32,
    account_diffs: Vec<AccountDiff>,
}

impl BlockUpdate {
    fn at(state_hash: &str, block_height: u32) -> Self {
        Self {
            state_hash: state_hash.into(),
            block_height,
            account_diffs: vec![],
        }
    }

    fn emitting_events(
        mut self,
        pk: &PublicKey,
        token: &TokenAddress,
        events: &[&str],
    ) -> Self {
        self.account_diffs
            .push(AccountDiff::ZkappEvents(ZkappEventsDiff {
                token: token.clone(),
                public_key: pk.clone(),
                events: events.iter().copied().map(ZkappEvent::from).collect(),
                txn_hash: TxnHash::default(),
            }));
        self
    }

    fn build(self) -> AccountUpdate {
        AccountUpdate {
            account_diffs: self.account_diffs,
            token_diffs: vec![],
            new_accounts: Default::default(),
            new_zkapp_accounts: Default::default(),
            accounts_accessed: vec![],
            state_hash: self.state_hash,
            block_height: self.block_height,
        }
    }
}

/// Regression for mina-indexer#126.
///
/// `apply_updates` takes a batch of per-block `AccountUpdate`s and the best-tip
/// state hash. When the batch spans more than one block (a reorg or a catch-up
/// advance), every update's events must be keyed to *its own* block, not the
/// tip. The pre-fix code keyed them all to the tip, so an event landed under a
/// block that never held its emitting command and the events resolver then
/// failed the whole query with "no command <hash> in <block>".
#[test]
fn apply_updates_keys_events_to_their_own_block() -> anyhow::Result<()> {
    const BLOCK_A: &str = "3NLXXQ1ZtzPMb1Tcx2mLdAUEKgL8bWH4qdFPdwsUKMpJQ7hNAwfW";
    const BLOCK_B: &str = "3NKAwFdBbSGEcsqokVhdLoHm3kN4eDiV9akSQX2XMGeeXbikopjd";

    let db = IndexerStore::new(setup_new_db_dir("apply-updates-event-attribution")?.path(), true)?;
    let (pk, token) = (PublicKey::default(), TokenAddress::default());

    // valid 66-char `0x…` event field elements, one per block
    let ev_a = format!("0x{:064x}", 0xAAAA_u32);
    let ev_b = format!("0x{:064x}", 0xBBBB_u32);

    // a two-block canonical segment, applied under a *different* best tip
    let best_tip: StateHash = "3NLMSgtHY8eToF5P1y7ai6bpkfZ2vZauGnLyUCme4LjGSbu4pUTp".into();
    DbAccountUpdate::apply_updates(
        &db,
        vec![
            BlockUpdate::at(BLOCK_A, 531_667)
                .emitting_events(&pk, &token, &[&ev_a])
                .build(),
            BlockUpdate::at(BLOCK_B, 531_668)
                .emitting_events(&pk, &token, &[&ev_b])
                .build(),
        ],
        &best_tip,
    )?;

    // each event keeps its own block's provenance, never the best tip's
    let e0 = db.get_event(&pk, &token, 0)?.unwrap();
    let e1 = db.get_event(&pk, &token, 1)?.unwrap();

    assert_eq!(e0.state_hash, BLOCK_A.into(), "event 0 keeps block 531667");
    assert_eq!(e0.block_height, 531_667);
    assert_eq!(e1.state_hash, BLOCK_B.into(), "event 1 keeps block 531668");
    assert_eq!(e1.block_height, 531_668);
    assert_ne!(e0.state_hash, best_tip, "not keyed to the best tip");

    Ok(())
}
