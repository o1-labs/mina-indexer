//! `get_staged_accounts` reads many staged accounts in one walk; it must agree
//! with `get_staged_account` account by account

use crate::helpers::{state::*, store::*};
use mina_indexer::{
    base::public_key::PublicKey,
    block::{
        parser::{BlockParser, ParsedBlock},
        store::BlockStore,
    },
    ledger::{store::staged::StagedLedgerStore, token::TokenAddress},
};
use std::{collections::HashSet, path::PathBuf};

#[tokio::test]
async fn batch_matches_single_lookups() -> anyhow::Result<()> {
    let store_dir = setup_new_db_dir("staged-accounts-batch")?;
    let block_dir = PathBuf::from("./tests/data/canonical_chain_discovery/contiguous");

    let mut block_parser = BlockParser::new_testing(&block_dir)?;
    let mut state = mainnet_genesis_state(store_dir.as_ref())?;
    state.add_blocks(&mut block_parser).await?;
    let store = state.indexer_store.as_ref().unwrap();

    // accounts the blocks touch, some genesis accounts, and one that never existed
    let token = TokenAddress::default();
    let mut accounts: HashSet<(PublicKey, TokenAddress)> = HashSet::new();
    let mut block_parser = BlockParser::new_testing(&block_dir)?;
    let mut state_hashes = vec![];
    while let Some((block, _)) = block_parser.next_block().await? {
        let (ParsedBlock::Recent(block)
        | ParsedBlock::DeepCanonical(block)
        | ParsedBlock::Orphaned(block)) = block;
        state_hashes.push(block.state_hash());
        for pk in block.all_command_public_keys() {
            accounts.insert((pk, token.clone()));
        }
    }
    for pk in state.ledger.tokens[&token].accounts.keys().take(20) {
        accounts.insert((pk.clone(), token.clone()));
    }
    accounts.insert((
        PublicKey::from("B62qrecVjpoZ4Re3a5arN6gXZ6orhmj1enUtA887XdG5mtZfdUbBUh4"),
        token.clone(),
    ));
    assert!(accounts.len() > 20);

    for state_hash in state_hashes
        .iter()
        .filter(|h| store.get_block_height(h).ok().flatten().is_some())
    {
        let batch = store.get_staged_accounts(&accounts, state_hash)?;
        assert_eq!(batch.len(), accounts.len());

        for (pk, token) in accounts.iter() {
            let single = store.get_staged_account(pk, token, state_hash)?;
            assert_eq!(
                batch[&(pk.clone(), token.clone())],
                single,
                "staged account {pk} at {state_hash}"
            );
        }
    }

    Ok(())
}
