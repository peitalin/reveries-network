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
use p2p_network::TryPeerId;
use serde_json::Value;
use tracing::{debug, info, warn, error};

use rpc::rpc_client::{parse_url, create_rpc_client};
use p2p_network::{
    node_client::RestartReason,
    types::NodeVesselStatus,
    get_node_name,
    short_peer_id,
};
use telemetry;



#[tokio::main()]
async fn main() -> Result<()> {

    color_eyre::install()?;
    telemetry::init_logger(telemetry::LoggerConfig {
        show_log_level: true,
        show_path: true,
        ..Default::default()
    });

    let cmd = Cmd::parse();
    let port = cmd.rpc_server_address.port();

    match cmd.argument {

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

            info!("Peers subscribed to '{}-{}' kfrag broadcasts:", agent_name, agent_nonce);
            for (frag_num, peers) in peers_sorted_by_fragments {
                let peer_names = peers.iter()
                    .map(|peer_id| get_node_name(peer_id))
                    .collect::<Vec<&str>>();

                info!("Fragment({}): {:?}", format!("{}", frag_num).green(), peer_names);
            }
        }

        CliArgument::SpawnAgent {
            total_frags,
            threshold,
        } => {

            let client = create_rpc_client(port).await?;
            // Read local AgentSecretJson file and send to node over a secure channel/TLS.
            // TODO: user will send this over a secure channel and commit the hash onchain
            // along with some payment.
            // TODO: use port as seed for prototyping
            let secret_key_seed = port.to_string().chars().last()
                .unwrap_or('1')
                .to_digit(10)
                .unwrap_or(1);

            let agent_secrets_json = runtime::llm::read_agent_secrets(
                secret_key_seed as usize
            );

            let NodeVesselStatus {
                peer_id,
                umbral_public_key,
                agent_vessel_info,
                vessel_status
            } = client.request(
                "spawn_agent",
                rpc_params![
                    agent_secrets_json,
                    total_frags,
                    threshold
                ]
            ).await?;

            info!("{}\n{}",
                format!("Spawned Agent. Next Vessel: {}\n{}",
                    get_node_name(&peer_id),
                    short_peer_id(&peer_id),
                ).yellow(),
                format!(
                    "Umbral PublicKey: {}",
                    hex::encode(umbral_public_key.to_uncompressed_bytes())
                ).blue()
            );

        }

        CliArgument::TriggerNodeFailure => {
            let client = create_rpc_client(port).await?;
            match client.request::<RestartReason, ArrayParams>(
                "trigger_node_failure",
                rpc_params![]
            ).await {
                Ok(r) => {
                    info!("Scheduled node shutdown with reason: {:?} shortly...", r);
                }
                Err(e) => {
                    info!("Node failed, but with unexpected error: {:?}", e);
                }
            }
        }

        CliArgument::GetNodeStates { ports } => {

            let mut clients: Vec<(u16, Client)> = vec![];

            for port_str in ports {
                let port = port_str.parse::<u16>().unwrap();
                match create_rpc_client(port).await {
                    Ok(c) => clients.push((port, c)),
                    Err(_) => error!("Cannot connect to port {}, skipping.", port_str),
                }
            }

            let mut promises = vec![];
            for (_port, client) in clients.iter() {
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
                info!("{}", format!("\n{}", serde_json::to_string_pretty(r).unwrap()));
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
                info!("another letter!: {:?}", a);
            }
            warn!("none: {:?}", sub_params_two.next().await);
        }

        CliArgument::SubscribeHeartbeat => {

            let url = parse_url(port)?;
            let ws_client = WsClientBuilder::default().build(&url).await?;
            info!("SubscribeHeartbeat to: {}", url);

            let mut hb_subscription: Subscription<Option<serde_json::Value>> = ws_client
                .subscribe(
                    "subscribe_hb",
                    rpc_params![0],
                    "unsubscribe_hb"
                ).await?;

            while let Some(hb) = hb_subscription.next().await {
                if let Ok(Some(b)) = hb {
                    info!(
                        "\n{}\nnode_state: {}\ntee_attestation: {}\ntime: {}\n",
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

