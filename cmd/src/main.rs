mod commands;

use commands::{Cmd, CliArgument};
use clap::Parser;
use color_eyre::Result;
use jsonrpsee::{
    rpc_params,
    core::client::ClientT
};
use rpc::rpc_client::create_rpc_client;
use p2p_network::{
    get_node_name,
    short_peer_id,
    behaviour::UmbralPublicKeyResponse
};
use runtime::llm::AgentSecretsJson;



#[tokio::main()]
async fn main() -> Result<()> {

    color_eyre::install()?;
	let _ = tracing_subscriber::FmtSubscriber::builder()
		.with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
		.try_init();

    let cmd = Cmd::parse();

    println!("\nCreating RPC Client...");
    let port = cmd.rpc_server_address;
    let client = create_rpc_client(port.port()).await?;

    match cmd.argument {
        CliArgument::Broadcast { agent_name, shares, threshold } => {

            println!("Requesting network to proxy re-encrypt agent: {}'s secrets and broadcast fragments(n={}, t={})",
                agent_name,
                shares,
                threshold
            );
            // client tells node to create proxy re-encryption key fragments and broadcast them
            // to the network as an example
            let response: UmbralPublicKeyResponse = client.request(
                "broadcast",
                rpc_params![
                    agent_name,
                    shares,
                    threshold
                ]
            ).await?;

            println!(
                "\nNext Vessel:\n{}\n{}\nUmbral Public Key: {}",
                get_node_name(&response.umbral_peer_id.clone().into()),
                short_peer_id(&response.umbral_peer_id.into()),
                &response.umbral_public_key
            );
        }
        CliArgument::Respawn { agent_name } => {
            let response2: AgentSecretsJson = client.request(
                "request",
                rpc_params![
                    agent_name
                ]
            ).await?;
            println!("\nDecrypted Agent Secrets:\n{:?}\n", response2);
        }
    }

    Ok(())
}

