use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::Duration,
};
use color_eyre::Result;
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

use crate::SendError;
use crate::types::{VesselPeerId, NetworkEvent};
use crate::behaviour::Behaviour;
use crate::behaviour::heartbeat_behaviour::{
    HeartbeatBehaviour,
    HeartbeatConfig
};
use crate::network_events::{NetworkEvents, NodeIdentity};
use crate::node_client::{NodeClient, ContainerManager};

thread_local! {
    pub static NODE_SEED_NUM: std::cell::RefCell<usize> = std::cell::RefCell::new(1);
}

/// Creates the network components, namely:
/// - The network client to interact with the network layer from anywhere within your application.
/// - The network event stream, e.g. for incoming requests.
/// - The network task driving the network itself.
pub async fn new<'a>(
    secret_key_seed: Option<usize>,
    bootstrap_nodes: Vec<(String, Multiaddr)>,
) -> Result<(
    NodeClient<'a>,
    mpsc::Receiver<NetworkEvent>,
    NetworkEvents<'a>
)> {
    // Create a public/private key pair, either random or based on a seed.
    let (
        peer_id,
        id_keys,
        node_name,
        umbral_key
    ) = generate_peer_keys(secret_key_seed);

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

            // To content-address message, we can take the hash of message and use it as an ID.
            let _message_id_fn = |message: &gossipsub::Message| {
                let mut s = DefaultHasher::new();
                message.data.hash(&mut s);
                gossipsub::MessageId::from(s.finish().to_string())
            };

            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(Duration::from_secs(1))
                .validation_mode(gossipsub::ValidationMode::Strict)
                .duplicate_cache_time(Duration::from_secs(5))
                .build()
                .map_err(|e| SendError(e.to_string()))?;

            let gossipsub = gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(key.clone()),
                gossipsub_config
            )?;

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
            let identify = libp2p_identify::Behaviour::new(libp2p_identify::Config::new(
                "/my-node/1.0.0".to_string(),
                key.public(),
            ));

            Ok(Behaviour {
                kademlia,
                heartbeat: HeartbeatBehaviour::new(
                    // send_timeout should be larger than idle_timeout
                    HeartbeatConfig {
                        // Sending of `TeeAttestationBytes` should not take longer than this
                        // This is the delay before ContainerManager reboots node.
                        send_timeout: Duration::from_millis(16_000),
                        // Idle time before sending next `TeeAttestationBytes`
                        // This is the delay before Vessels attempt to reincarnate a unresponsive vessel
                        // In production, set this much higher
                        idle_timeout: Duration::from_millis(8_000),
                        // Max failures allowed. Requests disconnection if reached
                        max_failures: 1,
                    },
                    heartbeat_failure_sender,
                    heartbeat_sender,
                ),
                gossipsub: gossipsub,
                identify: identify,
                request_response: libp2p::request_response::cbor::Behaviour::new(
                    [(
                        StreamProtocol::new("/1up-kfrags-reqres/1.0.0"),
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
        &node_name,
        peer_id,
        id_keys,
        seed,
        umbral_key.clone(),
    );

    let node_client = NodeClient::new(
        node_identity.clone(),
        command_sender,
        umbral_key,
        container_manager.clone(),
        heartbeat_receiver,
    );

    Ok((
        node_client,
        network_events_receiver,
        NetworkEvents::new(
            swarm,
            node_identity,
            command_receiver,
            network_events_sender,
            heartbeat_failure_receiver,
            container_manager
        ),
    ))
}

pub fn generate_peer_keys<'a>(secret_key_seed: Option<usize>) -> (
    libp2p::PeerId,
    identity::Keypair,
    &'a str,
    runtime::reencrypt::UmbralKey
) {

    // Create a public/private key pair, either random or based on a seed.
    let (id_keys, umbral_key) = match secret_key_seed {
        Some(seed) => {
            let mut bytes = [0u8; 32];
            bytes[0] = seed as u8;

            // set seed for working out frag_num this this peer
            NODE_SEED_NUM.with(|n| {
                *n.borrow_mut() = seed;
            });

            let id_keys = identity::Keypair::ed25519_from_bytes(bytes).unwrap();
            let umbral_key = runtime::reencrypt::UmbralKey::new(Some(bytes.as_slice()));
            (id_keys, umbral_key)
        },
        None => {
            let id_keys = identity::Keypair::generate_ed25519();
            let umbral_key = runtime::reencrypt::UmbralKey::new(None);
            (id_keys, umbral_key)
        }
    };

    let peer_id = id_keys.public().to_peer_id();
    let node_name = crate::get_node_name(&peer_id);
    (peer_id, id_keys, node_name, umbral_key)
}
