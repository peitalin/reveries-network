mod rpc_server;
mod rpc_client;
mod commands;

use anyhow::Result;
use clap::Parser;

use p2p_network::create_network;
use commands::Opt;
use rpc_server::run_server;


#[tokio::main]
async fn main() -> Result<()> {

	let _ = tracing_subscriber::FmtSubscriber::builder()
		.with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
		.try_init();

    let opt = Opt::parse();


    // Create p2p network and node
    let (
        mut node_client,
        network_events_receiver,
        mut network_event_loop,
    ) = create_network::new(opt.secret_key_seed).await?;


    // Subscribe and listen to gossip network for messages
    if let Some(chat_topics) = opt.topics {
        for topic in chat_topics {
            network_event_loop.subscribe_topic(&topic).await?;
        }
        network_event_loop.print_subscribed_topics();
    }

    // Encrypt PRE plaintext and store it in client
    let agent_secrets = runtime::llm::read_agent_secrets(
        opt.secret_key_seed.or(Some(0)).unwrap() as i32
    );
    let agent_secrets_bytes = &serde_json::to_vec(&agent_secrets)?;
    node_client.encrypt_secret(agent_secrets_bytes);

    // Spawn the network task to listen to incoming commands, run in the background.
    tokio::task::spawn(network_event_loop.listen_for_commands_and_events());

    node_client
        .start_listening(opt.listen_address)
        .await?;

    let mut nc = node_client.clone();
    tokio::spawn(async move {
        let _ = nc.listen_to_network_events(network_events_receiver).await;
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


