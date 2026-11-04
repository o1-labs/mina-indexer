use crate::{generators::*, helpers::store::*};
use mina_indexer::{
    base::{public_key::PublicKey, state_hash::StateHash},
    command::TxnHash,
    ledger::token::TokenAddress,
    mina_blocks::v2::zkapp::action_state::{ActionState, ActionStateWithMeta},
    store::{zkapp::actions::ZkappActionStore, IndexerStore},
    utility::store::zkapp::actions::zkapp_action_index,
};
use quickcheck::Arbitrary;
use speedb::Direction;

#[test]
fn action_store_test() -> anyhow::Result<()> {
    let store_dir = setup_new_db_dir("zkapp-action-store")?;
    let indexer_store = IndexerStore::new(store_dir.path(), true)?;

    // generate arbitrary actions
    let g = &mut gen();
    let actions = vec![
        <TestGen<ActionState>>::arbitrary(g).0,
        <TestGen<ActionState>>::arbitrary(g).0,
        <TestGen<ActionState>>::arbitrary(g).0,
    ];
    let actions_length = actions.len() as u32;

    // set block/txn
    let state_hash = StateHash::default();
    let block_height = u32::arbitrary(g);
    let txn_hash = TxnHash::default();

    // set token account
    let pk = PublicKey::default();
    let token = TokenAddress::default();

    /////////////////
    // add actions //
    /////////////////

    // before
    assert_eq!(None, indexer_store.get_num_actions(&pk, &token)?);

    let actions_added =
        indexer_store.add_actions(&pk, &token, &actions, &state_hash, block_height, &txn_hash)?;
    assert_eq!(actions_added, actions_length);

    // after
    assert_eq!(
        actions_added,
        indexer_store.get_num_actions(&pk, &token)?.unwrap()
    );

    /////////////////
    // get actions //
    /////////////////

    for (idx, action) in actions.iter().cloned().enumerate() {
        assert_eq!(
            indexer_store.get_action(&pk, &token, idx as u32)?.unwrap(),
            ActionStateWithMeta {
                action,
                block_height,
                txn_hash: txn_hash.clone(),
                state_hash: state_hash.clone(),
            }
        );
    }

    ////////////////
    // set action //
    ////////////////

    let index = u32::arbitrary(g);
    let index = index % actions_length;

    let set_action = <TestGen<ActionState>>::arbitrary(g).0;
    let set_action = ActionStateWithMeta {
        action: set_action,
        txn_hash,
        state_hash,
        block_height,
    };

    indexer_store.set_action(&pk, &token, &set_action, index)?;
    assert_eq!(
        set_action,
        indexer_store.get_action(&pk, &token, index)?.unwrap()
    );

    ////////////////////
    // remove actions //
    ////////////////////

    let num = u32::arbitrary(g);
    let num = num % actions_length;

    assert_eq!(
        indexer_store.remove_actions(&pk, &token, num)?,
        actions_length - num
    );

    // check remaining number
    assert_eq!(
        indexer_store.get_num_actions(&pk, &token)?.unwrap(),
        actions_length - num
    );

    Ok(())
}

/// Keys sort by (token, pk, index), so an iterator with no bound runs into the
/// neighbouring accounts. It must only yield the requested token account.
#[test]
fn actions_iterator_stays_inside_the_token_account() -> anyhow::Result<()> {
    let store_dir = setup_new_db_dir("zkapp-actions-iterator-bound")?;
    let indexer_store = IndexerStore::new(store_dir.path(), true)?;
    let g = &mut gen();

    let token = TokenAddress::default();
    // in key order
    let [before, emitter, silent, after] = [
        "B62qjSytpSK7aEauBprjXDSZwc9ai4YMv9tpmXLQK14Vy941YV36rMz",
        "B62qjYanmV7y9njVeH5UHkz3GYBm7xKir1rAnoY4KsEYUGLMiU45FSM",
        "B62qjwDWxjf4LtJ4YWJQDdTNPqZ69ZyeCzbpAFKN7EoZzYig5ZRz8JE",
        "B62qjzJvc59DdG9ahht9rwxkEz7GedKuUMsnaVTuXFUeANKqfBeWpRE",
    ]
    .map(PublicKey::from);

    let state_hash = StateHash::default();
    let txn_hash = TxnHash::default();
    for pk in [&before, &emitter, &after] {
        let actions: Vec<ActionState> = (0..3)
            .map(|_| <TestGen<ActionState>>::arbitrary(g).0)
            .collect();
        indexer_store.add_actions(pk, &token, &actions, &state_hash, 1, &txn_hash)?;
    }

    let indices = |pk: &PublicKey, index, direction| -> Vec<u32> {
        indexer_store
            .actions_iterator(pk, &token, index, direction)
            .flatten()
            .map(|(key, _)| zkapp_action_index(&key))
            .collect()
    };

    assert_eq!(indices(&emitter, None, Direction::Forward), [0, 1, 2]);
    assert_eq!(indices(&emitter, None, Direction::Reverse), [2, 1, 0]);
    assert_eq!(indices(&emitter, Some(1), Direction::Forward), [1, 2]);
    assert_eq!(indices(&emitter, Some(1), Direction::Reverse), [1, 0]);

    // an account with no actions has no rows, in either direction
    assert!(indices(&silent, None, Direction::Forward).is_empty());
    assert!(indices(&silent, None, Direction::Reverse).is_empty());

    Ok(())
}
