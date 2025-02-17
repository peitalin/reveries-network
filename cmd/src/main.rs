mod commands;

use std::collections::{HashMap, HashSet};
use std::cmp::Ord;
use itertools::Itertools;
use commands::{Cmd, CliArgument};
use clap::Parser;
use colored::Colorize;
use color_eyre::Result;
use hex;
use jsonrpsee::core::params::ArrayParams;
use jsonrpsee::{
    rpc_params,
    core::client::{
        Client,
        ClientT,
        Subscription,
        SubscriptionClientT
    },
    ws_client::WsClientBuilder,
};
use libp2p::PeerId;
use serde_json::Value;

use rpc::rpc_client::{parse_url, create_rpc_client};
use p2p_network::{
    node_client::RestartReason,
    types::UmbralPublicKeyResponse,
    get_node_name,
    short_peer_id,
};



#[tokio::main()]
async fn main() -> Result<()> {

    color_eyre::install()?;
	let _ = tracing_subscriber::FmtSubscriber::builder()
		.with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
		.try_init();

    let cmd = Cmd::parse();
    let port = cmd.rpc_server_address.port();

    match cmd.argument {
        CliArgument::Broadcast { agent_name, agent_nonce, shares, threshold } => {

            let client = create_rpc_client(port).await?;
            log(format!("Broadcasting agent: {}/{}'s re-encryption fragments(n={}, t={})",
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

        CliArgument::GetKfragBroadcastPeers { agent_name, agent_nonce } => {
            let client = create_rpc_client(port).await?;
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
                    .collect::<Vec<&str>>();

                log(format!("Fragment({}): {:?}", format!("{}", frag_num).green(), peer_names));
            }
        }

        CliArgument::SpawnAgent {
            total_frags,
            threshold,
            secret_key_seed,
        } => {

            let client = create_rpc_client(port).await?;
            // Read local AgentSecretJson file and send to node over a secure channel/TLS.
            // TODO: user will send this over a secure channel and commit the hash onchain
            // along with some payment.
            let agent_secrets_json = runtime::llm::read_agent_secrets(
                secret_key_seed
            );

            let UmbralPublicKeyResponse {
                umbral_peer_id,
                umbral_public_key,
            } = client.request(
                "spawn_agent",
                rpc_params![
                    agent_secrets_json,
                    total_frags,
                    threshold
                ]
            ).await?;

            log("Spawned Agent");
            log(format!("Next Vessel: {} {}",
                short_peer_id(&umbral_peer_id.clone().into()),
                get_node_name(&umbral_peer_id.into())
            ).yellow());
            log(format!("Umbral PublicKey: {}",
                hex::encode(umbral_public_key.to_uncompressed_bytes())
            ));
        }

        CliArgument::TriggerNodeFailure => {
            let client = create_rpc_client(port).await?;
            match client.request::<RestartReason, ArrayParams>(
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

        CliArgument::GetNodeStates { ports } => {

            let mut clients: Vec<(u16, Client)> = vec![];

            for port_str in ports {
                let port = port_str.parse::<u16>().unwrap();
                match create_rpc_client(port).await {
                    Ok(c) => clients.push((port, c)),
                    Err(_) => println!("Cannot connect to port {}, skipping.", port_str),
                }
            }

            let mut promises = vec![];
            for (port, client) in clients.iter() {
                let promise = client.request::<Value, ArrayParams>(
                    "get_node_state",
                    rpc_params![]
                );
                promises.push(promise);
            }

            let results = futures::future::join_all(promises)
                .await
                .into_iter()
                .filter_map(|res| match res {
                    Ok(r) => Some(r),
                    Err(_) => None
                }).collect::<Vec<Value>>();

            for r in results.iter() {
                log(format!("\n{}", serde_json::to_string_pretty(r).unwrap()));
            }
        }

        CliArgument::Websocket => {

            let url = parse_url(port)?;
            let ws_client = WsClientBuilder::default().build(&url).await?;

            // Subscription with multiple parameters
            let mut sub_params_two: Subscription<String> = ws_client
                .subscribe(
                    "subscribe_letter_stream",
                    rpc_params![6],
                    "unsubscribe_letter_stream"
                ).await?;

            while let Some(a) = sub_params_two.next().await {
                println!("another letter!: {:?}", a);
            }
            println!("none: {:?}", sub_params_two.next().await);
        }

        CliArgument::SubscribeHeartbeat => {

            let url = parse_url(port)?;
            let ws_client = WsClientBuilder::default().build(&url).await?;
            println!("SubscribeHeartbeat to: {:?}", url.port());

            let mut hb_subscription: Subscription<Option<serde_json::Value>> = ws_client
                .subscribe(
                    "subscribe_hb",
                    rpc_params![0],
                    "unsubscribe_hb"
                ).await?;
            println!("{:?}", hb_subscription);

            while let Some(hb) = hb_subscription.next().await {
                if let Ok(Some(b)) = hb {
                    println!("\n{}\nnode_state: {}\ntee_attestation: {}\ntime: {}\n",
                        "Heartbeat:".green(),
                        format!("{:?}", b.get("node_state").unwrap()).bright_blue(),
                        format!("{:?}", b.get("tee_attestation").unwrap()).bright_black(),
                        format!("{:?}", b.get("time").unwrap()).bright_green(),
                    );
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