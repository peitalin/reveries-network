mod commands;

use std::collections::{HashMap, HashSet};
use commands::{Cmd, CliArgument};
use clap::Parser;
use colored::Colorize;
use color_eyre::Result;
use jsonrpsee::{
    rpc_params,
    core::client::ClientT
};
use libp2p::PeerId;
use rpc::rpc_client::create_rpc_client;
use p2p_network::{
    get_node_name,
    short_peer_id,
};
use p2p_network::types::UmbralPublicKeyResponse;
use runtime::llm::AgentSecretsJson;



#[tokio::main()]
async fn main() -> Result<()> {

    color_eyre::install()?;
	let _ = tracing_subscriber::FmtSubscriber::builder()
		.with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
		.try_init();

    let cmd = Cmd::parse();

    log(format!("Creating RPC Client..."));
    let port = cmd.rpc_server_address;
    let client = create_rpc_client(port.port()).await?;

    match cmd.argument {
        CliArgument::Broadcast { agent_name, agent_nonce, shares, threshold } => {

            log(format!("Requesting network to proxy re-encrypt agent: {}/{}'s secrets and broadcast fragments(n={}, t={})",
                agent_name,
                agent_nonce,
                shares,
                threshold
            ));
            // client tells node to create proxy re-encryption key fragments and broadcast them
            // to the network as an example
            let response: UmbralPublicKeyResponse = client.request(
                "broadcast",
                rpc_params![
                    agent_name,
                    agent_nonce,
                    shares,
                    threshold
                ]
            ).await?;

            log(format!(
                "Next Vessel: {} {}\nUmbral Public Key: {}",
                get_node_name(&response.umbral_peer_id.clone().into()).yellow(),
                short_peer_id(&response.umbral_peer_id.into()).green(),
                format!("{}", &response.umbral_public_key).red()
            ));
        }
        CliArgument::Respawn { agent_name, agent_nonce } => {
            let response2: AgentSecretsJson = client.request(
                "request",
                rpc_params![
                    agent_name,
                    agent_nonce
                ]
            ).await?;
            log(format!("Decrypted Agent Secrets:\n{:?}\n", response2));
        }
        CliArgument::GetAgentKfragPeers { agent_name, agent_nonce } => {
            let response3: HashMap<u32, HashSet<PeerId>> = client.request(
                "get_agent_kfrag_peers",
                rpc_params![
                    agent_name.clone(),
                    agent_nonce.clone()
                ]
            ).await?;

            log(format!("Peers holding Agent '{}/{}' Kfrags", agent_name, agent_nonce).red());
            for (frag_num, peers) in response3 {

                let peer_names = peers.iter()
                    .map(|peer_id| get_node_name(peer_id))
                    .collect::<Vec<String>>();

                log(format!("Fragment({}): {:?}", format!("{}", frag_num).green(), peer_names));
            }
        }
    }

    Ok(())
}


pub fn log<S: std::fmt::Display>(message: S) {
    println!("{}{} {}",
        "Cmd Client".bright_blue(),
        ">".blue(),
        message
    );
}