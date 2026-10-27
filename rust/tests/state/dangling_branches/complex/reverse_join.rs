use mina_indexer::{
    block::parser::BlockParser,
    state::{ExtensionType, IndexerState},
};
use std::path::PathBuf;

/// A block that is the new root of one dangling branch and extends another
/// dangling branch forward joins the two, so both merge into the root branch
#[tokio::test]
async fn extension() -> anyhow::Result<()> {
    // -----------------------------
    //    Before    |    After
    // -----------------------------
    //  root 105489 =>  root 105489
    //              =>      |
    //  [0] 105493  =>    105490
    //  [1] 105491  =>      |
    //              =>    105491
    //              =>      |
    //              =>    105492
    //              =>      |
    //              =>    105493
    // -----------------------------
    // 105492 arrives after both dangling branches: it is the parent of [0]'s
    // root and a child of [1]'s tip. 105490 then connects all to the root.

    let blocks_dir = PathBuf::from("./tests/data/sequential_blocks");
    let mut block_parser = BlockParser::new_testing(&blocks_dir)?;

    let (root_block, root_block_bytes) = block_parser
        .get_precomputed_block("3NK4huLvUDiL4XuCUcyrWCKynmvhqfKsx5h2MfBXVVUq2Qwzi5uT")
        .await?;
    let (block_105490, _) = block_parser
        .get_precomputed_block("3NKxEA9gztvEGxL4uk4eTncZAxuRmMsB8n81UkeAMevUjMbLHmkC")
        .await?;
    let (block_105491, _) = block_parser
        .get_precomputed_block("3NKizDx3nnhXha2WqHDNUvJk9jW7GsonsEGYs26tCPW2Wow1ZoR3")
        .await?;
    let (block_105492, _) = block_parser
        .get_precomputed_block("3NKAqzELKDp2BbdKKwdRWEoMNehyMrxJGCoGCyH1t1PyyH7VQMgk")
        .await?;
    let (block_105493, _) = block_parser
        .get_precomputed_block("3NKakum3B2Tigw9TSsxwvXvV3x8L2LvrJ3yXFLEAJDMZu2vkn7db")
        .await?;

    let mut state =
        IndexerState::new_testing(&root_block, root_block_bytes, None, None, None, false)?;

    let (extension_type, _) = state.add_block_to_witness_tree(&block_105493, true, true)?;
    assert_eq!(extension_type, ExtensionType::DanglingNew);
    let (extension_type, _) = state.add_block_to_witness_tree(&block_105491, true, true)?;
    assert_eq!(extension_type, ExtensionType::DanglingNew);
    assert_eq!(state.dangling_branches.len(), 2);

    // the new root of [0] is a forward extension of [1]: they join
    let (extension_type, _) = state.add_block_to_witness_tree(&block_105492, true, true)?;
    assert_eq!(extension_type, ExtensionType::DanglingComplex);
    assert_eq!(state.dangling_branches.len(), 1);
    assert_eq!(state.dangling_branches[0].len(), 3);

    // the joined branch merges into the root branch as a whole
    let (extension_type, _) = state.add_block_to_witness_tree(&block_105490, true, true)?;
    assert!(matches!(extension_type, ExtensionType::RootComplex(_)));
    assert!(state.dangling_branches.is_empty());
    assert_eq!(state.root_branch.len(), 5);
    assert_eq!(state.best_tip_block().state_hash, block_105493.state_hash());

    Ok(())
}
