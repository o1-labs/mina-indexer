use mina_indexer::{
    block::parser::BlockParser,
    client::{ClientCli, BIN_CODE_CONFIG},
    state::IndexerState,
    unix_socket_server::{create_socket_listener, handle_connection, ServerCliResponse},
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    sync::RwLock,
};
use tokio_graceful_shutdown::{SubsystemBuilder, Toplevel};

/// A client that hangs up without a response must not stop the listener: the
/// next client still gets its answer
#[tokio::test]
async fn client_hang_up_does_not_stop_listener() -> anyhow::Result<()> {
    let blocks_dir = PathBuf::from("./tests/data/sequential_blocks");
    let mut block_parser = BlockParser::new_testing(&blocks_dir)?;
    let (root_block, root_block_bytes) = block_parser
        .get_precomputed_block("3NK4huLvUDiL4XuCUcyrWCKynmvhqfKsx5h2MfBXVVUq2Qwzi5uT")
        .await?;
    let state = IndexerState::new_testing(&root_block, root_block_bytes, None, None, None, false)?;
    let state = Arc::new(RwLock::new(state));

    let socket_dir = tempfile::tempdir()?;
    let socket = socket_dir.path().join("mi.sock");
    let listener = create_socket_listener(&socket);

    let toplevel = tokio::spawn(
        Toplevel::new(move |s| async move {
            s.start(SubsystemBuilder::new("Socket Listener", move |subsys| {
                handle_connection(listener, state, subsys)
            }));
        })
        .handle_shutdown_requests(Duration::from_secs(5)),
    );

    // hang up before sending a command
    drop(UnixStream::connect(&socket).await?);

    // send a command and hang up before the response
    let mut stream = UnixStream::connect(&socket).await?;
    stream
        .write_all(&bincode::encode_to_vec(
            &ClientCli::DbVersion,
            BIN_CODE_CONFIG,
        )?)
        .await?;
    drop(stream);

    // the listener still serves the next client
    let mut stream = UnixStream::connect(&socket).await?;
    stream
        .write_all(&bincode::encode_to_vec(
            &ClientCli::DbVersion,
            BIN_CODE_CONFIG,
        )?)
        .await?;
    let mut buffer = vec![];
    tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut buffer)).await??;
    let (response, _): (ServerCliResponse, usize) =
        bincode::decode_from_slice(&buffer, BIN_CODE_CONFIG)?;
    assert!(matches!(response, ServerCliResponse::Error(_)));
    assert!(!toplevel.is_finished());

    toplevel.abort();
    Ok(())
}
