use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::Duration,
};
use color_eyre::Result;
use color_eyre::eyre::anyhow;
use libp2p::{
    dns,
    gossipsub,
    identity,
    kad,
    multiaddr::Multiaddr,
    noise,
    tcp,
    yamux,
    PeerId,
    StreamProtocol,
};
use tokio::sync::{mpsc, RwLock};
use std::sync::Arc;
use std::path::Path;
use ed25519_dalek::{VerifyingKey as EdVerifyingKey, PUBLIC_KEY_LENGTH};
use pkcs8::{EncodePublicKey, LineEnding};
use tracing::{info, warn, error};
use libp2p_identity::Keypair as IdentityKeypair;
use libp2p_identity::PublicKey;
use std::env;
use std::fs;

use crate::SendError;
use crate::types::NetworkEvent;
use crate::behaviour::Behaviour;
use crate::behaviour::heartbeat_behaviour::{
    HeartbeatBehaviour,
    HeartbeatConfig
};
use crate::network_events::{NetworkEvents, NodeIdentity};
use crate::node_client::{NodeClient, ContainerManager};
use crate::usage_db::init_usage_db;
use crate::env_var::EnvVars;
use crate::utils::pubkeys::{generate_peer_keys, export_libp2p_public_key};

/// Creates the network components, namely:
/// - The network client to interact with the network layer from anywhere within your application.
/// - The network event stream, e.g. for incoming requests.
/// - The network task driving the network itself.
pub async fn new(
    secret_key_seed: Option<usize>,
    listen_address: Vec<Multiaddr>,
    bootstrap_nodes: Vec<(String, Multiaddr)>,
) -> Result<NodeClient> {

    // Create a public/private key pair, either random or based on a seed.
    let (
        peer_id,
        id_keys,
        node_name,
        umbral_key
    ) = generate_peer_keys(secret_key_seed);

    let env_vars = EnvVars::load();

    match export_libp2p_public_key(&id_keys, &env_vars.P2P_NODE_PUBKEY_PATH) {
        Ok(_) => info!("Successfully exported p2p-node public key to {}", env_vars.P2P_NODE_PUBKEY_PATH),
        Err(e) => {
            error!("Failed to export p2p-node public key to {}: {}.
                Signature verification by proxy will fail.", env_vars.P2P_NODE_PUBKEY_PATH, e);
            return Err(e);
        }
    }

    // TODO: used for determining which fragment the peer subscribes
    // Replace with NODE_SEED_NUM
    let seed = secret_key_seed.unwrap_or(0);

    // Channels
    let (heartbeat_failure_sender, heartbeat_failure_receiver) = tokio::sync::mpsc::channel(100);
    let (heartbeat_sender, heartbeat_receiver) = async_channel::bounded(100);
    let (command_sender, command_receiver) = mpsc::channel(100);
    let (network_events_sender, network_events_receiver) = mpsc::channel(100);

    // Swarm Setup
    let swarm = libp2p::SwarmBuilder::with_existing_identity(id_keys.clone())
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default
        )?
        // QUIC has it's own connection timeout.
        .with_quic()
        .with_dns()?
        .with_behaviour(|key| {

            // Configure Kademlia for peer discovery
            let mut kademlia = kad::Behaviour::new(
                peer_id,
                kad::store::MemoryStore::new(key.public().to_peer_id())
            );

            // Enable Kademlia record publishing
            kademlia.set_mode(Some(kad::Mode::Server));

            // Add bootstrap nodes to Kademlia and attempt DNS resolution
            for (peer_id_str, addr) in &bootstrap_nodes {
                if let Ok(bootstrap_peer_id) = peer_id_str.parse::<PeerId>() {
                    kademlia.add_address(&bootstrap_peer_id, addr.clone());
                    // Try to bootstrap immediately
                    if let Err(e) = kademlia.bootstrap() {
                        tracing::warn!("Failed to bootstrap Kademlia: {}", e);
                    }
                }
            }

            // Create identify behavior
            let identify = libp2p_identify::Behaviour::new(
                libp2p_identify::Config::new(
                "/my-node/1.0.0".to_string(),
                key.public(),
                )
            );

            Ok(Behaviour {
                kademlia,
                heartbeat: HeartbeatBehaviour::new(
                    // send_timeout should be larger than idle_timeout
                    HeartbeatConfig {
                        // Sending of `TeeAttestationBytes` should not take longer than this
                        // This is the delay before ContainerManager reboots node.
                        send_timeout: Duration::from_millis(12_000),
                        // Idle time before sending next `TeeAttestationBytes`
                        // This is the delay before Vessels attempt to reincarnate a unresponsive vessel
                        // In production, set this much higher
                        idle_timeout: Duration::from_millis(6_000),
                        // Max failures allowed. Requests disconnection if reached
                        max_failures: 1,
                    },
                    heartbeat_failure_sender,
                    heartbeat_sender,
                ),
                // gossipsub: gossipsub,
                identify: identify,
                request_response: libp2p::request_response::cbor::Behaviour::new(
                    [(
                        StreamProtocol::new("/reverie-kfrags-requests/1.0.0"),
                        libp2p::request_response::ProtocolSupport::Full,
                    )],
                    libp2p::request_response::Config::default()
                )
            })
        })?
        .with_swarm_config(|c|
            c.with_idle_connection_timeout(Duration::from_secs(u64::MAX))
        )
        .build();

    let container_manager = Arc::new(RwLock::new(
        ContainerManager::new(
            std::time::Duration::from_secs(30),
        )
    ));

    let node_identity = NodeIdentity::new(
        node_name.to_string(),
        peer_id,
        id_keys,
        seed,
        umbral_key.clone(),
    );

    // 1. First spawn listen to incoming commands and network events, run in the background.
    tokio::task::spawn(
        NetworkEvents::new(
            swarm,
            node_identity.clone(),
            command_receiver,
            network_events_sender,
            heartbeat_failure_receiver,
            container_manager
        ).init_listen_for_network_events()
    );

    // Initialize Usage Report DB Pool
    let usage_db_pool = init_usage_db()?;

    let mut node_client = NodeClient::new(
        node_identity,
        command_sender,
        umbral_key,
        heartbeat_receiver,
        None, // llm-proxy's public_key will be polled
        usage_db_pool,
    );

    // 2. Start listening for peers on the network
    node_client.start_listening_to_network(listen_address).await?;

    // 3. Spawn a thread to listen to network events
    let mut nc = node_client.clone();
    tokio::spawn(async move {
        nc.listen_to_network_events(network_events_receiver).await.ok();
    });

    // 4. Poll for llm-proxy pubkeys for SignedUsageReport verification
    let mut nc = node_client.clone();
    tokio::spawn(async move {
        nc.poll_for_llm_proxy_pubkey(&env_vars).await.ok();
    });

    Ok(node_client)
}

