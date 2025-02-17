mod rpc_server;
mod rpc_client;
mod commands;

use color_eyre::Result;
use clap::Parser;

use p2p_network::create_network;
use commands::Opt;
use rpc_server::run_server;
use tracing::{debug, info, warn, error};


#[tokio::main]
async fn main() -> Result<()> {

    color_eyre::install()?;
    telemetry::init_logger(telemetry::LoggerConfig {
        show_log_level: true,
        show_path: true,
        show_time: false,
        show_crate_name: false,
        ..Default::default()
    });

    let opt = Opt::parse();

    // Create p2p network and node
    let (
        mut node_client,
        network_events_receiver,
        network_event_loop,
    ) = create_network::new(opt.secret_key_seed).await?;

    // Spawn the network task to listen to incoming commands, run in the background.
    tokio::task::spawn(network_event_loop.listen_for_network_events());

    node_client
        .start_listening_to_network(opt.listen_address)
        .await?;

    // Subscribe and listen to gossip network for messages
    node_client.subscribe_topics(vec![
        "chat".to_string(),
        "topic_switch".to_string(),
    ]).await?;

    let mut nc = node_client.clone();
    tokio::spawn(async move {
        nc.listen_to_network_events(network_events_receiver).await.ok();
    });

    // Run RPC server if provided an RPC port,
    // so clients can make requests without running a node themselves.
    let mut nc2 = node_client.clone();
    match opt.rpc_port {
        None => {
            // listen for input messages for chat in main blocking thread
            nc2.listen_and_handle_stdin().await;
        }
        Some(port) => {
            // listen for input messages for chat in background thread
            tokio::spawn(async move { nc2.listen_and_handle_stdin().await });
            // await to keep the server running
            run_server(port, node_client).await?;
        }
    }

    Ok(())
}


