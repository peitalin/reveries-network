use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::Duration,
};
use color_eyre::Result;
use libp2p::{
    gossipsub,
    identity,
    kad,
    mdns,
    multiaddr::Multiaddr,
    noise,
    tcp,
    yamux,
    PeerId,
    StreamProtocol
};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, warn, info};
use std::sync::Arc;

pub use crate::types::UmbralPeerId;
use crate::SendError;
use crate::types::NetworkEvent;
use crate::behaviour::Behaviour;
use crate::network_events::NetworkEvents;
use crate::behaviour::heartbeat_behaviour::{
    HeartbeatBehaviour,
    HeartbeatConfig
};
use crate::node_client::{
    NodeClient,
    ContainerManager
};

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
    let (chat_cmd_sender, chat_cmd_receiver) = mpsc::channel(100);

    // Swarm Setup
    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default
        )?
        // QUIC has it's own connection timeout.
        .with_quic()
        .with_behaviour(|key| {

            // To content-address message, we can take the hash of message and use it as an ID.
            let _message_id_fn = |message: &gossipsub::Message| {
                let mut s = DefaultHasher::new();
                message.data.hash(&mut s);
                gossipsub::MessageId::from(s.finish().to_string())
            };

            // local peer discovery with mdns
            let mdns = mdns::tokio::Behaviour::new(
                mdns::Config::default(),
                peer_id
            )?;

            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(Duration::from_secs(1))
                .validation_mode(gossipsub::ValidationMode::Strict)
                // Increase mesh parameters for better connectivity
                .mesh_n(12)          // Target number of peers in mesh (default is 6)
                .mesh_n_low(8)       // Lower bound for mesh peers (default is 4)
                .mesh_n_high(16)     // Upper bound for mesh peers (default is 12)
                .gossip_lazy(6)      // Number of peers to gossip to (default is 3)
                .history_length(10)  // Length of message cache (default is 5)
                .history_gossip(3)   // Number of history messages to gossip (default is 3)
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

            // Make ourselves discoverable
            let record_key = kad::RecordKey::new(&peer_id.to_string());
            if let Err(e) = kademlia.start_providing(record_key.clone()) {
                warn!("Failed to start providing peer ID: {}", e);
            }

            // Store our addresses in the DHT after we start listening
            // We'll do this in swarm_handlers.rs when we get NewListenAddr events
            // since we don't know our addresses yet

            // Add bootstrap nodes to Kademlia
            for (peer_id_str, addr) in &bootstrap_nodes {
                info!("Adding bootstrap node: {} at {}", peer_id_str, addr);

                // For direct addresses without peer IDs, just add the address
                if peer_id_str.is_empty() {
                    kademlia.add_address(&peer_id, addr.clone());
                    info!("Added bootstrap address: {}", addr);
                } else {
                    // If we have a peer ID, parse it and add both peer and address
                    match peer_id_str.parse::<PeerId>() {
                        Ok(bootstrap_peer_id) => {
                            kademlia.add_address(&bootstrap_peer_id, addr.clone());
                            info!("Added bootstrap peer: {} at {}", bootstrap_peer_id, addr);
                        }
                        Err(e) => warn!("Failed to parse bootstrap peer ID: {}", e),
                    }
                }
            }

            // Start the bootstrap process
            if !bootstrap_nodes.is_empty() {
                match kademlia.bootstrap() {
                    Ok(_) => debug!("Started Kademlia bootstrap process"),
                    Err(e) => warn!("Failed to bootstrap Kademlia DHT: {}", e),
                }
            }

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
                mdns: mdns,
                gossipsub: gossipsub,
                request_response: libp2p::request_response::cbor::Behaviour::new(
                    [(
                        StreamProtocol::new("/file-exchange/1"),
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

    let node_client = NodeClient::new(
        peer_id,
        &node_name,
        command_sender,
        chat_cmd_sender,
        umbral_key.clone(),
        container_manager.clone(),
        heartbeat_receiver,
    );

    Ok((
        node_client,
        network_events_receiver,
        NetworkEvents::new(
            seed,
            swarm,
            peer_id,
            &node_name,
            umbral_key,
            command_receiver,
            chat_cmd_receiver,
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
