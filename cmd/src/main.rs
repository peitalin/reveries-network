mod commands;

use std::collections::{HashMap, HashSet};
use std::cmp::Ord;
use futures::TryFutureExt;
use itertools::Itertools;
use commands::{Cmd, CliArgument};
use clap::Parser;
use colored::Colorize;
use color_eyre::{Result, eyre};
use jsonrpsee::core::params;
use jsonrpsee::{
    rpc_params,
    core::client::ClientT
};
use libp2p::PeerId;
use p2p_network::node_client::RestartReason;
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
    log(format!("Querying RPC address: {}", cmd.rpc_server_address).green());
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
        CliArgument::GetKfragBroadcastPeers { agent_name, agent_nonce } => {
            let response3: HashMap<u32, HashSet<PeerId>> = client.request(
                "get_kfrag_broadcast_peers",
                rpc_params![
                    agent_name.clone(),
                    agent_nonce.clone()
                ]
            ).await?;

            let peers_sorted_by_fragments = response3
                .into_iter()
                .sorted_by(|a, b| Ord::cmp(&a.0, &b.0))
                .collect::<Vec<(u32, HashSet<PeerId>)>>();

            log(format!("Peers subscribed to '{}-{}' kfrag broadcasts:", agent_name, agent_nonce).green());
            for (frag_num, peers) in peers_sorted_by_fragments {

                let peer_names = peers.iter()
                    .map(|peer_id| get_node_name(peer_id))
                    .collect::<Vec<String>>();

                log(format!("Fragment({}): {:?}", format!("{}", frag_num).green(), peer_names));
            }
        }
        CliArgument::TopicSwitch {
            next_agent_name,
            next_agent_nonce,
            total_frags,
            threshold,
            prev_agent_name,
            prev_agent_nonce,
            peer_id
        } => {
            let response: usize = client.request(
                "topic_switch",
                rpc_params![
                    next_agent_name,
                    next_agent_nonce,
                    total_frags,
                    threshold,
                    prev_agent_name,
                    prev_agent_nonce,
                    peer_id
                ]
            ).await?;

            log(format!("Topic switched: {:?}", response).green());
        }

        CliArgument::TriggerNodeFailure => {
            match client.request::<RestartReason, params::ArrayParams>(
                "trigger_node_failure",
                rpc_params![]
            ).await {
                Ok(r) => {
                    log(format!("Scheduled node shutdown with reason: {:?} shortly...", r).green());
                }
                Err(e) => {
                    log(format!("Node failed, but with unexpected error: {:?}", e).yellow());
                }
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